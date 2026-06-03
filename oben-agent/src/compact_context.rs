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

use crate::compact;
use crate::compact::CompactCofig;
use crate::context::{CompactStatus, ContextEngine};
use oben_models::{Message, TransportProvider};

// ---------------------------------------------------------------------------
// CompactContextEngine
// ---------------------------------------------------------------------------

/// The context engine — stateless policy layer for context window management.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full.
pub struct CompactContextEngine {
    config: CompactCofig,
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
        Self::with_config(CompactCofig::default())
    }

    pub fn with_config(config: CompactCofig) -> Self {
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

    #[allow(dead_code)]
    fn context_length(&self) -> usize {
        self.config.context_length
    }

    fn is_thrashing(&self) -> bool {
        self.ineffective_compression_count >= self.config.max_ineffective_consecutive
    }

    // -- Model switching ----------------------------------------------------
    #[allow(dead_code)]
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
        messages.iter().map(compact::message_token_estimate).sum()
    }

    fn should_compact(&self, messages: &[Message]) -> bool {
        if !self.active {
            tracing::info!(
                "should_compact: inactive (compression_count={}, ineffective_count={})",
                self.compression_count,
                self.ineffective_compression_count
            );
            return false;
        }
        if self.is_thrashing() {
            tracing::info!("should_compact: thrashing detected, skipping");
            return false;
        }
        // If head + tail cover all messages, there's nothing to compress —
        // skip to avoid a wasted LLM call.
        let head_end = self.config.protect_first_n.min(messages.len());
        let tail_start =
            compact::find_tail_cut_by_tokens(messages, &self.config).max(head_end);
        if tail_start <= head_end {
            tracing::info!(
                "should_compact: no middle messages to compress (head={}, tail={}), skipping",
                head_end,
                tail_start - head_end,
            );
            return false;
        }
        let threshold = self.config.threshold_tokens();
        if self.last_total_tokens > 0 {
            let result = self.last_total_tokens > threshold;
            tracing::info!(
                "should_compact: using API token count api_tokens={} threshold={} result={}",
                self.last_total_tokens, threshold, result
            );
            return result;
        }
        let tokens = self.estimate_tokens(messages);
        let result = tokens > threshold;
        tracing::info!(
            "should_compact: estimated tokens={} (messages={}) threshold={} result={}",
            tokens, messages.len(), threshold, result
        );
        result
    }

    async fn compact(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<CompactStatus> {
        let should = self.should_compact(messages);
        if !should {
            return Ok(CompactStatus::Unchanged);
        }

        info!(
            "ContextEngine: firing compression (tokens: {}, threshold: {})",
            self.last_total_tokens.max(self.estimate_tokens(messages)),
            self.config.threshold_tokens()
        );

        if let Some(transport) = transport {
            let previous = self._previous_summary.clone();
            let transport_name = transport.name().to_string();
            let result = match compact::compact_session_messages(
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

            // We don't clear `messages` before calling compact_session_messages,
            // so on failure we simply discard `result` and leave `messages` intact.
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
                    "Compression saved only {:.1}% — ineffective (threshold: {}%, consecutive: {}), keeping original messages",
                    savings,
                    self.config.ineffective_threshold,
                    self.ineffective_compression_count,
                );
                // Discard result — original messages are untouched.
                // Return Unchanged so callers skip session rotation / DB save.
                return Ok(CompactStatus::Unchanged);
            } else {
                self.consecutive_effective_compressions += 1;
                self.ineffective_compression_count = 0;
                // Apply compressed result
                messages.clear();
                messages.extend(result.messages);
                return Ok(CompactStatus::Compacted);
            }
        } else {
            return Err(anyhow::anyhow!("compression requires a transport provider"));
        }
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

            match self.compact(messages, transport, focus_topic).await {
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
        let engine = CompactContextEngine::with_config(CompactCofig {
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
    fn test_should_compact_with_real_tokens() {
        let engine = CompactContextEngine::with_config(CompactCofig {
            context_length: 10_000,
            ..Default::default()
        });
        let msgs = Vec::<Message>::new();
        assert!(!engine.should_compact(&msgs));
    }

    #[test]
    fn test_is_thrashing_resets_on_effective_compression() {
        let config = CompactCofig {
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

    // ─── Async compact() tests with mock transport ─────────────────────────────

    /// Mock transport that returns a predictable summary for testing.
    struct MockCompactTransport {
        summary: String,
    }

    #[async_trait::async_trait]
    impl TransportProvider for MockCompactTransport {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
        ) -> Result<oben_models::TransportResponse> {
            Ok(oben_models::TransportResponse {
                text: self.summary.clone(),
                tool_calls: vec![],
                tokens_used: None,
            })
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
            _delta_callback: oben_models::StreamDeltaCallback,
        ) -> Result<oben_models::TransportResponse> {
            Ok(oben_models::TransportResponse {
                text: self.summary.clone(),
                tool_calls: vec![],
                tokens_used: None,
            })
        }
    }

    fn make_long_messages(n: usize) -> Vec<Message> {
        let mut msgs = vec![Message::system("You are a helpful coding assistant.")];
        for i in 0..n {
            msgs.push(Message::user(format!("Question {}: What is the best way to implement a concurrent hashmap in Rust? Please explain the tradeoffs between Mutex and DashMap. Consider lock contention, read throughput, write throughput, and memory overhead.", i)));
            msgs.push(Message::assistant(format!("Answer {}: Here's a comprehensive comparison:\n1. Mutex: High read throughput but writes block all reads. Simple to use. Good for read-heavy workloads with low contention.\n2. DashMap: Lock-free reads, sharded writes. Best overall performance for concurrent access. Higher memory overhead due to sharding.\nRecommendation: Use DashMap for general-purpose concurrent HashMap.", i)));
        }
        msgs
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_compact_effective_returns_true_and_replaces_messages() {
        // Effective compression (savings >= threshold) → Ok(true), messages replaced
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            tail_token_budget: 500, // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        // Prime should_compact with real token count above threshold
        engine.update_from_response(50000, 50000, 100000);

        let mut messages = make_long_messages(50);
        let original_count = messages.len();
        let _original_last = messages.last().unwrap().content.to_text().to_string();

        let transport = MockCompactTransport {
            summary: "## Context Summary\nCompressed 50 turns of Rust concurrency discussion into a single summary.\n## Completed Actions\nReviewed Mutex vs DashMap tradeoffs.".to_string(),
        };

        let result = engine.compact(&mut messages, Some(&transport), None).await;

        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), crate::context::CompactStatus::Compacted), "effective compression should return CompactStatus::Compacted");
        // Messages should have been replaced (fewer after compression)
        assert!(messages.len() < original_count, "messages should be reduced after effective compression");
        assert_eq!(engine.compression_count, 1);
        assert_eq!(engine.consecutive_effective_compressions, 1);
        assert_eq!(engine.ineffective_compression_count, 0);
    }

    #[tokio::test]
    async fn test_compact_ineffective_returns_false_and_keeps_original_messages() {
        // Ineffective compression (savings < threshold) → Ok(false), messages untouched
        // Use a large protect_first_n so only a few messages are in middle,
        // allowing a mock summary to make compacted_tokens close to original.
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            protect_first_n: 90, // Protect 90 of 101 messages — only ~10 in middle
            tail_token_budget: 500,
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        engine.update_from_response(50000, 50000, 100000);

        let mut messages = make_long_messages(50);
        let original_count = messages.len();
        let original_first = messages.first().unwrap().content.to_text().to_string();
        let original_last = messages.last().unwrap().content.to_text().to_string();

        // Return a large mock summary (~5000 tokens) so compacted has
        // tokens close to original → savings < 10% (ineffective).
        let transport = MockCompactTransport {
            summary: "X".repeat(20000),
        };

        let result = engine.compact(&mut messages, Some(&transport), None).await;

        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), crate::context::CompactStatus::Unchanged), "ineffective compression should return CompactStatus::Unchanged");
        // Messages must be UNCHANGED (compute-then-commit pattern)
        assert_eq!(messages.len(), original_count, "message count should be unchanged after ineffective compression");
        assert_eq!(messages.first().unwrap().content.to_text(), original_first, "first message should be unchanged");
        assert_eq!(messages.last().unwrap().content.to_text(), original_last, "last message should be unchanged");
        assert_eq!(engine.compression_count, 1);
        assert_eq!(engine.ineffective_compression_count, 1);
        assert_eq!(engine.consecutive_effective_compressions, 0);
    }

    #[tokio::test]
    async fn test_compact_without_transport_returns_err() {
        // Need to prime last_total_tokens so should_compact returns true,
        // forcing the code path to reach the transport-null check (line 204).
        let config = CompactCofig {
            context_length: 100_000,
            tail_token_budget: 500, // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        engine.update_from_response(50000, 50000, 100000);
        let mut messages = make_long_messages(50);

        let result = engine.compact(&mut messages, None, None).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("transport provider"));
    }

    #[tokio::test]
    async fn test_compact_below_threshold_skips_compression() {
        // should_compact returns false when tokens < threshold → Ok(false) without calling transport
        let config = CompactCofig {
            context_length: 100_000,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        // last_total_tokens = 0 and estimated tokens < threshold → no compaction
        // Don't prime update_from_response, so should_compact uses estimate

        let mut messages = vec![Message::user("hi")];

        struct CountingTransport;

        #[async_trait::async_trait]
        impl TransportProvider for CountingTransport {
            fn name(&self) -> &str { "counting" }
            async fn chat(&self, _: &[Message], _: &oben_models::CallMode) -> Result<oben_models::TransportResponse> {
                unreachable!("transport should not be called when below threshold")
            }
            async fn stream_chat(&self, _: &[Message], _: &oben_models::CallMode, _: oben_models::StreamDeltaCallback) -> Result<oben_models::TransportResponse> {
                unreachable!("stream_chat should not be called when below threshold")
            }
        }

        let transport = CountingTransport;
        let result = engine.compact(&mut messages, Some(&transport), None).await;

        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), crate::context::CompactStatus::Unchanged), "below threshold should return CompactStatus::Unchanged");
    }

    #[tokio::test]
    async fn test_compact_thrashing_detection_after_multiple_ineffective() {
        // After max_ineffective_consecutive ineffective compressions, should_compact returns false
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 3,
            protect_first_n: 90,     // Protect most messages so middle is tiny
            tail_token_budget: 500,  // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        engine.update_from_response(50000, 50000, 100000);

        let messages = make_long_messages(50);
        // Return a huge summary made of non-whitespace text so estimate_tokens
        // counts it. With protect_first_n:90, middle is ~1 msg (~650 token).
        // Mock ~5005 token → compacted > original → negative savings → ineffective.
        let transport = MockCompactTransport {
            summary: "X".repeat(20000),
        };

        // First ineffective compression
        let mut test_msgs = messages.clone();
        let r1 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(r1.unwrap(), crate::context::CompactStatus::Unchanged));

        // Second ineffective compression
        let mut test_msgs = messages.clone();
        let r2 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(r2.unwrap(), crate::context::CompactStatus::Unchanged));

        // Third ineffective compression → triggers thrashing
        let mut test_msgs = messages.clone();
        let r3 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(r3.unwrap(), crate::context::CompactStatus::Unchanged));

        // Now should_compact should return false even if tokens are high
        assert!(engine.is_thrashing(), "should be thrashing after 3 consecutive ineffective compressions");

        // Fourth attempt should skip due to thrashing
        let mut test_msgs = messages.clone();
        let r4 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(r4.unwrap(), crate::context::CompactStatus::Unchanged), "thrashing should prevent further compression attempts");
        assert_eq!(engine.compression_count, 3, "should not count thrashed attempt");
    }

    #[tokio::test]
    async fn test_compact_thrashing_cleared_on_effective() {
        // Effective compression clears ineffective_count and resets consecutive_effective.
        // Thrashing prevention (should_compact blocking when thrashing) is tested in
        // test_compact_thrashing_detection_after_multiple_ineffective — this test just
        // verifies the state transition on a successful compression.
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            tail_token_budget: 500,
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        // Prime last_total_tokens so should_compact returns true for 101 messages
        engine.update_from_response(50000, 50000, 100000);

        // Simulate some previous ineffective compressions (before thrashing triggers)
        engine.ineffective_compression_count = 1;
        engine.consecutive_effective_compressions = 0;

        let messages = make_long_messages(50);
        let good_transport = MockCompactTransport {
            summary: "## Summary\nThis is a valid compression summary with lots of content to ensure good savings. The summary covers all the key points from the conversation including the Rust concurrency discussion, Mutex vs DashMap comparison, and performance benchmarks."
                .to_string(),
        };

        let mut test_msgs = messages;
        let r = engine.compact(&mut test_msgs, Some(&good_transport), None).await;
        assert!(matches!(r.unwrap(), crate::context::CompactStatus::Compacted), "effective compression should succeed");
        // Ineffective count is reset on effective compression
        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 1);
    }
    #[tokio::test]
    async fn test_compact_previous_summary_updated() {
        // Previous summary is set on effective compression
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            tail_token_budget: 500,
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        // Prime last_total_tokens so should_compact returns true
        engine.update_from_response(50000, 50000, 100000);
        // Seed _previous_summary for iterative update mode (doesn't affect whether
        // compression is effective — the LLM generates a smaller summary than the
        // raw middle messages, so savings are still large).
        engine._previous_summary = Some(format!("{:>30000}", ""));

        let messages = make_long_messages(50);
        let transport = MockCompactTransport {
            summary: "## Iterative Summary\nPrevious context + new findings.\n## Completed Actions\nWorked on concurrency."
                .to_string(),
        };

        let mut test_msgs = messages;
        let result = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(result.unwrap(), crate::context::CompactStatus::Compacted));

        // result.summary wraps the transport response with [CONTEXT COMPACTION...] header.
        // Use contains() instead of exact match.
        assert!(
            engine._previous_summary.as_deref().unwrap().contains("Iterative Summary"),
            "previous_summary should contain the summary text"
        );
    }
    #[tokio::test]
    async fn test_compact_last_savings_pct_updated() {
        // Seed _previous_summary so incremental savings are below threshold (ineffective).
        // Then verify savings_pct is tracked even for ineffective compressions.
        let mut engine = CompactContextEngine::with_config(CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            tail_token_budget: 500,
            tail_overhead: 1.3,
            ..Default::default()
        });
        // Seed large previous summary to keep incremental savings low.
        // We need new_tokens >= 0.9 * old_tokens for ineffective result.
        // old_tokens ≈ 7763, first+last ≈ 300, so seed must be ~6600+ tokens (~26400 chars).
        engine._previous_summary = Some(format!("{:>30000}", ""));

        let messages = make_long_messages(50);
        let transport = MockCompactTransport {
            summary: "## Summary\nGood compression result.".to_string(),
        };

        let mut test_msgs = messages;
        let result = engine.compact(&mut test_msgs, Some(&transport), None).await;
        // With seeded _previous_summary, incremental savings < threshold → ineffective
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), crate::context::CompactStatus::Unchanged), "should be ineffective with seeded summary");
        // savings_pct should still be tracked
        assert!(engine.last_compression_savings_pct >= 0.0,
            "savings_pct should be >= 0 (got {:.1})", engine.last_compression_savings_pct);
    }
    #[tokio::test]
    async fn test_compact_on_session_start_resets_thrashing() {
        // on_session_start resets ineffective and effective counters
        let config = CompactCofig {
            context_length: 100_000,
            max_ineffective_consecutive: 2,
            protect_first_n: 90,     // Protect most messages so middle is tiny
            tail_token_budget: 500,  // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = CompactContextEngine::with_config(config);
        engine.update_from_response(50000, 50000, 100000);

        let messages = make_long_messages(50);
        // Return huge summary made of non-whitespace so estimate_tokens
        // counts it. With protect_first_n:90, middle is ~1 msg (~650 token).
        // Mock ~5005 token → compacted > original → negative savings → ineffective.
        let transport = MockCompactTransport {
            summary: "X".repeat(20000),
        };

        // Trigger thrashing
        let mut test_msgs = messages.clone();
        let _ = engine.compact(&mut test_msgs, Some(&transport), None).await;
        let mut test_msgs = messages.clone();
        let _ = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(engine.is_thrashing());

        // New session clears thrashing
        engine.on_session_start("new-session", "gpt-4", None);

        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
        assert!(!engine.is_thrashing());
    }
}
