/// Skill lifecycle management — active → stale → archived states.

use crate::usage::UsageRecord;
use crate::usage::{load_usage, save_usage};
use tracing::{debug, info};

/// Current lifecycle state of a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleState {
    /// Skill is actively used and maintained.
    Active,
    /// Skill has not been used in a long time.
    Stale,
    /// Skill has been archived (not deleted — recoverable).
    Archived,
    /// Skill is pinned and bypasses auto-transitions.
    Pinned,
}

impl LifecycleState {
    pub fn as_str(&self) -> &str {
        match self {
            LifecycleState::Active => "active",
            LifecycleState::Stale => "stale",
            LifecycleState::Archived => "archived",
            LifecycleState::Pinned => "pinned",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "active" => Some(LifecycleState::Active),
            "stale" => Some(LifecycleState::Stale),
            "archived" => Some(LifecycleState::Archived),
            "pinned" => Some(LifecycleState::Pinned),
            _ => None,
        }
    }
}

/// Configuration for lifecycle transitions.
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Days without use before marking as stale.
    pub stale_after_days: usize,
    /// Days without use before archiving.
    pub archive_after_days: usize,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            stale_after_days: 30,
            archive_after_days: 90,
        }
    }
}

/// Manager for skill lifecycle states.
pub struct LifecycleManager {
    config: LifecycleConfig,
}

impl LifecycleManager {
    pub fn new(config: LifecycleConfig) -> Self {
        Self { config }
    }

    /// Get the current state of a skill.
    pub fn get_state(&self, skill_name: &str) -> LifecycleState {
        let records = load_usage();
        let record = match records.get(skill_name) {
            Some(r) => r,
            None => return LifecycleState::Active,
        };

        if record.pinned {
            return LifecycleState::Pinned;
        }

        // Check if archived
        if record.archived_at.is_some() {
            return LifecycleState::Archived;
        }

        // Calculate inactivity
        let inactivity_days = self.inactivity_days(record);

        if inactivity_days > self.config.archive_after_days {
            LifecycleState::Archived
        } else if inactivity_days > self.config.stale_after_days {
            LifecycleState::Stale
        } else {
            LifecycleState::Active
        }
    }

    /// Set the state of a skill.
    pub fn set_state(&mut self, skill_name: &str, state: LifecycleState) {
        let mut records = load_usage();
        let record = records.entry(skill_name.to_string()).or_default();

        match state {
            LifecycleState::Active => {
                record.state = Some("active".to_string());
                record.archived_at = None;
            }
            LifecycleState::Stale => {
                record.state = Some("stale".to_string());
            }
            LifecycleState::Archived => {
                record.state = Some("archived".to_string());
                record.archived_at = Some(chrono::Utc::now().to_rfc3339());
            }
            LifecycleState::Pinned => {
                record.pinned = true;
            }
        }

        save_usage(&records);
        debug!("Set skill '{}' to state '{}'", skill_name, state.as_str());
    }

    /// Check if a skill is pinned.
    pub fn is_pinned(&self, skill_name: &str) -> bool {
        let records = load_usage();
        records.get(skill_name).map(|r| r.pinned).unwrap_or(false)
    }

    /// Pin or unpin a skill.
    pub fn set_pinned(&mut self, skill_name: &str, pinned: bool) {
        let mut records = load_usage();
        let record = records.entry(skill_name.to_string()).or_default();
        record.pinned = pinned;
        save_usage(&records);
        debug!("Skill '{}' pinned: {}", skill_name, pinned);
    }

    /// Run a lifecycle check on all agent-created skills.
    pub fn check_all_skills(&mut self) -> Vec<(String, LifecycleState)> {
        let agent_created = crate::usage::list_agent_created_names();
        let mut changes = Vec::new();

        for skill_name in agent_created {
            let new_state = self.get_state(&skill_name);
            changes.push((skill_name, new_state));
        }

        info!("Checked {} agent-created skills", changes.len());
        changes
    }

    /// Calculate days since last activity.
    fn inactivity_days(&self, record: &UsageRecord) -> usize {
        // Find latest activity timestamp
        let latest = crate::usage::latest_activity_at(record);
        
        match latest {
            Some(ts) => {
                match chrono::DateTime::parse_from_rfc3339(&ts) {
                    Ok(datetime) => {
                        let now = chrono::Utc::now();
                        let duration = now - datetime.with_timezone(&chrono::Utc);
                        duration.num_days().max(0) as usize
                    }
                    Err(_) => 0,
                }
            }
            None => {
                // No activity recorded
                if record.created_by.is_some() {
                    0
                } else {
                    usize::MAX
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_state_as_str() {
        assert_eq!(LifecycleState::Active.as_str(), "active");
        assert_eq!(LifecycleState::Stale.as_str(), "stale");
        assert_eq!(LifecycleState::Archived.as_str(), "archived");
        assert_eq!(LifecycleState::Pinned.as_str(), "pinned");
    }

    #[test]
    fn test_lifecycle_state_from_str() {
        assert_eq!(LifecycleState::from_str("active"), Some(LifecycleState::Active));
        assert_eq!(LifecycleState::from_str("stale"), Some(LifecycleState::Stale));
        assert_eq!(LifecycleState::from_str("archived"), Some(LifecycleState::Archived));
        assert_eq!(LifecycleState::from_str("pinned"), Some(LifecycleState::Pinned));
        assert_eq!(LifecycleState::from_str("invalid"), None);
    }

    #[test]
    fn test_default_config() {
        let config = LifecycleConfig::default();
        assert_eq!(config.stale_after_days, 30);
        assert_eq!(config.archive_after_days, 90);
    }

    #[test]
    fn test_get_state_nonexistent_skill() {
        let manager = LifecycleManager::new(LifecycleConfig::default());
        let state = manager.get_state("nonexistent-skill");
        assert_eq!(state, LifecycleState::Active);
    }
}
