//! Core agent engine: conversation loop, prompt building, context management.
//!
//! Maps to `run_agent.AIAgent` + `agent/conversation_loop.py`.

pub mod conversation;
pub mod context;
pub mod prompt;
pub mod compression;
pub mod budget;

pub use conversation::*;
pub use context::*;
pub use prompt::*;
pub use compression::*;
pub use budget::*;
