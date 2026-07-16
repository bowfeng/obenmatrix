//! Core agent engine: conversation loop, prompt building, context management.
//!
//! Key abstractions:
//! - **Agent** (`agent.rs`) — the main agent struct (legacy, being phased out)
//! - **AgentBuilder** (`agent_builder.rs`) — builder for `Agent` with optional shared hooks.
//! - **ConversationCoordinator** (`coordinator/`) — unified conversation loop trait
//!   and implementations (CLI, TUI). Replaces `ConversationLoop` and `Agent::interactive_chat`.
//! - **TurnExecutor** (`turn_executor.rs`) — deep module for the core turn cycle.
//!   One method (`execute_turn`) hides the full budget/check/compress/call/dispatch loop.
//! - **ContextWindowManager** (`context.rs`) — trait defining the CWM interface.
//! - **BuiltinContextWindowManager** (`compact_context.rs`) — default implementation.
//! - **Compaction** (`compression.rs`) — the full compaction algorithm.
//! - **InteractionProvider** (`interaction.rs`) — communication protocol with external
//!   interfaces (CLI, TUI, Gateway).
//! - **HookEngine** (`hooks/runtime.rs`) — unified hook registry and execution hub.
//! - **TurnTerminationPolicy** + **TurnRemedyPolicy** (`coordinator/termination.rs`) —
//!   pluggable turn termination (response evaluation) and remedy (budget/empty recovery).

pub mod agent;
pub mod agent_builder;
pub mod agent_registry;
pub mod budget;
pub mod compact;
pub mod compact_context;
pub mod concurrent_dispatch;
pub mod context;
pub mod coordinator;
pub mod delegate;
pub mod error_classifier;
pub mod fallback;
pub mod hooks;
pub mod interrupt;
pub mod interaction;
pub mod message_sanitize;
pub mod memory_tools;
pub mod nudge;
pub mod prompt;
pub mod retry;
pub mod stream_processor;
pub mod system_prompt;
pub mod system_prompt_cache;
pub mod transport;
pub mod turn_executor;

pub use agent::*;
pub use agent_builder::AgentBuilder;
pub use agent_registry::AgentRegistry;
pub use budget::*;
pub use compact::*;
pub use compact_context::*;
pub use concurrent_dispatch::*;
pub use context::*;
pub use coordinator::*;
pub use error_classifier::*;
pub use fallback::*;
pub use hooks::*;
pub use interaction::*;
pub use interrupt::*;
pub use message_sanitize::*;
pub use nudge::NudgeConfig;
pub use prompt::*;
pub use retry::*;
pub use stream_processor::{StreamingContextScrubber, StreamingThinkScrubber};
pub use system_prompt::*;
pub use system_prompt_cache::*;
pub use turn_executor::*;
