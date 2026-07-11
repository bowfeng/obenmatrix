/// Skill Lifecycle Manager - Manage skill lifecycle states
/// 
/// Maps to `hermes-agent/skills_hub.py` lifecycle management functionality
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Skill lifecycle states
#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub enum SkillLifecycleState {
    /// Skill is active and being used
    Active,
    /// Skill is not currently in use but available
    Inactive,
    /// Skill has not been used in a while
    Stale,
    /// Skill should be archived
    Archived,
    /// Skill is deprecated
    Deprecated,
}

/// Lifecycle configuration
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Stale threshold in seconds (default: 30 days)
    pub stale_threshold: u64,
    /// Archive threshold in seconds (default: 90 days)
    pub archive_threshold: u64,
    /// Auto-transition to stale
    pub auto_transition: bool,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            stale_threshold: 30 * 24 * 60 * 60, // 30 days
            archive_threshold: 90 * 24 * 60 * 60, // 90 days
            auto_transition: true,
        }
    }
}

/// Skill lifecycle record
#[derive(Debug, Clone)]
pub struct LifecycleRecord {
    /// Skill name
    pub skill_name: String,
    /// Current lifecycle state
    pub state: SkillLifecycleState,
    /// When the skill was first registered
    pub registered_at: SystemTime,
    /// When the skill was last active
    pub last_active: SystemTime,
    /// When the skill transitioned to current state
    pub state_changed_at: SystemTime,
    /// Number of uses
    pub usage_count: u64,
}

/// Skill lifecycle manager
pub struct SkillLifecycleManager {
    config: LifecycleConfig,
    lifecycle_data: BTreeMap<String, LifecycleRecord>,
    storage_path: PathBuf,
}

impl SkillLifecycleManager {
    /// Parse state from YAML string
    fn parse_state(s: &str) -> Option<SkillLifecycleState> {
        match s {
            "Active" => Some(SkillLifecycleState::Active),
            "Inactive" => Some(SkillLifecycleState::Inactive),
            "Stale" => Some(SkillLifecycleState::Stale),
            "Archived" => Some(SkillLifecycleState::Archived),
            "Deprecated" => Some(SkillLifecycleState::Deprecated),
            _ => None,
        }
    }
    
    /// Create a new lifecycle manager
    pub fn new(config: LifecycleConfig, storage_path: impl Into<PathBuf>) -> Self {
        let storage_path = storage_path.into();
        fs::create_dir_all(storage_path.parent().unwrap_or(&storage_path)).ok();
        
        Self {
            config,
            lifecycle_data: BTreeMap::new(),
            storage_path,
        }
    }
    
    /// Register a skill for lifecycle management
    pub fn register(&mut self, skill_name: &str) -> Result<()> {
        let now = SystemTime::now();
        
        let record = LifecycleRecord {
            skill_name: skill_name.to_string(),
            state: SkillLifecycleState::Active,
            registered_at: now,
            last_active: now,
            state_changed_at: now,
            usage_count: 0,
        };
        
        self.lifecycle_data.insert(skill_name.to_string(), record);
        
        Ok(())
    }
    
    /// Update skill activity
    pub fn update_activity(&mut self, skill_name: &str, usage_count: u64) -> Result<()> {
        let now = SystemTime::now();
        
        if let Some(record) = self.lifecycle_data.get_mut(skill_name) {
            record.last_active = now;
            record.usage_count = usage_count;
            
            // Transition to active if not already
            if record.state != SkillLifecycleState::Active {
                record.state = SkillLifecycleState::Active;
                record.state_changed_at = now;
            }
        } else {
            // Register if not already registered
            self.register(skill_name)?;
            self.lifecycle_data.get_mut(skill_name).unwrap().usage_count = usage_count;
        }
        
        Ok(())
    }
    
    /// Check and update lifecycle states
    pub fn check_lifecycle(&mut self) -> Result<Vec<String>> {
        let now = SystemTime::now();
        let mut changed = Vec::new();
        
        for (skill_name, record) in &mut self.lifecycle_data {
            let age = now.duration_since(record.last_active).unwrap_or_default();
            let age_seconds = age.as_secs();
            
            let new_state = if age_seconds >= self.config.archive_threshold {
                SkillLifecycleState::Archived
            } else if age_seconds >= self.config.stale_threshold {
                SkillLifecycleState::Stale
            } else if record.state == SkillLifecycleState::Stale || record.state == SkillLifecycleState::Archived {
                SkillLifecycleState::Inactive
            } else {
                SkillLifecycleState::Active
            };
            
            if new_state != record.state {
                record.state = new_state;
                record.state_changed_at = now;
                changed.push(skill_name.clone());
            }
        }
        
        Ok(changed)
    }
    
    /// Get the state of a skill
    pub fn get_state(&self, skill_name: &str) -> Option<&SkillLifecycleState> {
        self.lifecycle_data.get(skill_name).map(|r| &r.state)
    }
    
    /// Get all lifecycle records
    pub fn get_all_records(&self) -> Vec<&LifecycleRecord> {
        self.lifecycle_data.values().collect()
    }
    
    /// Get skills by state
    pub fn by_state(&self, state: &SkillLifecycleState) -> Vec<&LifecycleRecord> {
        self.lifecycle_data
            .values()
            .filter(|r| &r.state == state)
            .collect()
    }
    
    /// Archive a skill
    pub fn archive(&mut self, skill_name: &str) -> Result<bool> {
        if let Some(record) = self.lifecycle_data.get_mut(skill_name) {
            record.state = SkillLifecycleState::Archived;
            record.state_changed_at = SystemTime::now();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// De-archive a skill
    pub fn dearchive(&mut self, skill_name: &str) -> Result<bool> {
        if let Some(record) = self.lifecycle_data.get_mut(skill_name) {
            record.state = SkillLifecycleState::Inactive;
            record.state_changed_at = SystemTime::now();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Mark a skill as deprecated
    pub fn deprecate(&mut self, skill_name: &str) -> Result<bool> {
        if let Some(record) = self.lifecycle_data.get_mut(skill_name) {
            record.state = SkillLifecycleState::Deprecated;
            record.state_changed_at = SystemTime::now();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Save lifecycle data to file
    pub fn save(&self) -> Result<()> {
        let content = self.to_yaml();
        fs::write(&self.storage_path, &content)?;
        Ok(())
    }
    
    /// Load lifecycle data from file
    pub fn load(&mut self) -> Result<()> {
        if !self.storage_path.exists() {
            return Ok(());
        }
        
        let content = fs::read_to_string(&self.storage_path)?;
        self.from_yaml(&content);
        
        Ok(())
    }
    
    /// Convert to YAML format
    fn to_yaml(&self) -> String {
        let mut yaml = String::from("skills:\n");
        
        for (name, record) in &self.lifecycle_data {
            yaml.push_str(&format!("  {}:\n", name));
            yaml.push_str(&format!("    state: {:?}\n", record.state));
            yaml.push_str(&format!("    registered_at: {}\n", record.registered_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
            yaml.push_str(&format!("    last_active: {}\n", record.last_active.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
            yaml.push_str(&format!("    state_changed_at: {}\n", record.state_changed_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
            yaml.push_str(&format!("    usage_count: {}\n", record.usage_count));
        }
        
        yaml
    }
    
    /// Parse from YAML format
    fn from_yaml(&mut self, content: &str) {
        let mut current_skill: Option<String> = None;
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("skills:") {
                continue;
            }
            
            // Skill name (2-space indent, ends with :)
            if line.starts_with("  ") && !line.trim_start().starts_with("    ") && trimmed.ends_with(':') {
                let skill_name = trimmed.strip_suffix(':').unwrap_or("").to_string();
                let record = LifecycleRecord {
                    state: SkillLifecycleState::Active,
                    registered_at: UNIX_EPOCH,
                    last_active: UNIX_EPOCH,
                    state_changed_at: UNIX_EPOCH,
                    usage_count: 0,
                    skill_name: skill_name.clone(),
                };
                current_skill = Some(skill_name.clone());
                self.lifecycle_data.insert(skill_name, record);
            } else if let Some(ref skill_name) = current_skill {
                // Parse fields (4+ space indent)
                if let Some((key, value)) = trimmed.split_once(':') {
                    let value = value.trim();
                    if let Some(record) = self.lifecycle_data.get_mut(skill_name) {
                        match key.trim() {
                            "state" => {
                                // Parse state enum
                                record.state = Self::parse_state(value).unwrap_or(SkillLifecycleState::Active);
                            }
                            "registered_at" => record.registered_at = UNIX_EPOCH + std::time::Duration::from_secs(value.parse().unwrap_or(0)),
                            "last_active" => record.last_active = UNIX_EPOCH + std::time::Duration::from_secs(value.parse().unwrap_or(0)),
                            "state_changed_at" => record.state_changed_at = UNIX_EPOCH + std::time::Duration::from_secs(value.parse().unwrap_or(0)),
                            "usage_count" => record.usage_count = value.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    /// Get the number of skills in each state
    pub fn state_counts(&self) -> BTreeMap<&SkillLifecycleState, usize> {
        let mut counts = BTreeMap::new();
        
        for record in self.lifecycle_data.values() {
            *counts.entry(&record.state).or_insert(0) += 1;
        }
        
        counts
    }
}

impl Default for SkillLifecycleManager {
    fn default() -> Self {
        Self::new(
            LifecycleConfig::default(),
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/lifecycle.yaml"))
                .unwrap_or_else(|| PathBuf::from("./lifecycle.yaml")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_lifecycle_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_register_skill() {
        let temp_dir = temp_dir("register");
        let config = LifecycleConfig::default();
        
        let mut manager = SkillLifecycleManager::new(config, temp_dir);
        
        manager.register("test-skill").unwrap();
        
        assert!(manager.get_state("test-skill").is_some());
        assert_eq!(manager.get_state("test-skill").unwrap(), &SkillLifecycleState::Active);
    }

    #[test]
    fn test_update_activity() {
        let temp_dir = temp_dir("activity");
        let config = LifecycleConfig::default();
        
        let mut manager = SkillLifecycleManager::new(config, temp_dir);
        
        manager.register("test-skill").unwrap();
        manager.update_activity("test-skill", 5).unwrap();
        
        let record = manager.lifecycle_data.get("test-skill").unwrap();
        assert_eq!(record.usage_count, 5);
    }

    #[test]
    fn test_check_lifecycle() {
        let temp_dir = temp_dir("check");
        let config = LifecycleConfig {
            stale_threshold: 1, // 1 second for testing
            ..Default::default()
        };
        
        let mut manager = SkillLifecycleManager::new(config, temp_dir);
        
        manager.register("test-skill").unwrap();
        
        // Simulate time passing
        std::thread::sleep(std::time::Duration::from_secs(2));
        
        let changed = manager.check_lifecycle().unwrap();
        assert!(!changed.is_empty());
    }

    #[test]
    fn test_archive_and_dearchive() {
        let temp_dir = temp_dir("archive");
        let config = LifecycleConfig::default();
        
        let mut manager = SkillLifecycleManager::new(config, temp_dir);
        
        manager.register("test-skill").unwrap();
        
        // Archive
        assert!(manager.archive("test-skill").unwrap());
        assert_eq!(manager.get_state("test-skill").unwrap(), &SkillLifecycleState::Archived);
        
        // Dearchive
        assert!(manager.dearchive("test-skill").unwrap());
        assert_eq!(manager.get_state("test-skill").unwrap(), &SkillLifecycleState::Inactive);
    }

    #[test]
    fn test_state_counts() {
        let temp_dir = temp_dir("counts");
        let config = LifecycleConfig::default();
        
        let mut manager = SkillLifecycleManager::new(config, temp_dir);
        
        manager.register("skill1").unwrap();
        manager.register("skill2").unwrap();
        manager.register("skill3").unwrap();
        
        // Archive one
        manager.archive("skill3").unwrap();
        
        let counts = manager.state_counts();
        assert_eq!(counts.get(&SkillLifecycleState::Active), Some(&2));
        assert_eq!(counts.get(&SkillLifecycleState::Archived), Some(&1));
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = temp_dir("save_load");
        let config = LifecycleConfig::default();
        
        let storage_path = temp_dir.join("lifecycle.yaml");
        let mut manager1 = SkillLifecycleManager::new(config.clone(), storage_path.clone());
        manager1.register("test-skill").unwrap();
        manager1.save().unwrap();
        
        let mut manager2 = SkillLifecycleManager::new(config, storage_path);
        manager2.load().unwrap();
        
        assert!(manager2.get_state("test-skill").is_some());
    }
}
