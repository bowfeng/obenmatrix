//! Core agent engine: conversation loop, prompt building, context management.
//!
//! Key abstractions:
//! - **TurnExecutor** (`turn_executor.rs`) — deep module for the core turn cycle.
//!   One method (`execute_turn`) hides the full budget/check/compress/call/dispatch loop.
//! - **ConversationLoop** (`conversation.rs`) — coordinator that wires the executor
//!   and provides session lifecycle hooks, compression summary tracking, and the
//!   two public entry points (`run_turn` / `run_turn_with_streaming`).
//! - **ContextEngine** (`context.rs`) — owns the message buffer, tracks token usage,
//!   decides when to compress, and performs compaction.
//! - **Compaction** (`compression.rs`) — the full compaction algorithm.

pub mod budget;
pub mod chat_session;
pub mod compression;
pub mod context;
pub mod conversation;
pub mod prompt;
pub mod system_prompt;
pub mod turn_executor;

pub use budget::*;
pub use chat_session::*;
pub use compression::*;
pub use context::*;
pub use conversation::*;
pub use prompt::*;
pub use system_prompt::*;
pub use turn_executor::*;
