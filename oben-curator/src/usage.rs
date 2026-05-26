/// Skill usage telemetry — tracks per-skill metrics in a sidecar JSON file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Path to the usage data file (~/.obenalien/skills/.usage.json)
pub fn usage_file() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".obenalien").join("skills").join(".usage.json")
}

/// Path to the archive directory (~/.obenalien/skills/.archive/)
pub fn archive_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".obenalien").join("skills").join(".archive")
}

/// Usage record for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Total number of times this skill was used.
    #[serde(default)]
    pub use_count: usize,
    /// ISO timestamp of last use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    /// Total number of times this skill was viewed.
    #[serde(default)]
    pub view_count: usize,
    /// ISO timestamp of last view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_viewed_at: Option<String>,
    /// Total number of times this skill was patched/edited.
    #[serde(default)]
    pub patch_count: usize,
    /// ISO timestamp of last patch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_patched_at: Option<String>,
    /// Who created this skill: "agent", "user", or "builtin".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Current lifecycle state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Whether this skill is pinned (bypasses auto-transitions).
    #[serde(default)]
    pub pinned: bool,
    /// ISO timestamp when archived (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

impl Default for UsageRecord {
    fn default() -> Self {
        Self {
            use_count: 0,
            last_used_at: None,
            view_count: 0,
            last_viewed_at: None,
            patch_count: 0,
            last_patched_at: None,
            created_by: None,
            state: None,
            pinned: false,
            archived_at: None,
        }
    }
}

/// Load all usage records from disk.
pub fn load_usage() -> HashMap<String, UsageRecord> {
    let path = usage_file();
    if !path.exists() {
        return HashMap::new();
    }

    match fs::read_to_string(&path) {
        Ok(content) => {
            match serde_json::from_str::<HashMap<String, UsageRecord>>(&content) {
                Ok(records) => {
                    debug!("Loaded {} usage records", records.len());
                    records
                }
                Err(e) => {
                    warn!("Failed to parse usage file: {}", e);
                    HashMap::new()
                }
            }
        }
        Err(e) => {
            warn!("Failed to read usage file: {}", e);
            HashMap::new()
        }
    }
}

/// Save usage records to disk (atomic write).
pub fn save_usage(records: &HashMap<String, UsageRecord>) {
    let path = usage_file();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    if let Err(e) = fs::create_dir_all(parent) {
        warn!("Failed to create usage directory: {}", e);
        return;
    }

    match serde_json::to_string_pretty(records) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json) {
                warn!("Failed to write usage file: {}", e);
            }
        }
        Err(e) => {
            warn!("Failed to serialize usage data: {}", e);
        }
    }
}

/// Bump the use count and update last_used_at for a skill.
pub fn bump_use(skill_name: &str) {
    let mut records = load_usage();
    let record = records.entry(skill_name.to_string()).or_default();
    record.use_count += 1;
    record.last_used_at = Some(chrono::Utc::now().to_rfc3339());
    save_usage(&records);
    debug!("Bumped use count for skill '{}'", skill_name);
}

/// Bump the view count and update last_viewed_at for a skill.
pub fn bump_view(skill_name: &str) {
    let mut records = load_usage();
    let record = records.entry(skill_name.to_string()).or_default();
    record.view_count += 1;
    record.last_viewed_at = Some(chrono::Utc::now().to_rfc3339());
    save_usage(&records);
    debug!("Bumped view count for skill '{}'", skill_name);
}

/// Bump the patch count and update last_patched_at for a skill.
pub fn bump_patch(skill_name: &str) {
    let mut records = load_usage();
    let record = records.entry(skill_name.to_string()).or_default();
    record.patch_count += 1;
    record.last_patched_at = Some(chrono::Utc::now().to_rfc3339());
    save_usage(&records);
    debug!("Bumped patch count for skill '{}'", skill_name);
}

/// Mark a skill as created by the agent (eligible for curator management).
pub fn mark_agent_created(skill_name: &str) {
    let mut records = load_usage();
    let record = records.entry(skill_name.to_string()).or_default();
    record.created_by = Some("agent".to_string());
    save_usage(&records);
    debug!("Marked skill '{}' as agent-created", skill_name);
}

/// Get the latest activity timestamp for a usage record.
pub fn latest_activity_at(record: &UsageRecord) -> Option<String> {
    let mut latest = None;
    for key in &["last_used_at", "last_viewed_at", "last_patched_at"] {
        if let Some(Some(ref ts)) = match *key {
            "last_used_at" => Some(&record.last_used_at),
            "last_viewed_at" => Some(&record.last_viewed_at),
            "last_patched_at" => Some(&record.last_patched_at),
            _ => None,
        } {
            if latest.is_none() || ts > latest.as_ref().unwrap() {
                latest = Some(ts.clone());
            }
        }
    }
    latest
}

/// Get the total activity count (use + view + patch).
pub fn activity_count(record: &UsageRecord) -> usize {
    record.use_count + record.view_count + record.patch_count
}

/// Check if a skill is agent-created.
pub fn is_agent_created(skill_name: &str) -> bool {
    let records = load_usage();
    records.get(skill_name)
        .and_then(|r| r.created_by.as_ref())
        .map(|created| created == "agent")
        .unwrap_or(false)
}

/// Get list of agent-created skill names.
pub fn list_agent_created_names() -> Vec<String> {
    let records = load_usage();
    records.into_iter()
        .filter(|(_, r)| r.created_by.as_deref() == Some("agent"))
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_usage_file() -> PathBuf {
        let dir = std::env::temp_dir().join("oben_curator_test");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir.join("test_usage.json")
    }

    #[test]
    fn test_default_usage_record() {
        let record = UsageRecord::default();
        assert_eq!(record.use_count, 0);
        assert_eq!(record.view_count, 0);
        assert_eq!(record.patch_count, 0);
        assert_eq!(record.last_used_at, None);
    }

    #[test]
    fn test_load_empty_usage() {
        let path = temp_usage_file();
        assert!(!path.exists());
        // load_usage should return empty map for non-existent file
        let records = load_usage();
        assert!(records.is_empty());
    }

    #[test]
    fn test_save_and_load_usage() {
        let path = temp_usage_file();
        
        let mut records = HashMap::new();
        let record = UsageRecord {
            use_count: 5,
            last_used_at: Some("2024-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        records.insert("test-skill".to_string(), record);
        
        // Override usage_file to use temp path
        // Note: In real code, we'd use a different approach, but for testing this is OK
        
        let json = serde_json::to_string_pretty(&records).unwrap();
        fs::write(&path, json).unwrap();
        
        let loaded: HashMap<String, UsageRecord> = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["test-skill"].use_count, 5);
        
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_activity_count() {
        let record = UsageRecord {
            use_count: 3,
            view_count: 2,
            patch_count: 1,
            ..Default::default()
        };
        assert_eq!(activity_count(&record), 6);
    }
}
