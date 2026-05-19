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
    /// Number of tail messages to protect.
    pub protect_last_n: usize,
    /// Max messages buffer size (safety limit).
    pub max_messages: usize,
}

impl Default for ContextEngineConfig {
    fn default() -> Self {
        Self {
            context_length: 128_000,
            threshold_percent: 0.75,
            protect_first_n: 3,
            protect_last_n: 6,
            max_messages: 100,
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

    /// Return true if context is getting full and should be compressed.
    ///
    /// Priority: if we have real token data from the last API response,
    /// use that. Otherwise fall back to estimating from the message buffer.
    pub fn should_compress(&self, messages: &[Message]) -> bool {
        if !self.active {
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
        if !self.should_compress(messages) {
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
                protect_last_n: self.config.protect_last_n,
                ..Default::default()
            };

            let result = compression::compact_session_messages(
                transport,
                messages,
                &config,
                None,
                focus_topic,
                self.compression_count,
            )
            .await?;

            // Replace messages in-place
            messages.clear();
            messages.extend(result.messages);
            self.compression_count += 1;

            info!(
                "Compression complete: {} -> {} messages, {} tokens saved ({:.0}%)",
                result.stats.original_count,
                result.stats.compressed_count,
                result.stats.original_tokens.saturating_sub(result.stats.compressed_tokens),
                result.stats.savings_pct,
            );
        } else {
            // Fallback: lightweight text summarization (no LLM)
            let summary = compression::summarize_context_legacy(messages)?;
            let msg_count = messages.len();
            messages.clear();
            messages.push(Message::system(&summary));
            self.compression_count += 1;

            info!("ContextEngine: compressed {} messages to 1 summary (legacy)", msg_count);
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
        let mut ctx = ContextEngine::with_config(config);
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
}
