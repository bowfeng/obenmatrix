//! Core agent engine: conversation loop, prompt building, context management.
//!
//! Maps to `run_agent.AIAgent` + `agent/conversation_loop.py`.
//!
//! Key abstractions:
//! - **ContextEngine** (`context.rs`) — owns the message buffer, tracks token
//!   usage from API responses, decides when to compress (`should_compress()`),
//!   and performs compaction (`compress()`).
//! - **ConversationLoop** (`conversation.rs`) — the main agent turn cycle.
//! - **Compaction** (`compression.rs`) — the full compaction algorithm
//!   (head/tail protection, tool pruning, LLM summarization). The `config_from_app`
//!   helper builds `CompressionConfig` from `AppConfig`.
//!
//! The ContextEngine owns everything — no more separate ContextManager +
//! ContextCompressor. `maybe_compress()` on ConversationLoop now calls
//! `ContextEngine::compress()` which delegates to `compact_session_messages()`.

pub mod budget;
pub mod chat_session;
pub mod compression;
pub mod context;
pub mod conversation;
pub mod prompt;
pub mod system_prompt;

pub use budget::*;
pub use chat_session::*;
pub use compression::*;
pub use context::*;
pub use conversation::*;
pub use prompt::*;
pub use system_prompt::*;
