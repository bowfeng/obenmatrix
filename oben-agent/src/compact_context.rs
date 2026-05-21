/// Compact context engine — the default implementation of `ContextEngine`.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextEngine is a stateless policy layer for context window management.
/// It borrows messages from the Session (the single source of truth), tracks
/// real token usage from API responses, decides when compression should fire,
/// and mutates the message buffer in-place via `compress()`.

use anyhow::Result;
use tracing::info;

use crate::compression;
use crate::compression::CompressionConfig;
use crate::context::ContextEngine;
use oben_models::{Message, TransportProvider};

// ---------------------------------------------------------------------------
// CompactContextEngine
// ---------------------------------------------------------------------------

/// The context engine — stateless policy layer for context window management.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full.
pub struct CompactContextEngine {
    config: CompressionConfig,
    last_prompt_tokens: usize,
    last_completion_tokens: usize,
    last_total_tokens: usize,
    active: bool,
    compression_count: usize,
    last_compression_savings_pct: f64,
    ineffective_compression_count: usize,
    consecutive_effective_compressions: usize,
    _previous_summary: Option<String>,
    _last_summary_error: Option<String>,
    _last_aux_model_failure_model: Option<String>,
}

impl CompactContextEngine {
    pub fn new() -> Self {
        Self::with_config(CompressionConfig::default())
    }

    pub fn with_config(config: CompressionConfig) -> Self {
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

    fn context_length(&self) -> usize {
        self.config.context_length
    }

    fn is_thrashing(&self) -> bool {
        self.ineffective_compression_count >= self.config.max_ineffective_consecutive
    }

    // -- Model switching ----------------------------------------------------
    const MINIMUM_CONTEXT_LENGTH: usize = 4096;
}

#[async_trait::async_trait]
impl ContextEngine for CompactContextEngine {
    fn update_from_response(&mut self, prompt_tokens: usize, completion_tokens: usize, total_tokens: usize) {
        self.last_prompt_tokens = prompt_tokens;
        self.last_completion_tokens = completion_tokens;
        self.last_total_tokens = total_tokens;
    }

    fn last_total_tokens(&self) -> usize {
        self.last_total_tokens
    }

    fn estimate_tokens(&self, messages: &[Message]) -> usize {
        messages.iter().map(compression::message_token_estimate).sum()
    }

    fn should_compress(&self, messages: &[Message]) -> bool {
        if !self.active || self.is_thrashing() {
            return false;
        }
        if self.last_total_tokens > 0 {
            return self.last_total_tokens > self.config.threshold_tokens();
        }
        let tokens = self.estimate_tokens(messages);
        tokens > self.config.threshold_tokens()
    }

    async fn compress(
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
            let previous = self._previous_summary.clone();
            let transport_name = transport.name().to_string();
            let result = match compression::compact_session_messages(
                transport,
                messages,
                &self.config,
                previous.as_deref(),
                focus_topic,
                self.compression_count,
            )
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    let err_str = e.to_string();
                    self._last_summary_error = Some(err_str.clone());
                    self._last_aux_model_failure_model = Some(transport_name.clone());
                    tracing::error!(
                        "Compression failed after summary generation via {}: {}",
                        transport_name,
                        err_str
                    );
                    if self.config.max_tool_result_tokens == 0 {
                        return Err(anyhow::anyhow!("compression aborted: {}", err_str));
                    }
                    return Err(anyhow::anyhow!("compression failed (messages unchanged): {}", err_str));
                }
            };

            messages.clear();
            messages.extend(result.messages);
            self.compression_count += 1;

            if result.stats.summary_generated {
                if let Some(ref summary_text) = result.summary {
                    self._previous_summary = Some(summary_text.clone());
                }
            }

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

    fn on_session_start(&mut self, session_id: &str, model_name: &str, context_length: Option<usize>) {
        self.ineffective_compression_count = 0;
        self.consecutive_effective_compressions = 0;
        self.last_compression_savings_pct = 0.0;
        if let Some(ctx_len) = context_length {
            self.config.context_length = ctx_len;
        }
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;
        tracing::info!(
            "ContextEngine::on_session_start: session={} model={} context_length={}",
            session_id, model_name, self.config.context_length
        );
    }

    fn on_session_reset(&mut self) {
        self.compression_count = 0;
        self.last_prompt_tokens = 0;
        self.last_completion_tokens = 0;
        self.last_total_tokens = 0;
        self._previous_summary = None;
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;
        tracing::info!("ContextEngine::on_session_reset: compression_count={}", self.compression_count);
    }

    fn on_session_end(&mut self, session_id: &str) {
        tracing::info!(
            "ContextEngine::on_session_end: session={} compression_count={} last_error={:?}",
            session_id, self.compression_count, self._last_summary_error
        );
        self._last_summary_error = None;
        self._last_aux_model_failure_model = None;
    }

    async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<usize> {
        const MAX_PREFLIGHT_PASSES: usize = 3;
        let mut passes = 0usize;

        loop {
            let tokens = self.estimate_tokens(messages);
            let threshold = self.config.threshold_tokens();

            if tokens < threshold {
                break;
            }

            if passes >= MAX_PREFLIGHT_PASSES {
                tracing::warn!(
                    "Preflight: session still over budget after {} compression pass(es) (tokens={}, threshold={}); consider /new",
                    MAX_PREFLIGHT_PASSES, tokens, threshold
                );
                break;
            }

            passes += 1;
            tracing::info!("Preflight compression pass {}: tokens={}/{}", passes, tokens, threshold);

            self.ineffective_compression_count = 0;
            self.consecutive_effective_compressions = 0;

            match self.compress(messages, transport, focus_topic).await {
                Ok(_) => info!("Preflight compression pass {} completed", passes),
                Err(e) => {
                    tracing::warn!("Preflight compression pass {} failed: {}", passes, e);
                }
            }
        }

        info!("Preflight check complete: {} pass(es), session tokens now {}", passes, self.estimate_tokens(messages));
        Ok(passes)
    }

    fn reset(&mut self) {
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
        let engine = CompactContextEngine::new();
        let msgs = vec![make_message(&"a".repeat(400))];
        let tokens = engine.estimate_tokens(&msgs);
        assert!(tokens > 50);
        assert!(tokens < 200);
    }

    #[test]
    fn test_threshold_tokens() {
        let engine = CompactContextEngine::with_config(CompressionConfig {
            context_length: 100_000,
            ..Default::default()
        });
        assert_eq!(engine.config.threshold_tokens(), 75_000);
    }

    #[test]
    fn test_update_from_response() {
        let mut engine = CompactContextEngine::new();
        engine.update_from_response(1000, 500, 1500);
        assert_eq!(engine.last_total_tokens(), 1500);
    }

    #[test]
    fn test_should_compress_with_real_tokens() {
        let engine = CompactContextEngine::with_config(CompressionConfig {
            context_length: 10_000,
            ..Default::default()
        });
        let msgs = Vec::<Message>::new();
        assert!(!engine.should_compress(&msgs));
    }

    #[test]
    fn test_is_thrashing_resets_on_effective_compression() {
        let config = CompressionConfig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            ..Default::default()
        };
        let engine = CompactContextEngine::with_config(config);
        assert!(!engine.is_thrashing());
        assert!(!engine.is_thrashing());
    }

    #[test]
    fn test_reset_clears_thrashing_state() {
        let mut engine = CompactContextEngine::new();
        engine.ineffective_compression_count = 10;
        engine.consecutive_effective_compressions = 5;
        engine.reset();
        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
    }

    #[test]
    fn test_previous_summary_cleared_on_reset() {
        let mut engine = CompactContextEngine::new();
        engine._previous_summary = Some("test summary".to_string());
        engine.reset();
        assert!(engine._previous_summary.is_none());
    }

    #[test]
    fn test_on_session_start_resets_state() {
        let mut engine = CompactContextEngine::new();
        engine.compression_count = 5;
        engine.ineffective_compression_count = 3;
        engine.consecutive_effective_compressions = 2;
        engine._last_summary_error = Some("test error".to_string());
        engine._last_aux_model_failure_model = Some("old-model".to_string());

        engine.on_session_start("session-123", "gpt-4", Some(8192));

        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
        assert_eq!(engine.last_compression_savings_pct, 0.0);
        assert!(engine._last_summary_error.is_none());
        assert!(engine._last_aux_model_failure_model.is_none());
        assert_eq!(engine.config.context_length, 8192);
    }

    #[test]
    fn test_on_session_reset_clears_state() {
        let mut engine = CompactContextEngine::new();
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
        let mut engine = CompactContextEngine::new();
        engine._last_summary_error = Some("session ended".to_string());
        engine._last_aux_model_failure_model = Some("model-x".to_string());

        engine.on_session_end("session-456");

        assert!(engine._last_summary_error.is_none());
        assert!(engine._last_aux_model_failure_model.is_none());
    }
}
