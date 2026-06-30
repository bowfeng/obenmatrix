//! Skill management — loading, parsing, and applying skill definitions.
//!
//! Maps to `skills/` directory structure and skill loading in Hermes.

pub mod catalog;
pub mod loader;
pub mod skill_preprocessing;
pub mod system;

pub use catalog::*;
pub use loader::*;
pub use skill_preprocessing::*;
pub use system::*;
