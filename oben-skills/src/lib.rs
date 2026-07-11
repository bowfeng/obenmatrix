//! Skill management — loading, parsing, and applying skill definitions.
//!
//! Maps to `skills/` directory structure and skill loading in Hermes.

pub mod backup;
pub mod bundles;
pub mod catalog;
pub mod commands;
pub mod category_manager;
pub mod enable_disable;
pub mod guard;
pub mod hub;
pub mod info;
pub mod local_installer;
pub mod lifecycle_manager;
pub mod lister;
pub mod loader;
pub mod provenance;
pub mod remover;
pub mod skill_preprocessing;
pub mod sync;
pub mod system;
pub mod updater;
pub mod usage_tracker;

pub use backup::*;
pub use bundles::*;
pub use catalog::*;
pub use commands::*;
pub use category_manager::*;
pub use enable_disable::*;
pub use guard::*;
pub use hub::*;
pub use info::*;
pub use local_installer::*;
pub use lifecycle_manager::*;
pub use lister::*;
pub use loader::*;
pub use provenance::*;
pub use remover::*;
pub use skill_preprocessing::*;
pub use sync::*;
pub use system::*;
pub use updater::*;
pub use usage_tracker::*;
