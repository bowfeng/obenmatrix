//! Skill management — loading, parsing, and applying skill definitions.
//!
//! Maps to `skills/` directory structure and skill loading in Hermes.

pub mod loader;
pub mod system;

pub use loader::*;
pub use system::*;
