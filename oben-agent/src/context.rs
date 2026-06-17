/// Context window manager trait — manages the conversation's context window.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextWindowManager is a stateful policy layer for context window management
/// and session lifecycle control. It borrows messages from the Session (the single
/// source of truth), tracks real token usage from API responses, decides when
/// compaction should fire, and mutates the message buffer in-place via `compact()`.
use anyhow::Result;
use chrono::{DateTime, Utc};
use oben_models::{Message, SessionManager, TransportProvider};

/// What happened when `ContextWindowManager::compact()` was called.
///
/// Distinguishes between "nothing to do" and "tried but didn't save enough".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactStatus {
    /// No compaction was needed or possible — messages unchanged.
    /// This covers: no middle messages to summarize, or savings below threshold.
    Unchanged,
    /// A meaningful compaction was applied — messages were replaced with a summary.
    Compacted,
}

/// Configuration for automatic session splitting based on inactivity.
#[derive(Debug, Clone)]
pub struct SessionSplitConfig {
    /// Seconds after last message before a split is triggered.
    pub max_session_duration_seconds: u64,
    /// Whether this feature is enabled.
    pub enabled: bool,
}

impl Default for SessionSplitConfig {
    fn default() -> Self {
        Self {
            max_session_duration_seconds: 86_400, // 1 day
            enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// ContextWindowManager trait
// ---------------------------------------------------------------------------

/// The ContextWindowManager trait — stateful policy layer for context window
/// management and session lifecycle control.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full. When `should_compact()` returns true,
/// call `compact()` to perform the full compaction (head/tail protection,
/// tool pruning, LLM summarization).
///
/// **Does not own messages.** All message operations go through the
/// `SessionManager` — the ContextWindowManager finds and mutates messages by
/// session_id, never taking ownership of the message vector.
///
/// # Session Lifecycle
///
/// The CWM owns the `active_session_key` that was previously in `SessionManager`.
/// This allows multiple concurrent sessions (e.g. Gateway / Telegram) to each
/// have their own CWM instance, all sharing one `SessionManager` backend.
///
/// Two split paths:
///
/// 1. **Time-based** — When `should_split_session()` returns true (e.g.
///    >1 day gap since last message). The caller creates a new session,
///    then calls `on_session_split()`.
/// 2. **Lineage-based** — When `compact()` triggers compression.
///    The `TurnExecutor` calls `session_manager.split_after_compression()` to
///    create the child, then calls `on_session_split()` to reset CWM state.
///
/// Both paths invoke `on_session_split()` which resets internal tracking and
/// binds the CWM to the new session id.
#[async_trait::async_trait]
pub trait ContextWindowManager: Send + Sync {
    // -- Token tracking (from API responses) --------------------------------

    /// Update tracked token usage from an LLM API response.
    fn update_from_response(
        &mut self,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
    );

    /// Get the real token count from the last API response.
    fn last_total_tokens(&self) -> usize;

    /// Get an estimate of current message list tokens.
    fn estimate_tokens(&self, messages: &[Message]) -> usize;

    // -- Compaction decision -----------------------------------------------

    /// Return true if context is getting full and should be compacted.
    fn should_compact(&self, messages: &[Message]) -> bool;

    /// Compact the message list if it's over the token threshold.
    ///
    /// Mutates `messages` in-place when compaction fires. Returns `Ok(CompactStatus::Compacted)`
    /// when a meaningful compaction was applied (savings >= threshold).
    /// Returns `Ok(CompactStatus::Unchanged)` when compression was attempted but produced no
    /// useful summary — `messages` are left unchanged so the caller can skip
    /// session rotation and other post-compact operations.
    async fn compact(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<CompactStatus>;

    /// Preflight check: compress messages if already over threshold.
    ///
    /// Mutates `messages` in-place. Returns the number of compression passes.
    async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<usize>;

    /// Reset the engine's token/compression state. Does not touch messages.
    fn reset(&mut self);

    // -- Session lifecycle -------------------------------------------------

    /// The session ID this CWM is managing.
    ///
    /// `None` when no session has been bound yet (e.g. before the first turn).
    fn session_id(&self) -> Option<String>;

    /// Set the active session for this CWM, creating it in the session manager first.
    ///
    /// Creates the session in `session_manager` and binds the CWM to it, then
    /// resets token tracking and compression counters for the new session.
    /// Called when no active session exists (first turn in a new conversation
    /// or after a time-based split).
    fn set_active_session(&mut self, session_manager: &mut dyn SessionManager, session_name: String);

    /// Check whether the time gap since the last message exceeds the session
    /// duration threshold.
    ///
    /// Returns `true` only when:
    /// - Time-based split is enabled (`session_split_config.enabled`) AND
    /// - The duration since the last `on_message_received()` call exceeds
    ///   `max_session_duration_seconds`.
    ///
    /// The caller (`TurnExecutor`) should invoke this at turn start. If
    /// `true`, it creates a new session first, then calls `on_session_split()`.
    fn should_split_session(&self, current_time: DateTime<Utc>) -> bool;

    /// Update the last-activity timestamp when a new message arrives.
    ///
    /// Called by the caller (TurnExecutor or Agent) immediately after the
    /// user's message is added to the session. This drives
    /// `should_split_session()` for the *next* turn.
    fn on_message_received(&mut self, timestamp: DateTime<Utc>);

    /// Called after lineage-based split (compression rotation).
    ///
    /// The caller has already called `session_manager.split_after_compression()`
    /// to create the child session. This resets CWM state, binds the CWM
    /// to the new session id, and syncs the SM with a fresh reset on the new session.
    ///
    /// Time-based splits also call this — after creating the new session,
    /// before processing any messages.
    fn on_session_split(&mut self, session_manager: &mut dyn SessionManager, new_session_id: String);

    /// Check for time-based split: if threshold exceeded, return the new session ID.
    ///
    /// The caller (TurnExecutor) is responsible for creating the new session
    /// and calling `on_session_split()` to sync CWM + SM.
    fn should_do_time_based_split(&mut self, session_manager: &mut dyn SessionManager) -> Option<String>;

    /// Decide if a split is needed after compaction.
    ///
    /// Returns `true` if rotation is needed (`status` is `Compacted` or `Branched`),
    /// `false` if `status` is `Unchanged` (compaction skipped or ineffective).
    ///
    /// Note: Does NOT create child session or persist. The TurnExecutor handles
    /// `split_after_compression`, `save_compacted`, and `on_session_split`.
    fn should_split_after_compaction(&self, status: CompactStatus) -> bool;
}

/// ── Blanket impl: Box<dyn ContextWindowManager> delegates to inner ────────────

#[async_trait::async_trait]
impl ContextWindowManager for Box<dyn ContextWindowManager> {
    fn update_from_response(
        &mut self,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
    ) {
        (**self).update_from_response(prompt_tokens, completion_tokens, total_tokens)
    }
    fn last_total_tokens(&self) -> usize {
        (**self).last_total_tokens()
    }
    fn estimate_tokens(&self, messages: &[Message]) -> usize {
        (**self).estimate_tokens(messages)
    }
    fn should_compact(&self, messages: &[Message]) -> bool {
        (**self).should_compact(messages)
    }
    async fn compact(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<CompactStatus> {
        (**self).compact(messages, transport, focus_topic).await
    }
    async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<usize> {
        (**self)
            .preflight_check(messages, transport, focus_topic)
            .await
    }
    fn reset(&mut self) {
        (**self).reset()
    }

    fn session_id(&self) -> Option<String> {
        (**self).session_id()
    }
    fn set_active_session(&mut self, session_manager: &mut dyn SessionManager, session_name: String) {
        (**self).set_active_session(session_manager, session_name)
    }
    fn should_split_session(&self, current_time: DateTime<Utc>) -> bool {
        (**self).should_split_session(current_time)
    }
    fn on_message_received(&mut self, timestamp: DateTime<Utc>) {
        (**self).on_message_received(timestamp)
    }
    fn on_session_split(&mut self, session_manager: &mut dyn SessionManager, new_session_id: String) {
        (**self).on_session_split(session_manager, new_session_id)
    }
    fn should_do_time_based_split(&mut self, session_manager: &mut dyn SessionManager) -> Option<String> {
        (**self).should_do_time_based_split(session_manager)
    }
    fn should_split_after_compaction(&self, status: CompactStatus) -> bool {
        (**self).should_split_after_compaction(status)
    }
}