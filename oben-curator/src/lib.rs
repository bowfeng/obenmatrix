//! Curator — background skill maintenance and lifecycle management.
//!
//! Mirrors `agent/curator.py` and `tools/skill_usage.py` from Hermes Agent.
//!
//! Responsibilities:
//! - Track per-skill usage metrics (use_count, last_used_at, etc.)
//! - Manage skill lifecycle: active → stale → archived
//! - Run periodic reviews to suggest consolidation/archival
//! - Maintain .curator_state for scheduler status

pub mod cron_rewrite;
pub mod curator;
pub mod lifecycle;
pub mod llm_runner;
pub mod reconciler;
pub mod report;
pub mod usage;

pub use cron_rewrite::{scan_cron_directory, update_cron_references, write_cron_rewrites, CronJob, CronRewrite};
pub use curator::{Curator, CuratorConfig, CuratorState};
pub use lifecycle::{LifecycleManager, LifecycleState};
pub use llm_runner::{extract_yaml_from_consolidation_response};
pub use reconciler::{extract_yaml_from_response, heuristic_classify, reconcile_classifications, AbsorptionEntry, ClassificationResult};
pub use report::{generate_consolidation_reports, generate_json_report, generate_report, generate_summary, ConsolidationReport};
pub use usage::{bump_use, load_usage, mark_agent_created, UsageRecord};
