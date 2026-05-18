//! Messaging gateway — serve conversations from multiple platforms.
//!
//! Maps to `gateway/` directory in Hermes.

pub mod gateway;
pub mod platform;

pub use gateway::*;
pub use platform::*;
