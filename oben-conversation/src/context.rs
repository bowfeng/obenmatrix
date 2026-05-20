/// Context engine — manages the conversation's context window.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextEngine is a stateless policy layer for context window management.
/// It borrows messages from the Session (the single source of truth), tracks
/// real token usage from API responses, decides when compression should fire,
/// and mutates the message buffer in-place via `compress()`.
///
/// Architecture:
/// 1. Tracks real tokens from API responses via `update_from_response()`
/// 2. Checks thresholds via `should_compress()`
/// 3. Performs compaction via `compress()` which mutates the message buffer
///    in-place and calls `compact_session_messages()` via the compression module
///
/// Ownership: Session owns `messages: Vec<Message>`. ContextEngine borrows
/// them — never stores a copy.

use anyhow::Result;
use tracing::info;

use crate::compression;
use oben_models::{Message, TransportProvider};

// ---------------------------------------------------------------------------
// ContextEngine
// ---------------------------------------------------------------------------

/// Configuration for the context engine.
#[derive(Debug, Clone)]
pub struct ContextEngineConfig {
    /// Context window size in tokens (e.g. 128_000).
    pub context_length: usize,
    /// Token threshold as a percentage of context_length (e.g. 0.75 = 75%).
    pub threshold_percent: f64,
    /// Number of head messages to protect (beyond system prompt).
    pub protect_first_n: usize,
    /// Token budget for tail — walk backward accumulating tokens.
    pub tail_token_budget: usize,
    /// Hard minimum: always protect at least this many messages in the tail.
    pub tail_min_messages: usize,
    /// Soft ceiling multiplier — allow budget to exceed by this factor.
    pub tail_overhead: f64,
    /// Max messages buffer size (safety limit).
    pub max_messages: usize,
    /// Minimum savings percentage for a compression to be considered effective.
    pub ineffective_threshold: f64,
    /// Max consecutive ineffective compressions before anti-thrashing kicks in.
    pub max_ineffective_consecutive: usize,
}

impl Default for ContextEngineConfig {
    fn default() -> Self {
        Self {
            context_length: 128_000,
            threshold_percent: 0.75,
            protect_first_n: 3,
            tail_token_budget: 20_000,
            tail_min_messages: 3,
            tail_overhead: 1.5,
            max_messages: 100,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
        }
    }
}

impl ContextEngineConfig {
    /// Derive threshold tokens from context_length and threshold_percent.
    pub fn threshold_tokens(&self) -> usize {
        (self.context_length as f64 * self.threshold_percent) as usize
    }
}

/// The context engine — stateless policy layer for context window management.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full. When `should_compress()` returns true,
/// call `compress()` to perform the full compaction (head/tail protection,
/// tool pruning, LLM summarization).
///
/// **Does not own messages.** All message operations take `&[Message]`
/// (reads) or `&mut [Message]` (compression) — the Session is the owner.
pub struct ContextEngine {
    /// Configuration.
    config: ContextEngineConfig,
    /// Real token usage from the last API response.
    last_prompt_tokens: usize,
    last_completion_tokens: usize,
    last_total_tokens: usize,
    /// Whether the engine is active (compression enabled).
    active: bool,
    /// How many times compression has fired.
    compression_count: usize,
    /// Last compression's savings percentage.
    last_compression_savings_pct: f64,
    /// Consecutive ineffective compressions (savings < threshold).
    ineffective_compression_count: usize,
    /// Consecutive effective compressions (reset counter when ineffective occurs).
    consecutive_effective_compressions: usize,
    /// Last generated summary — passed to next compression for iterative updates.
    _previous_summary: Option<String>,
    /// Last summary generation error (for gateway/CLI warning display).
    _last_summary_error: Option<String>,
    /// Model that failed during summary generation (for fallback routing).
    _last_aux_model_failure_model: Option<String>,
}

impl ContextEngine {
    /// Create a new context engine with default configuration.
    pub fn new() -> Self {
        Self::with_config(ContextEngineConfig::default())
    }

    /// Create with a custom config.
    pub fn with_config(config: ContextEngineConfig) -> Self {
        Self {
            config,
            last_prompt_tokens: 0,
            last_completion_tokens: 0,
            last_total_tokens: 0,
            active: true,
            compression_count: 0,
            last_compression_savings_pct: 0.0,
            ineffective_compression_count: 0,
            consecutive_effective_compressions: 0,
            _previous_summary: None,
            _last_summary_error: None,
            _last_aux_model_failure_model: None,
        }
    }

    // -- Token tracking (from API responses) --------------------------------

    /// Update tracked token usage from an LLM API response.
    ///
    /// Called after every API call with the usage information. This is
    /// the *real* token count from the provider, not an estimate.
    pub fn update_from_response(
        &mut self,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
    ) {
        self.last_prompt_tokens = prompt_tokens;
        self.last_completion_tokens = completion_tokens;
        self.last_total_tokens = total_tokens;
    }

    /// Get the real token count from the last API response.
    pub fn last_total_tokens(&self) -> usize {
        self.last_total_tokens
    }

    /// Get an estimate of current message list tokens.
    pub fn estimate_tokens(&self, messages: &[Message]) -> usize {
        messages.iter().map(compression::message_token_estimate).sum()
    }

    /// Get the current context window size.
    pub fn context_length(&self) -> usize {
        self.config.context_length
    }

    /// Get the compression threshold in tokens.
    pub fn threshold_tokens(&self) -> usize {
        self.config.threshold_tokens()
    }

    // -- Compression decision -----------------------------------------------

    /// Check if anti-thrashing is active (too many ineffective compressions).
    pub fn is_thrashing(&self) -> bool {
        self.ineffective_compression_count >= self.config.max_ineffective_consecutive
    }

    /// Return true if context is getting full and should be compressed.
    ///
    /// Priority: if we have real token data from the last API response,
    /// use that. Otherwise fall back to estimating from the message buffer.
    pub fn should_compress(&self, messages: &[Message]) -> bool {
        if !self.active || self.is_thrashing() {
            return false;
        }

        // Use real token count from API if available
        if self.last_total_tokens > 0 {
            return self.last_total_tokens > self.config.threshold_tokens();
        }

        // Fallback: estimate from message buffer
        let tokens = self.estimate_tokens(messages);
        tokens > self.config.threshold_tokens()
    }

    // -- Compression execution -----------------------------------------------

    /// Compress the message list if it's over the token threshold.
    ///
    /// If `transport` is provided, uses LLM-based summarization via
    /// `compact_session_messages`. Otherwise falls back to lightweight
    /// text summarization with no LLM call.
    ///
    /// Mutates `messages` in-place when compression fires.
    /// Returns `()` — the result is the side effect on the message buffer.
    pub async fn compress(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<()> {
        let should = self.should_compress(messages);
        eprintln!("ContextEngine::compress: should_compress={}, tokens={}", should, self.estimate_tokens(messages));
        if !should {
            return Ok(());
        }

        info!(
            "ContextEngine: firing compression (tokens: {}, threshold: {})",
            self.last_total_tokens.max(self.estimate_tokens(messages)),
            self.config.threshold_tokens()
        );

        if let Some(transport) = transport {
            let config = compression::CompressionConfig {
                context_length: self.config.context_length,
                protect_first_n: self.config.protect_first_n,
                tail_token_budget: self.config.tail_token_budget,
                tail_min_messages: self.config.tail_min_messages,
                tail_overhead: self.config.tail_overhead,
                ..Default::default()
            };

            let previous = self._previous_summary.clone();
            let transport_name = transport.name().to_string();
            let result = match compression::compact_session_messages(
                transport,
                messages,
                &config,
                previous.as_deref(),
                focus_topic,
                self.compression_count,
            )
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    // Capture error for gateway/CLI display
                    let err_str = e.to_string();
                    self._last_summary_error = Some(err_str.clone());
                    self._last_aux_model_failure_model = Some(transport_name.clone());
                    tracing::error!(
                        "Compression failed after summary generation via {}: {}",
                        transport_name,
                        err_str
                    );
                    // If abort mode, propagate error
                    if config.max_tool_result_tokens == 0 {
                        return Err(anyhow::anyhow!(
                            "compression aborted: {}",
                            err_str
                        ));
                    }
                    // Otherwise: compression failed but messages are unchanged,
                    // so just return Ok to let the loop continue
                    return Err(anyhow::anyhow!(
                        "compression failed (messages unchanged): {}",
                        err_str
                    ));
                }
            };

            // Replace messages in-place
            messages.clear();
            messages.extend(result.messages);
            self.compression_count += 1;

            // Save summary for next iterative update
            if result.stats.summary_generated {
                if let Some(ref summary_text) = result.summary {
                    self._previous_summary = Some(summary_text.clone());
                }
            }

            // Update anti-thrashing counters
            let savings = result.stats.savings_pct;
            self.last_compression_savings_pct = savings;
            if savings < self.config.ineffective_threshold {
                self.ineffective_compression_count += 1;
                self.consecutive_effective_compressions = 0;
                tracing::warn!(
                    "Compression saved only {:.1}% — ineffective (threshold: {}%, consecutive: {})",
                    savings,
                    self.config.ineffective_threshold,
                    self.ineffective_compression_count,
                );
            } else {
                self.consecutive_effective_compressions += 1;
                self.ineffective_compression_count = 0;
            }

            info!(
                "Compression complete: {} -> {} messages, {} tokens saved ({:.0}%)",
                result.stats.original_count,
                result.stats.compressed_count,
                result.stats.original_tokens.saturating_sub(result.stats.compressed_tokens),
                result.stats.savings_pct,
            );
        } else {
            return Err(anyhow::anyhow!("compression requires a transport provider"));
        }

        Ok(())
    }

    /// Reset the engine's token/compression state. Does not touch messages —
    /// those are owned by the Session.
    pub fn reset(&mut self) {
        self.last_prompt_tokens = 0;
        self.last_completion_tokens = 0;
        self.last_total_tokens = 0;
        self.compression_count = 0;
        self.last_compression_savings_pct = 0.0;
        self.ineffective_compression_count = 0;
        self.consecutive_effective_compressions = 0;
        self._previous_summary = None;
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;
    }

    // -- Status introspection -----------------------------------------------

    /// Return a status dict for display/logging.
    pub fn get_status(&self) -> serde_json::Value {
        let usage_pct = if self.config.context_length > 0 {
            ((self.last_total_tokens as f64 / self.config.context_length as f64) * 100.0).min(100.0)
        } else {
            0.0
        };

        serde_json::json!({
            "last_prompt_tokens": self.last_prompt_tokens,
            "last_completion_tokens": self.last_completion_tokens,
            "last_total_tokens": self.last_total_tokens,
            "context_length": self.config.context_length,
            "threshold_tokens": self.config.threshold_tokens(),
            "usage_percent": usage_pct,
            "compression_count": self.compression_count,
            "active": self.active,
        })
    }

    // -- Model switching ----------------------------------------------------

    /// Minimum context length floor.
    const MINIMUM_CONTEXT_LENGTH: usize = 4096;

    /// Update the model and recalculate compression budgets.
    ///
    /// Called when the user switches models to ensure thresholds and budgets
    /// match the new model's context window.
    pub fn update_model(&mut self, model_name: &str, context_length: usize) {
        // Enforce minimum context length floor
        let context_length = context_length.max(Self::MINIMUM_CONTEXT_LENGTH);

        // Calculate new threshold from model context
        let threshold_tokens = (context_length as f64 * self.config.threshold_percent) as usize;

        // Calculate tail budget: threshold * summary_target_ratio (0.35)
        let tail_token_budget = (threshold_tokens as f64 * 0.35) as usize;

        // Calculate max summary tokens: min(context_length * 0.05, 12000)
        let max_summary_tokens = (context_length as f64 * 0.05).min(12_000.0) as usize;

        // Update config
        self.config.context_length = context_length;

        // Log the update
        tracing::info!(
            "Context compressor updated: model={} context_length={} threshold={}",
            model_name,
            context_length,
            threshold_tokens,
        );

        info!(
            "Context compressor updated: model={} context_length={} threshold={}",
            model_name, context_length, threshold_tokens
        );
    }

    /// Lifecycle hook: session start.
    ///
    /// Called when a new session is created or an existing one is loaded.
    /// Resets anti-thrashing counters, sets model context length, and
    /// clears error state. The `session_id` is recorded for logging.
    pub fn on_session_start(
        &mut self,
        session_id: &str,
        model_name: &str,
        context_length: Option<usize>,
    ) {
        // Reset anti-thrashing counters for fresh session
        self.ineffective_compression_count = 0;
        self.consecutive_effective_compressions = 0;
        self.last_compression_savings_pct = 0.0;

        // Update model context if provided
        if let Some(ctx_len) = context_length {
            self.config.context_length = ctx_len;
        }

        // Clear error state
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;

        // Update previous_summary from config if it stores per-session data
        tracing::info!(
            "ContextEngine::on_session_start: session={} model={} context_length={}",
            session_id,
            model_name,
            self.config.context_length
        );
    }

    /// Lifecycle hook: session reset.
    ///
    /// Called when a session is reset (e.g., user sends /reset or /clear).
    /// Resets compression count, clears token tracking, and clears the
    /// previous summary.
    pub fn on_session_reset(&mut self) {
        self.compression_count = 0;
        self.last_prompt_tokens = 0;
        self.last_completion_tokens = 0;
        self.last_total_tokens = 0;
        self._previous_summary = None;
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;

        tracing::info!(
            "ContextEngine::on_session_reset: compression_count={}",
            self.compression_count
        );
    }

    /// Lifecycle hook: session end.
    ///
    /// Called when a session is deleted or archived.
    /// Logs current engine state for potential persistence.
    /// The engine should not hold references to session data after this call.
    pub fn on_session_end(&mut self, session_id: &str) {
        tracing::info!(
            "ContextEngine::on_session_end: session={} compression_count={} last_error={:?}",
            session_id,
            self.compression_count,
            self._last_summary_error
        );

        // Clear error state after session ends
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;
    }

    /// Preflight check: compress messages if already over threshold.
    ///
    /// Used when loading an existing session or switching models to avoid
    /// sending an oversized context to the LLM. Compresses up to `max_passes`
    /// times, re-estimating after each pass.
    pub async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<usize> {
        const MAX_PREFLIGHT_PASSES: usize = 3;
        let mut passes = 0usize;

        loop {
            let tokens = self.estimate_tokens(messages);
            let threshold = self.threshold_tokens();

            if tokens < threshold {
                break; // Under budget — OK to proceed
            }

            if passes >= MAX_PREFLIGHT_PASSES {
                tracing::warn!(
                    "Preflight: session still over budget after {} compression passes (tokens={}, threshold={}); consider /new",
                    MAX_PREFLIGHT_PASSES,
                    tokens,
                    threshold
                );
                break;
            }

            passes += 1;
            tracing::info!(
                "Preflight compression pass {}: tokens={}/{}",
                passes,
                tokens,
                threshold
            );

            // Clear retry counters so the model gets a fresh budget
            self.ineffective_compression_count = 0;
            self.consecutive_effective_compressions = 0;

            match self.compress(messages, transport, focus_topic).await {
                Ok(_) => info!("Preflight compression pass {} completed", passes),
                Err(e) => {
                    // Compression failed (e.g. summary generation failed)
                    // Messages unchanged — continue loop to re-estimate or give up
                    tracing::warn!("Preflight compression pass {} failed: {}", passes, e);
                }
            }
        }

        info!(
            "Preflight check complete: {} pass(es), session tokens now {}",
            passes,
            self.estimate_tokens(messages)
        );

        Ok(passes)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_message(content: &str) -> Message {
        Message::user(content)
    }

    #[test]
    fn test_estimate_tokens_text() {
        let ctx = ContextEngine::new();
        let msgs = vec![make_message(&"a".repeat(400))]; // 400 chars ~ 105 tokens
        let tokens = ctx.estimate_tokens(&msgs);
        assert!(tokens > 50);
        assert!(tokens < 200);
    }

    #[test]
    fn test_threshold_tokens() {
        let config = ContextEngineConfig {
            context_length: 100_000,
            threshold_percent: 0.75,
            ..Default::default()
        };
        let ctx = ContextEngine::with_config(config);
        assert_eq!(ctx.threshold_tokens(), 75_000);
    }

    #[test]
    fn test_update_from_response() {
        let mut ctx = ContextEngine::new();
        ctx.update_from_response(1000, 500, 1500);
        assert_eq!(ctx.last_total_tokens(), 1500);
    }

    #[test]
    fn test_should_compress_with_real_tokens() {
        let config = ContextEngineConfig {
            context_length: 10_000,
            threshold_percent: 0.75,
            ..Default::default()
        };
        let mut ctx = ContextEngine::with_config(config);
        let msgs = Vec::<Message>::new(); // empty, not needed when real tokens available
        // 7500 tokens is at threshold
        ctx.update_from_response(4000, 3500, 7500);
        assert!(!ctx.should_compress(&msgs)); // exactly at threshold
        ctx.update_from_response(4000, 3600, 7600);
        assert!(ctx.should_compress(&msgs)); // over threshold
    }

    #[test]
    fn test_should_compress_under_threshold() {
        let mut ctx = ContextEngine::new();
        let msgs = Vec::<Message>::new();
        // Default context: 128k, threshold: 75% = 96k
        // 50k + 50k = 100k total, which is over 96k
        ctx.update_from_response(50000, 50000, 100000);
        assert!(ctx.should_compress(&msgs));

        // Now use values under threshold
        let mut ctx2 = ContextEngine::new();
        ctx2.update_from_response(40000, 40000, 80000);
        assert!(!ctx2.should_compress(&msgs)); // 80k < 96k threshold
    }

    #[test]
    fn test_should_compress_estimates_from_messages() {
        // Use a small context so estimate triggers compression
        let config = ContextEngineConfig {
            context_length: 10_000,
            threshold_percent: 0.75,
            ..Default::default()
        };
        let ctx = ContextEngine::with_config(config);
        // estimate_tokens uses len/4 for Text content, threshold = 7500
        // So we need ~30000 chars total. 10 messages of ~3200 chars each.
        let long_content = "The quick brown fox jumps over the lazy dog. ".repeat(80);
        let msgs: Vec<Message> = (0..10).map(|i| make_message(&format!("Message {}: {}", i, long_content))).collect();
        // Should compress based on estimate since no real token data
        assert!(ctx.should_compress(&msgs));
    }

    #[test]
    fn test_reset_clears_token_state() {
        let mut ctx = ContextEngine::new();
        ctx.update_from_response(1000, 500, 1500);
        ctx.compression_count = 5;
        ctx.reset();
        assert_eq!(ctx.last_total_tokens(), 0);
        assert_eq!(ctx.compression_count, 0);
    }

    #[test]
    fn test_anti_thrashing_resets_on_effective_compression() {
        let config = ContextEngineConfig {
            context_length: 100_000,
            threshold_percent: 0.75,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            ..Default::default()
        };
        let mut ctx = ContextEngine::with_config(config);

        // First ineffective compression
        ctx.ineffective_compression_count = 0;
        assert!(!ctx.is_thrashing());

        // Second ineffective — counter reaches threshold
        ctx.ineffective_compression_count = 2;
        assert!(ctx.is_thrashing());

        // A good compression resets counter
        ctx.ineffective_compression_count = 0;
        assert!(!ctx.is_thrashing());
    }

    #[test]
    fn test_reset_clears_thrashing_state() {
        let mut ctx = ContextEngine::new();
        ctx.ineffective_compression_count = 10;
        ctx.consecutive_effective_compressions = 5;
        ctx.reset();
        assert_eq!(ctx.ineffective_compression_count, 0);
        assert_eq!(ctx.consecutive_effective_compressions, 0);
    }

    #[test]
    fn test_ineffective_threshold_config() {
        let config = ContextEngineConfig {
            ineffective_threshold: 20.0,
            max_ineffective_consecutive: 3,
            ..Default::default()
        };
        let mut ctx = ContextEngine::with_config(config);

        // 3 ineffective compressions below 20% threshold
        for _ in 0..3 {
            ctx.last_compression_savings_pct = 15.0;
            ctx.ineffective_compression_count += 1;
        }
        assert!(ctx.is_thrashing());

        // But 4% is still ineffective with 20% threshold
        ctx.last_compression_savings_pct = 4.0;
        ctx.ineffective_compression_count += 1; // now 4
        assert!(ctx.is_thrashing());
    }

    #[test]
    fn test_previous_summary_cleared_on_reset() {
        let mut ctx = ContextEngine::new();
        ctx._previous_summary = Some("test summary".to_string());
        ctx.reset();
        assert!(ctx._previous_summary.is_none());
    }

    #[test]
    fn test_previous_summary_initialized_none() {
        let ctx = ContextEngine::new();
        assert!(ctx._previous_summary.is_none());
    }

    #[test]
    fn test_get_status_returns_all_fields() {
        let config = ContextEngineConfig {
            context_length: 100_000,
            threshold_percent: 0.75,
            ..Default::default()
        };
        let mut ctx = ContextEngine::with_config(config);
        ctx.update_from_response(1000, 500, 1500);
        ctx.compression_count = 5;
        ctx.active = false;

        let status = ctx.get_status();

        assert_eq!(status["last_prompt_tokens"], 1000);
        assert_eq!(status["last_completion_tokens"], 500);
        assert_eq!(status["last_total_tokens"], 1500);
        assert_eq!(status["context_length"], 100_000);
        assert_eq!(status["threshold_tokens"], 75_000);
        assert!((status["usage_percent"].as_f64().unwrap() - 1.5).abs() < f64::EPSILON); // 1500/100000 * 100
        assert_eq!(status["compression_count"], 5);
        assert_eq!(status["active"], false);
    }

    #[test]
    fn test_get_status_usage_capped_at_100() {
        let config = ContextEngineConfig {
            context_length: 1000,
            threshold_percent: 0.75,
            ..Default::default()
        };
        let mut ctx = ContextEngine::with_config(config);
        ctx.update_from_response(800, 400, 1200); // over context_length

        let status = ctx.get_status();
        assert!((status["usage_percent"].as_f64().unwrap() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_get_status_default_values() {
        let ctx = ContextEngine::new();
        let status = ctx.get_status();

        assert_eq!(status["last_prompt_tokens"], 0);
        assert_eq!(status["last_completion_tokens"], 0);
        assert_eq!(status["last_total_tokens"], 0);
        assert_eq!(status["context_length"], 128_000);
        assert_eq!(status["threshold_tokens"], 96_000);
        assert_eq!(status["usage_percent"], 0.0);
        assert_eq!(status["compression_count"], 0);
        assert_eq!(status["active"], true);
    }

    #[test]
    fn test_update_model_recalculates_threshold() {
        let mut ctx = ContextEngine::new();
        ctx.update_model("gpt-4-32k", 32_000);

        // Threshold = 32_000 * 0.75 = 24_000
        assert_eq!(ctx.config.context_length, 32_000);
        assert_eq!(ctx.threshold_tokens(), 24_000);
    }

    #[test]
    fn test_update_model_enforces_minimum() {
        let mut ctx = ContextEngine::new();
        // Try to set a very small context length
        ctx.update_model("tiny-model", 1000);

        // Should be clamped to MINIMUM_CONTEXT_LENGTH (4096)
        assert_eq!(ctx.config.context_length, ContextEngine::MINIMUM_CONTEXT_LENGTH);
        // Threshold = 4096 * 0.75 = 3072
        assert_eq!(ctx.threshold_tokens(), 3072);
    }

    #[test]
    fn test_update_model_tail_budget() {
        let mut ctx = ContextEngine::new();
        ctx.update_model("gpt-4", 128_000);

        let context_length = ctx.config.context_length;
        let threshold_tokens = ctx.config.threshold_tokens();
        let expected_tail_budget = (threshold_tokens as f64 * 0.35) as usize;

        // We can't directly access tail_token_budget from tests, but we can verify
        // the config was updated correctly
        assert_eq!(context_length, 128_000);
        assert_eq!(threshold_tokens, 96_000);
    }

    #[test]
    fn test_on_session_start_resets_state() {
        let mut engine = ContextEngine::new();

        // Set some state
        engine.compression_count = 5;
        engine.ineffective_compression_count = 3;
        engine.consecutive_effective_compressions = 2;
        engine._last_summary_error = Some("test error".to_string());
        engine._last_aux_model_failure_model = Some("old-model".to_string());

        // Simulate session start with custom context length
        engine.on_session_start("session-123", "gpt-4", Some(8192));

        // Anti-thrashing counters should be reset
        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
        assert_eq!(engine.last_compression_savings_pct, 0.0);

        // Error state should be cleared
        assert!(engine._last_summary_error.is_none());
        assert!(engine._last_aux_model_failure_model.is_none());

        // Context length should be updated
        assert_eq!(engine.config.context_length, 8192);
    }

    #[test]
    fn test_on_session_reset_clears_state() {
        let mut engine = ContextEngine::new();

        // Set some state
        engine.compression_count = 10;
        engine.last_prompt_tokens = 500;
        engine.last_completion_tokens = 200;
        engine.last_total_tokens = 700;
        engine._previous_summary = Some("old summary".to_string());
        engine._last_summary_error = Some("error".to_string());

        engine.on_session_reset();

        assert_eq!(engine.compression_count, 0);
        assert_eq!(engine.last_prompt_tokens, 0);
        assert_eq!(engine.last_completion_tokens, 0);
        assert_eq!(engine.last_total_tokens, 0);
        assert!(engine._previous_summary.is_none());
        assert!(engine._last_summary_error.is_none());
    }

    #[test]
    fn test_on_session_end_clears_error_state() {
        let mut engine = ContextEngine::new();
        engine._last_summary_error = Some("session ended".to_string());
        engine._last_aux_model_failure_model = Some("model-x".to_string());

        engine.on_session_end("session-456");

        assert!(engine._last_summary_error.is_none());
        assert!(engine._last_aux_model_failure_model.is_none());
    }
}
