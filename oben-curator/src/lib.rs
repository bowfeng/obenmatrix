//! Curator — background skill maintenance and lifecycle management.
//!
//! Mirrors `agent/curator.py` and `tools/skill_usage.py` from Hermes Agent.
//!
//! Responsibilities:
//! - Track per-skill usage metrics (use_count, last_used_at, etc.)
//! - Manage skill lifecycle: active → stale → archived
//! - Run periodic reviews to suggest consolidation/archival
//! - Maintain .curator_state for scheduler status

pub mod curator;
pub mod lifecycle;
pub mod report;
pub mod usage;

pub use curator::{Curator, CuratorConfig, CuratorState};
pub use lifecycle::{LifecycleManager, LifecycleState};
pub use report::{generate_json_report, generate_report, generate_summary};
pub use usage::{bump_use, load_usage, mark_agent_created, UsageRecord};
