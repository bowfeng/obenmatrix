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
pub mod compact_context;
pub mod compression;
pub mod context;
pub mod conversation;
pub mod prompt;
pub mod system_prompt;
pub mod turn_executor;

pub use agent::*;
pub use budget::*;
pub use compact_context::*;
pub use compression::*;
pub use context::*;
pub use conversation::*;
pub use prompt::*;
pub use system_prompt::*;
pub use turn_executor::*;
