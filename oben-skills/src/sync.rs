/// Skills Sync - Remote synchronization for skills
/// 
/// Maps to `hermes-agent/skills_sync.py` functionality
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Sync configuration
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Remote sync URL (for cloud sync)
    pub remote_url: Option<String>,
    /// Local skills directory
    pub local_dir: PathBuf,
    /// Whether to sync on startup
    pub sync_on_startup: bool,
    /// Sync interval in seconds
    pub sync_interval: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            remote_url: None,
            local_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skills"))
                .unwrap_or_else(|| PathBuf::from("./skills")),
            sync_on_startup: true,
            sync_interval: 3600, // 1 hour
        }
    }
}

/// Skills Sync manager
pub struct SkillsSync {
    config: SyncConfig,
    last_sync: Option<SystemTime>,
    sync_history: BTreeMap<String, SyncRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncRecord {
    timestamp: SystemTime,
    action: SyncAction,
    skill_name: String,
}

#[derive(Debug, Clone, PartialEq)]
enum SyncAction {
    Upload,
    Download,
    Merge,
}

impl SkillsSync {
    /// Create a new SkillsSync manager
    pub fn new(config: SyncConfig) -> Self {
        Self {
            config,
            last_sync: None,
            sync_history: BTreeMap::new(),
        }
    }
    
    /// Get the current configuration
    pub fn config(&self) -> &SyncConfig {
        &self.config
    }
    
    /// Sync skills with remote
    pub async fn sync(&mut self) -> Result<SyncResult> {
        // Get local skills
        let local_skills = self.get_local_skills()?;
        
        // TODO: Fetch remote skills
        // For now, just return success
        let remote_skills = Vec::new();
        
        // Determine what needs to be synced
        let result = self.analyze_sync(&local_skills, &remote_skills);
        
        // Update last sync time
        self.last_sync = Some(SystemTime::now());
        
        Ok(result)
    }
    
    /// Get all local skills
    fn get_local_skills(&self) -> Result<Vec<String>> {
        let mut skills = Vec::new();
        
        if !self.config.local_dir.exists() {
            return Ok(skills);
        }
        
        for entry in fs::read_dir(&self.config.local_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_file = entry.path().join("SKILL.md");
                if skill_file.exists() {
                    if let Some(name) = entry.file_name().to_str() {
                        skills.push(name.to_string());
                    }
                }
            }
        }
        
        Ok(skills)
    }
    
    /// Analyze what needs to be synced
    fn analyze_sync(&self, local: &[String], remote: &[String]) -> SyncResult {
        let local_set: HashSet<String> = local.iter().cloned().collect();
        let remote_set: HashSet<String> = remote.iter().cloned().collect();
        
        let to_upload: Vec<String> = local_set.difference(&remote_set).cloned().collect();
        let to_download: Vec<String> = remote_set.difference(&local_set).cloned().collect();
        let in_both: Vec<String> = local_set.intersection(&remote_set).cloned().collect();
        
        SyncResult::Completed {
            uploaded: to_upload,
            downloaded: to_download,
            unchanged: in_both,
        }
    }
    
    /// Record a sync operation
    fn record_sync(&mut self, action: SyncAction, skill_name: &str) {
        let record = SyncRecord {
            timestamp: SystemTime::now(),
            action,
            skill_name: skill_name.to_string(),
        };
        
        self.sync_history.insert(
            format!("{}:{}", record.timestamp.duration_since(UNIX_EPOCH).unwrap().as_secs(), skill_name),
            record,
        );
    }
    
    /// Get sync history
    pub fn history(&self) -> Vec<SyncRecord> {
        self.sync_history.values().cloned().collect()
    }
    
    /// Get the last sync time
    pub fn last_sync_time(&self) -> Option<SystemTime> {
        self.last_sync
    }
    
    /// Check if sync is needed based on interval
    pub fn needs_sync(&self) -> bool {
        let Some(last_sync) = self.last_sync else {
            return true;
        };
        
        let now = SystemTime::now();
        let interval = std::time::Duration::from_secs(self.config.sync_interval);
        
        now.duration_since(last_sync).map(|d| d >= interval).unwrap_or(true)
    }
}

/// Result of a sync operation
#[derive(Debug, Clone)]
pub enum SyncResult {
    /// Sync was skipped (no remote configured)
    Skipped,
    /// Sync completed with results
    Completed {
        uploaded: Vec<String>,
        downloaded: Vec<String>,
        unchanged: Vec<String>,
    },
}

impl SyncResult {
    pub fn was_skipped(&self) -> bool {
        matches!(self, SyncResult::Skipped)
    }
    
    pub fn was_completed(&self) -> bool {
        matches!(self, SyncResult::Completed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_sync_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert!(config.sync_on_startup);
        assert_eq!(config.sync_interval, 3600);
    }

    #[test]
    fn test_skills_sync_new() {
        let temp_dir = temp_dir("new_sync");
        let config = SyncConfig {
            local_dir: temp_dir,
            ..Default::default()
        };
        
        let sync = SkillsSync::new(config);
        assert!(sync.last_sync_time().is_none());
        assert!(sync.history().is_empty());
    }

    #[test]
    fn test_get_local_skills_empty() {
        let temp_dir = temp_dir("local_empty");
        let config = SyncConfig {
            local_dir: temp_dir,
            ..Default::default()
        };
        
        let sync = SkillsSync::new(config);
        let skills = sync.get_local_skills().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_get_local_skills_with_skills() {
        let temp_dir = temp_dir("local_skills");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "test").unwrap();
        
        let config = SyncConfig {
            local_dir: temp_dir,
            ..Default::default()
        };
        
        let sync = SkillsSync::new(config);
        let skills = sync.get_local_skills().unwrap();
        assert!(skills.contains(&"test-skill".to_string()));
    }

    #[test]
    fn test_needs_sync_initial() {
        let config = SyncConfig::default();
        let sync = SkillsSync::new(config);
        
        // First check should always need sync
        assert!(sync.needs_sync());
    }

    #[test]
    fn test_needs_sync_after_sync() {
        let config = SyncConfig {
            sync_interval: 60, // 60 seconds for testing
            ..Default::default()
        };
        
        let mut sync = SkillsSync::new(config);
        
        // Mark as just synced
        sync.last_sync = Some(SystemTime::now());
        
        // Should not need sync yet
        assert!(!sync.needs_sync());
    }

    #[test]
    fn test_analyze_sync() {
        let config = SyncConfig::default();
        let sync = SkillsSync::new(config);
        
        let local = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let remote = vec!["b".to_string(), "c".to_string(), "d".to_string()];
        
        let result = sync.analyze_sync(&local, &remote);
        
        match result {
            SyncResult::Completed { uploaded, downloaded, unchanged } => {
                assert_eq!(uploaded, vec!["a"]);
                assert_eq!(downloaded, vec!["d"]);
                let mut unchanged_sorted = unchanged;
                unchanged_sorted.sort();
                assert_eq!(unchanged_sorted, vec!["b", "c"]);
            }
            _ => panic!("Expected Completed result"),
        }
    }

    #[test]
    fn test_record_sync() {
        let config = SyncConfig::default();
        let mut sync = SkillsSync::new(config);
        
        sync.record_sync(SyncAction::Upload, "test-skill");
        
        let history = sync.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].skill_name, "test-skill");
        assert!(matches!(history[0].action, SyncAction::Upload));
    }
}
