//! Core agent engine: conversation loop, prompt building, context management.
//!
//! Key abstractions:
//! - **Agent** (`agent.rs`) — the main agent struct, owns session lifecycle
//!   and conversation loop, analogous to HermesAgent in hermes-agent.
//! - **TurnExecutor** (`turn_executor.rs`) — deep module for the core turn cycle.
//!   One method (`execute_turn`) hides the full budget/check/compress/call/dispatch loop.
//! - **ConversationLoop** (`conversation.rs`) — coordinator that wires the executor
//!   and provides session lifecycle hooks, compression summary tracking, and the
//!   two public entry points (`run_turn` / `run_turn_with_streaming`).
//! - **ContextEngine** (`context.rs`) — trait defining the context engine interface.
//! - **CompactContextEngine** (`compact_context.rs`) — default implementation.
//! - **Compaction** (`compression.rs`) — the full compaction algorithm.

pub mod agent;
pub mod budget;
pub mod callbacks;
pub mod compact;
pub mod compact_context;
pub mod concurrent_dispatch;
pub mod context;
pub mod conversation;
pub mod delegate;
pub mod error_classifier;
pub mod fallback;
pub mod interrupt;
pub mod message_sanitize;
pub mod nudge;
pub mod post_turn_hook;
pub mod prompt;
pub mod retry;
pub mod stream_processor;
pub mod system_prompt;
pub mod system_prompt_cache;
pub mod turn_executor;

pub use agent::*;
pub use budget::*;
pub use callbacks::*;
pub use compact::*;
pub use compact_context::*;
pub use concurrent_dispatch::*;
pub use context::*;
pub use conversation::*;
#[allow(ambiguous_glob_reexports)]
pub use error_classifier::*;
pub use fallback::*;
pub use interrupt::*;
pub use message_sanitize::*;
pub use nudge::NudgeConfig;
pub use prompt::*;
pub use retry::*;
pub use stream_processor::*;
pub use system_prompt::*;
pub use system_prompt_cache::*;
pub use turn_executor::*;
