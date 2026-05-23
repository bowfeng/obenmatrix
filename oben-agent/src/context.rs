/// Context engine trait — manages the conversation's context window.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextEngine is a stateless policy layer for context window management.
/// It borrows messages from the Session (the single source of truth), tracks
/// real token usage from API responses, decides when compaction should fire,
/// and mutates the message buffer in-place via `compact()`.

use anyhow::Result;
use oben_models::{Message, TransportProvider};

// ---------------------------------------------------------------------------
// ContextEngine trait
// ---------------------------------------------------------------------------

/// The context engine trait — stateless policy layer for context window management.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full. When `should_compact()` returns true,
/// call `compact()` to perform the full compaction (head/tail protection,
/// tool pruning, LLM summarization).
///
/// **Does not own messages.** All message operations go through the
/// `SessionStore` — the ContextEngine finds and mutates messages by
/// session_id, never taking ownership of the message vector.
#[async_trait::async_trait]
pub trait ContextEngine: Send + Sync {
    // -- Token tracking (from API responses) --------------------------------

    /// Update tracked token usage from an LLM API response.
    fn update_from_response(&mut self, prompt_tokens: usize, completion_tokens: usize, total_tokens: usize);

    /// Get the real token count from the last API response.
    fn last_total_tokens(&self) -> usize;

    /// Get an estimate of current message list tokens.
    fn estimate_tokens(&self, messages: &[Message]) -> usize;

    // -- Compaction decision -----------------------------------------------

    /// Return true if context is getting full and should be compacted.
    fn should_compact(&self, messages: &[Message]) -> bool;

    /// Compact the message list if it's over the token threshold.
    ///
    /// Mutates `messages` in-place when compaction fires.
    /// Caller is responsible for getting messages from the session store.
    async fn compact(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<()>;

    // -- Lifecycle hooks ----------------------------------------------------

    /// Lifecycle hook: session start.
    fn on_session_start(&mut self, session_id: &str, model_name: &str, context_length: Option<usize>);

    /// Lifecycle hook: session reset.
    fn on_session_reset(&mut self);

    /// Lifecycle hook: session end.
    fn on_session_end(&mut self, session_id: &str);

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
}

// ---------------------------------------------------------------------------
// Blanket impl: Box<dyn ContextEngine> delegates to inner
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl ContextEngine for Box<dyn ContextEngine> {
    fn update_from_response(&mut self, prompt_tokens: usize, completion_tokens: usize, total_tokens: usize) {
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
    async fn compact(&mut self, messages: &mut Vec<Message>, transport: Option<&dyn TransportProvider>, focus_topic: Option<&str>) -> Result<()> {
        (**self).compact(messages, transport, focus_topic).await
    }
    fn on_session_start(&mut self, session_id: &str, model_name: &str, context_length: Option<usize>) {
        (**self).on_session_start(session_id, model_name, context_length)
    }
    fn on_session_reset(&mut self) {
        (**self).on_session_reset()
    }
    fn on_session_end(&mut self, session_id: &str) {
        (**self).on_session_end(session_id)
    }
    async fn preflight_check(&mut self, messages: &mut Vec<Message>, transport: Option<&dyn TransportProvider>, focus_topic: Option<&str>) -> Result<usize> {
        (**self).preflight_check(messages, transport, focus_topic).await
    }
    fn reset(&mut self) {
        (**self).reset()
    }
}
