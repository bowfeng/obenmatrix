//! Curator — background skill maintenance and lifecycle management.
//!
//! Mirrors `agent/curator.py` and `tools/skill_usage.py` from Hermes Agent.
//!
//! Responsibilities:
//! - Track per-skill usage metrics (use_count, last_used_at, etc.)
//! - Manage skill lifecycle: active → stale → archived
//! - Run periodic reviews to suggest consolidation/archival
//! - Maintain .curator_state for scheduler status

pub mod usage;
pub mod lifecycle;
pub mod curator;
pub mod report;

pub use usage::{UsageRecord, load_usage, bump_use, mark_agent_created};
pub use lifecycle::{LifecycleState, LifecycleManager};
pub use curator::{Curator, CuratorConfig, CuratorState};
pub use report::{generate_report, generate_summary, generate_json_report};
