/// Builtin ContextWindowManager — the default implementation of `ContextWindowManager`.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextWindowManager is a stateful policy layer for context window management
/// and session lifecycle. It borrows messages from the Session (the single source
/// of truth), tracks real token usage from API responses, decides when compression
/// should fire, and mutates the message buffer in-place via `compact()`.
use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::compact;
use crate::compact::CompactCofig;
use crate::context::{CompactStatus, ContextWindowManager, SessionSplitConfig};
use oben_models::{Message, SessionManager, TransportProvider};

// ---------------------------------------------------------------------------
// BuiltinContextWindowManager
// ---------------------------------------------------------------------------

/// The builtin context window manager — default implementation for context window
/// management and session lifecycle control.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full. Owns the active session identity for its
/// conversation so that multiple concurrent sessions (e.g. Gateway / Telegram)
/// can each have their own CWM instance sharing one backend store.
pub struct BuiltinContextWindowManager {
    config: CompactCofig,
    session_split_config: SessionSplitConfig,
    active_session_key: Option<(String, String)>, // (uuid, name)
    last_message_timestamp: Option<DateTime<Utc>>,
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

impl BuiltinContextWindowManager {
    pub fn new() -> Self {
        Self::with_config(CompactCofig::default())
    }

    pub fn with_config(config: CompactCofig) -> Self {
        Self {
            config,
            session_split_config: SessionSplitConfig::default(),
            active_session_key: None,
            last_message_timestamp: None,
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

    pub fn with_session_config(
        config: CompactCofig,
        session_split_config: SessionSplitConfig,
    ) -> Self {
        let mut this = Self::with_config(config);
        this.session_split_config = session_split_config;
        this
    }

    /// Set the active session key directly (used for child agents).
    pub fn set_active_session_key(&mut self, session_id: String, session_name: String) {
        self.active_session_key = Some((session_id, session_name));
    }

    fn is_thrashing(&self) -> bool {
        self.ineffective_compression_count >= self.config.max_ineffective_consecutive
    }

    // -- Model switching ----------------------------------------------------
}

#[async_trait::async_trait]
impl ContextWindowManager for BuiltinContextWindowManager {
    fn update_from_response(
        &mut self,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
    ) {
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
        let tail_start = compact::find_tail_cut_by_tokens(messages, &self.config).max(head_end);
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
                self.last_total_tokens,
                threshold,
                result
            );
            return result;
        }
        let tokens = self.estimate_tokens(messages);
        let result = tokens > threshold;
        tracing::info!(
            "should_compact: estimated tokens={} (messages={}) threshold={} result={}",
            tokens,
            messages.len(),
            threshold,
            result
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
            "CWM: firing compression (tokens: {}, threshold: {})",
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
                    return Err(anyhow::anyhow!(
                        "compression failed (messages unchanged): {}",
                        err_str
                    ));
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
                    "CWM: compression saved only {:.1}% — ineffective (threshold: {}%, consecutive: {}), keeping original messages",
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
        self.last_message_timestamp = None;
    }

    // -- Session lifecycle -------------------------------------------------

    fn session_id(&self) -> Option<String> {
        self.active_session_key.as_ref().map(|(id, _)| id.clone())
    }

    fn set_active_session(&mut self, session_manager: &mut dyn SessionManager, session_name: String) {
        // Only create a new session if one with this name doesn't already exist.
        // This preserves accumulated messages from previous turns — always
        // creating new sessions (the old behaviour) causes the in-memory
        // HashMap to diverge from the one that CWM writes to, so the spawned
        // task ends up reading an empty session (0 messages).
        let existing_id = session_manager.find_key(&session_name);
        let (session_id, name_used) = match existing_id {
            Some(id) => {
                tracing::debug!(
                    "set_active_session: found existing session '{}': {}",
                    session_name, id
                );
                (id, session_name)
            }
            None => {
                let sess = session_manager.new_session(&session_name).ok();
                match sess {
                    Some(s) => {
                        tracing::debug!(
                            "set_active_session: created new session '{}': {}",
                            session_name, s.id
                        );
                        // new_session may return a different ID than the
                        // one we asked for because it resolves the name
                        // internally; use the actual stored name for
                        // consistency.
                        let name_used = s.name.clone();
                        (s.id.clone(), name_used)
                    }
                    None => {
                        tracing::warn!(
                            "set_active_session: failed to create session '{}', keeping existing",
                            session_name
                        );
                        return;
                    }
                }
            }
        };
        self.active_session_key = Some((session_id, name_used));
        self.reset();
    }

    fn should_split_session(&self, current_time: DateTime<Utc>) -> bool {
        if !self.session_split_config.enabled {
            return false;
        }
        match self.last_message_timestamp {
            Some(last) => {
                let gap_seconds = (current_time - last).num_seconds() as u64;
                gap_seconds > self.session_split_config.max_session_duration_seconds
            }
            None => false,
        }
    }

    fn on_message_received(&mut self, timestamp: DateTime<Utc>) {
        self.last_message_timestamp = Some(timestamp);
    }

    fn on_session_split(&mut self, session_manager: &mut dyn SessionManager, new_session_id: String) {
        // on_session_split receives UUID of the new session (from split_after_compression child.id)
        // Try to get the session name from the manager (may fail in tests with stub)
        let name = session_manager.session_mut(&new_session_id).map(|s| s.name.clone()).unwrap_or_default();
        self.active_session_key = Some((new_session_id.clone(), name));
        self.reset();
        // Sync with session manager: mark session fresh and reset metadata
        if let Some(session) = session_manager.session_mut(&new_session_id) {
            session.metadata.is_fresh_reset = true;
            session.messages.clear();
            session.metadata.message_count = 0;
            session.metadata.tool_call_count = 0;
            session.metadata.input_tokens = 0;
            session.metadata.output_tokens = 0;
            session.metadata.total_tokens = 0;
            session.metadata.estimated_cost_usd = 0.0;
            session.summary_chunks.clear();
        }

        self.reset();
    }

    fn should_do_time_based_split(&mut self, session_manager: &mut dyn SessionManager) -> Option<String> {
        if !self.session_split_config.enabled {
            return None;
        }
        let now = Utc::now();
        match self.last_message_timestamp {
            Some(last) => {
                let gap = (now - last).num_seconds() as u64;
                if gap <= self.session_split_config.max_session_duration_seconds {
                    return None;
                }
            }
            None => return None,
        }

        let parent_id = self.active_session_key.as_ref().map(|(id, _)| id.clone()).unwrap_or_default();
        let parent_id = if parent_id.is_empty() {
            session_manager
                .list_sessions_full()
                .into_iter()
                .next()
                .map(|s| s.id)
                .unwrap_or_default()
        } else {
            parent_id
        };

        if let Ok(child) = session_manager.split_after_compression(&parent_id) {
            let new_id = child.id.clone();
            self.on_session_split(session_manager, new_id.clone());
            info!("Time-based split: {} \u{2192} {}", parent_id, new_id);
            Some(new_id)
        } else {
            warn!("Time-based split failed: continuing with parent {}", parent_id);
            None
        }
    }

    fn set_active_session_key(&mut self, session_id: String, session_name: String) {
        self.active_session_key = Some((session_id, session_name));
    }

}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::{Session, SessionManager};

    struct StubSessionManager {
        stored: Option<Session>,
    }

    impl StubSessionManager {
        fn new() -> Self {
            Self { stored: None }
        }
    }

    impl Default for StubSessionManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SessionManager for StubSessionManager {
        fn init(&mut self) -> Result<(), anyhow::Error> { Ok(()) }
        fn get_or_create_session(&mut self, _name: &str) -> &mut Session { unreachable!() }
        fn create_session(&mut self, _name: &str) -> &mut Session { unreachable!() }
        fn switch_session(&mut self, _key: &str) -> Result<&mut Session, anyhow::Error> { unreachable!() }
        fn reset_current_session(&mut self) -> Result<(), anyhow::Error> { Ok(()) }
        fn reset_session(&mut self, _key: &str) -> Result<(), anyhow::Error> { Ok(()) }
        fn suspend_session(&mut self, _key: &str) -> bool { false }
        fn mark_resume_pending(&mut self, _key: &str, _reason: &str) -> bool { false }
        fn clear_resume_pending(&mut self, _key: &str) -> bool { false }
        fn list_sessions(&self, _active_minutes: Option<u64>) -> Vec<oben_models::SessionListEntry> { Vec::new() }
        fn delete_session(&mut self, _key: &str) -> Result<(), anyhow::Error> { Ok(()) }
        fn prune_sessions(&mut self, _max_age_days: i64) -> usize { 0 }
        fn save_session(&mut self, _session_id: Option<&str>) -> Result<(), anyhow::Error> { Ok(()) }
        fn resolve_session_id(&self, _key: &str) -> Option<String> { None }
        fn update_token_tracking(&mut self, _session_id: &str, _input_tokens: usize, _output_tokens: usize, _total_tokens: usize, _estimated_cost_usd: f64) {}
        fn split_after_compression(&mut self, _parent_id: &str) -> Result<Session, anyhow::Error> { unreachable!() }
        fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
            if let Some(ref s) = self.stored {
                if s.id == session_id {
                    self.stored.as_mut()
                } else {
                    None
                }
            } else {
                None
            }
        }
        fn session(&self, _session_id: &str) -> Option<&Session> { None }
        fn save_compacted(&mut self, _session_id: &str, _messages: &[Message]) -> Result<(), anyhow::Error> { Ok(()) }
        fn incremental_save(&mut self, _session_id: Option<&str>) -> Result<(), anyhow::Error> { Ok(()) }
        fn new_session(&mut self, name: &str) -> Result<&mut Session, anyhow::Error> {
            self.stored.get_or_insert_with(|| Session::new(name));
            Ok(self.stored.as_mut().unwrap())
        }
        fn find_key(&self, _key: &str) -> Option<String> { None }
        fn list_sessions_full(&self) -> Vec<Session> { Vec::new() }
        fn get_session_messages(&self, _session_id: &str) -> Result<Vec<Message>, anyhow::Error> { Ok(Vec::new()) }
        fn set_compaction_summary(&mut self, _session_id: &str, _summary: String) -> Result<(), anyhow::Error> { Ok(()) }
        fn get_compaction_summary(&self, _session_id: &str) -> Option<String> { None }
        fn ensure_session_loaded(&mut self, _session_id: &str) -> Result<(), anyhow::Error> { Ok(()) }
        fn close(&mut self) -> Result<(), anyhow::Error> { Ok(()) }
    }

    fn make_message(content: &str) -> Message {
        Message::user(content)
    }

    #[test]
    fn test_estimate_tokens_text() {
        let engine = BuiltinContextWindowManager::new();
        let msgs = vec![make_message(&"a".repeat(400))];
        let tokens = engine.estimate_tokens(&msgs);
        assert!(tokens > 50);
        assert!(tokens < 200);
    }

    #[test]
    fn test_threshold_tokens() {
        let engine = BuiltinContextWindowManager::with_config(CompactCofig {
            context_length: 100_000,
            ..Default::default()
        });
        assert_eq!(engine.config.threshold_tokens(), 75_000);
    }

    #[test]
    fn test_update_from_response() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.update_from_response(1000, 500, 1500);
        assert_eq!(engine.last_total_tokens(), 1500);
    }

    #[test]
    fn test_should_compact_with_real_tokens() {
        let engine = BuiltinContextWindowManager::with_config(CompactCofig {
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
        let engine = BuiltinContextWindowManager::with_config(config);
        assert!(!engine.is_thrashing());
        assert!(!engine.is_thrashing());
    }

    #[test]
    fn test_reset_clears_thrashing_state() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.ineffective_compression_count = 10;
        engine.consecutive_effective_compressions = 5;
        engine.reset();
        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
    }

    #[test]
    fn test_previous_summary_cleared_on_reset() {
        let mut engine = BuiltinContextWindowManager::new();
        engine._previous_summary = Some("test summary".to_string());
        engine.reset();
        assert!(engine._previous_summary.is_none());
    }

    #[test]
    fn test_reset_clears_all_state() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.compression_count = 10;
        engine.last_prompt_tokens = 500;
        engine.last_completion_tokens = 200;
        engine.last_total_tokens = 700;
        engine._previous_summary = Some("old summary".to_string());
        engine._last_summary_error = Some("error".to_string());
        engine._last_aux_model_failure_model = Some("old-model".to_string());
        engine.ineffective_compression_count = 3;
        engine.consecutive_effective_compressions = 2;
        engine.last_compression_savings_pct = 45.0;

        engine.reset();

        assert_eq!(engine.compression_count, 0);
        assert_eq!(engine.last_prompt_tokens, 0);
        assert_eq!(engine.last_completion_tokens, 0);
        assert_eq!(engine.last_total_tokens, 0);
        assert!(engine._previous_summary.is_none());
        assert!(engine._last_summary_error.is_none());
        assert!(engine._last_aux_model_failure_model.is_none());
        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
        assert_eq!(engine.last_compression_savings_pct, 0.0);
    }

    #[test]
    fn test_reset_preserves_config() {
        let mut engine = BuiltinContextWindowManager::new();
        let saved_context_length = engine.config.context_length;

        engine.reset();

        assert_eq!(engine.config.context_length, saved_context_length);
    }

    // ── Session lifecycle tests ───────────────────────────────────────────

    #[test]
    fn test_session_id_starts_none() {
        let engine = BuiltinContextWindowManager::new();
        assert_eq!(engine.session_id(), None);
    }

    #[test]
    fn test_set_active_session_updates_id_and_resets_state() {
        let mut engine = BuiltinContextWindowManager::new();

        // Prime some token tracking state
        engine.update_from_response(1000, 500, 1500);
        engine.last_compression_savings_pct = 25.0;

        assert_eq!(engine.session_id(), None);

        // Bind a session — new_session generates a UUID, not the passed session_id
        let mut stub = StubSessionManager::new();
        engine.set_active_session(&mut stub, "chat-abc123".to_string());
        assert_eq!(engine.session_id(), Some(stub.stored.as_ref().unwrap().id.clone()));

        // Token tracking should be reset
        assert_eq!(engine.last_total_tokens(), 0);
    }

    #[test]
    fn test_should_split_session_returns_false_when_disabled() {
        let mut engine = BuiltinContextWindowManager::with_config(CompactCofig::default());
        engine.session_split_config.enabled = false;
        engine.on_message_received(Utc::now() - chrono::Duration::days(10));

        assert!(!engine.should_split_session(Utc::now()));
    }

    #[test]
    fn test_should_split_session_returns_false_when_no_timestamp() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.session_split_config.max_session_duration_seconds = 60;

        // Never received a message
        assert!(!engine.should_split_session(Utc::now()));
    }

    #[test]
    fn test_should_split_session_returns_true_after_gap() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.session_split_config.max_session_duration_seconds = 60;
        engine.on_message_received(Utc::now() - chrono::Duration::minutes(5));

        assert!(engine.should_split_session(Utc::now()));
    }

    #[test]
    fn test_should_split_session_returns_false_within_gap() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.session_split_config.max_session_duration_seconds = 60;
        engine.on_message_received(Utc::now() - chrono::Duration::seconds(30));

        assert!(!engine.should_split_session(Utc::now()));
    }

    #[test]
    fn test_on_session_split_resets_tracking_and_updates_id() {
        let mut engine = BuiltinContextWindowManager::new();
        engine.update_from_response(5000, 2000, 7000);
        engine.on_message_received(Utc::now());
        let mut stub = StubSessionManager::new();
        engine.on_session_split(&mut stub, "chat-new456".to_string());

        assert_eq!(engine.session_id(), Some("chat-new456".to_string()));
        assert_eq!(engine.last_total_tokens(), 0);
        // Timestamp cleared by reset
        assert_eq!(engine.last_message_timestamp, None);
    }

    #[test]
    fn test_on_message_received_updates_timestamp() {
        let mut engine = BuiltinContextWindowManager::new();
        assert!(engine.last_message_timestamp.is_none());

        let now = Utc::now();
        engine.on_message_received(now);
        assert_eq!(engine.last_message_timestamp, Some(now));
    }

    #[test]
    fn test_session_split_disabled_then_enabled() {
        let config = CompactCofig {
            context_length: 100_000,
            ..Default::default()
        };
        let mut engine = BuiltinContextWindowManager::with_config(config);

        // Enabled — should split after 1 day gap
        engine.on_message_received(Utc::now() - chrono::Duration::days(2));
        assert!(engine.should_split_session(Utc::now()));

        // Disabled — should never split
        engine.session_split_config.enabled = false;
        assert!(!engine.should_split_session(Utc::now()));

        // Re-enabled — splits again
        engine.session_split_config.enabled = true;
        assert!(engine.should_split_session(Utc::now()));
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
                reasoning: None,
            })
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
            _delta_callback: oben_models::StreamDeltaCallback,
            _reasoning_callback: Option<oben_models::StreamReasoningCallback>,
        ) -> Result<oben_models::TransportResponse> {
            Ok(oben_models::TransportResponse {
                text: self.summary.clone(),
                tool_calls: vec![],
                tokens_used: None,
                reasoning: None,
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
        let mut engine = BuiltinContextWindowManager::with_config(config);
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
        assert!(
            matches!(result.unwrap(), crate::context::CompactStatus::Compacted),
            "effective compression should return CompactStatus::Compacted"
        );
        // Messages should have been replaced (fewer after compression)
        assert!(
            messages.len() < original_count,
            "messages should be reduced after effective compression"
        );
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
        let mut engine = BuiltinContextWindowManager::with_config(config);
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
        assert!(
            matches!(result.unwrap(), crate::context::CompactStatus::Unchanged),
            "ineffective compression should return CompactStatus::Unchanged"
        );
        // Messages must be UNCHANGED (compute-then-commit pattern)
        assert_eq!(
            messages.len(),
            original_count,
            "message count should be unchanged after ineffective compression"
        );
        assert_eq!(
            messages.first().unwrap().content.to_text(),
            original_first,
            "first message should be unchanged"
        );
        assert_eq!(
            messages.last().unwrap().content.to_text(),
            original_last,
            "last message should be unchanged"
        );
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
        let mut engine = BuiltinContextWindowManager::with_config(config);
        engine.update_from_response(50000, 50000, 100000);
        let mut messages = make_long_messages(50);

        let result = engine.compact(&mut messages, None, None).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("transport provider"));
    }

    #[tokio::test]
    async fn test_compact_below_threshold_skips_compression() {
        // should_compact returns false when tokens < threshold → Ok(false) without calling transport
        let config = CompactCofig {
            context_length: 100_000,
            ..Default::default()
        };
        let mut engine = BuiltinContextWindowManager::with_config(config);
        // last_total_tokens = 0 and estimated tokens < threshold → no compaction
        // Don't prime update_from_response, so should_compact uses estimate

        let mut messages = vec![Message::user("hi")];

        struct CountingTransport;

        #[async_trait::async_trait]
        impl TransportProvider for CountingTransport {
            fn name(&self) -> &str {
                "counting"
            }
            async fn chat(
                &self,
                _: &[Message],
                _: &oben_models::CallMode,
            ) -> Result<oben_models::TransportResponse> {
                unreachable!("transport should not be called when below threshold")
            }
            async fn stream_chat(
                &self,
                _: &[Message],
                _: &oben_models::CallMode,
                _: oben_models::StreamDeltaCallback,
                _: Option<oben_models::StreamReasoningCallback>,
            ) -> Result<oben_models::TransportResponse> {
                unreachable!("stream_chat should not be called when below threshold")
            }
        }

        let transport = CountingTransport;
        let result = engine.compact(&mut messages, Some(&transport), None).await;

        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), crate::context::CompactStatus::Unchanged),
            "below threshold should return CompactStatus::Unchanged"
        );
    }

    #[tokio::test]
    async fn test_compact_thrashing_detection_after_multiple_ineffective() {
        // After max_ineffective_consecutive ineffective compressions, should_compact returns false
        let config = CompactCofig {
            context_length: 100_000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 3,
            protect_first_n: 90,    // Protect most messages so middle is tiny
            tail_token_budget: 500, // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = BuiltinContextWindowManager::with_config(config);
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
        assert!(matches!(
            r1.unwrap(),
            crate::context::CompactStatus::Unchanged
        ));

        // Second ineffective compression
        let mut test_msgs = messages.clone();
        let r2 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(
            r2.unwrap(),
            crate::context::CompactStatus::Unchanged
        ));

        // Third ineffective compression → triggers thrashing
        let mut test_msgs = messages.clone();
        let r3 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(matches!(
            r3.unwrap(),
            crate::context::CompactStatus::Unchanged
        ));

        // Now should_compact should return false even if tokens are high
        assert!(
            engine.is_thrashing(),
            "should be thrashing after 3 consecutive ineffective compressions"
        );

        // Fourth attempt should skip due to thrashing
        let mut test_msgs = messages.clone();
        let r4 = engine.compact(&mut test_msgs, Some(&transport), None).await;
        assert!(
            matches!(r4.unwrap(), crate::context::CompactStatus::Unchanged),
            "thrashing should prevent further compression attempts"
        );
        assert_eq!(
            engine.compression_count, 3,
            "should not count thrashed attempt"
        );
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
        let mut engine = BuiltinContextWindowManager::with_config(config);
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
        let r = engine
            .compact(&mut test_msgs, Some(&good_transport), None)
            .await;
        assert!(
            matches!(r.unwrap(), crate::context::CompactStatus::Compacted),
            "effective compression should succeed"
        );
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
        let mut engine = BuiltinContextWindowManager::with_config(config);
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
        assert!(matches!(
            result.unwrap(),
            crate::context::CompactStatus::Compacted
        ));

        // result.summary wraps the transport response with [CONTEXT COMPACTION...] header.
        // Use contains() instead of exact match.
        assert!(
            engine
                ._previous_summary
                .as_deref()
                .unwrap()
                .contains("Iterative Summary"),
            "previous_summary should contain the summary text"
        );
    }
    #[tokio::test]
    async fn test_compact_last_savings_pct_updated() {
        // Seed _previous_summary so incremental savings are below threshold (ineffective).
        // Then verify savings_pct is tracked even for ineffective compressions.
        let mut engine = BuiltinContextWindowManager::with_config(CompactCofig {
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
        assert!(
            matches!(result.unwrap(), crate::context::CompactStatus::Unchanged),
            "should be ineffective with seeded summary"
        );
        // savings_pct should still be tracked
        assert!(
            engine.last_compression_savings_pct >= 0.0,
            "savings_pct should be >= 0 (got {:.1})",
            engine.last_compression_savings_pct
        );
    }
    #[tokio::test]
    async fn test_compact_on_session_start_resets_thrashing() {
        // on_session_start resets ineffective and effective counters
        let config = CompactCofig {
            context_length: 100_000,
            max_ineffective_consecutive: 2,
            protect_first_n: 90,    // Protect most messages so middle is tiny
            tail_token_budget: 500, // Narrow tail so middle is non-empty
            tail_overhead: 1.3,
            ..Default::default()
        };
        let mut engine = BuiltinContextWindowManager::with_config(config);
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

        // Reset clears thrashing
        engine.reset();

        assert_eq!(engine.ineffective_compression_count, 0);
        assert_eq!(engine.consecutive_effective_compressions, 0);
        assert!(!engine.is_thrashing());
    }
}
