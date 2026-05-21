/// Context engine trait — manages the conversation's context window.
///
/// Maps to Hermes' `agent/context_engine.py::ContextEngine`.
///
/// The ContextEngine is a stateless policy layer for context window management.
/// It borrows messages from the Session (the single source of truth), tracks
/// real token usage from API responses, decides when compression should fire,
/// and mutates the message buffer in-place via `compress()`.

use anyhow::Result;
use oben_models::{Message, TransportProvider};

use crate::compression::CompressionConfig;

// ---------------------------------------------------------------------------
// ContextEngine trait
// ---------------------------------------------------------------------------

/// The context engine trait — stateless policy layer for context window management.
///
/// Tracks real token usage from API responses and decides when the
/// context window is getting full. When `should_compress()` returns true,
/// call `compress()` to perform the full compaction (head/tail protection,
/// tool pruning, LLM summarization).
///
/// **Does not own messages.** All message operations take `&[Message]`
/// (reads) or `&mut [Message]` (compression) — the Session is the owner.
#[async_trait::async_trait]
pub trait ContextEngine: Send + Sync {
    // -- Token tracking (from API responses) --------------------------------

    /// Update tracked token usage from an LLM API response.
    fn update_from_response(&mut self, prompt_tokens: usize, completion_tokens: usize, total_tokens: usize);

    /// Get the real token count from the last API response.
    fn last_total_tokens(&self) -> usize;

    /// Get an estimate of current message list tokens.
    fn estimate_tokens(&self, messages: &[Message]) -> usize;

    // -- Compression decision -----------------------------------------------

    /// Return true if context is getting full and should be compressed.
    fn should_compress(&self, messages: &[Message]) -> bool;

    /// Compress the message list if it's over the token threshold.
    ///
    /// Mutates `messages` in-place when compression fires.
    async fn compress(
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
    async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
        transport: Option<&dyn TransportProvider>,
        focus_topic: Option<&str>,
    ) -> Result<usize>;

    /// Reset the engine's token/compression state. Does not touch messages.
    fn reset(&mut self);
}
