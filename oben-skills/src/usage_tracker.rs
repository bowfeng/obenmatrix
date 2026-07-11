/// Skill Usage Tracker - Track skill usage statistics
/// 
/// Maps to `hermes-agent/skills_hub.py` usage tracking functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Usage record for a skill
#[derive(Debug, Clone)]
pub struct UsageRecord {
    /// Skill name
    pub skill_name: String,
    /// Number of times used
    pub usage_count: u64,
    /// Last used timestamp
    pub last_used: SystemTime,
    /// First used timestamp
    pub first_used: SystemTime,
    /// Total execution time in milliseconds (approximate)
    pub total_time_ms: u64,
}

/// Usage tracking configuration
#[derive(Debug, Clone)]
pub struct UsageConfig {
    /// Storage path for usage data
    pub storage_path: PathBuf,
    /// Maximum number of records to keep per skill
    pub max_records: usize,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            storage_path: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/usage_tracking.yaml"))
                .unwrap_or_else(|| PathBuf::from("./usage_tracking.yaml")),
            max_records: 1000,
        }
    }
}

/// Skill usage tracker
pub struct SkillUsageTracker {
    config: UsageConfig,
    usage_data: BTreeMap<String, UsageRecord>,
}

impl SkillUsageTracker {
    /// Create a new usage tracker
    pub fn new(config: UsageConfig) -> Self {
        let storage_path = config.storage_path.clone();
        fs::create_dir_all(storage_path.parent().unwrap_or(&storage_path)).ok();
        
        Self {
            config,
            usage_data: BTreeMap::new(),
        }
    }
    
    /// Record skill usage
    pub fn record_usage(&mut self, skill_name: &str, execution_time_ms: u64) -> Result<()> {
        let now = SystemTime::now();
        let timestamp = now.duration_since(UNIX_EPOCH)?.as_secs();
        
        let record = self.usage_data.entry(skill_name.to_string())
            .or_insert_with(|| UsageRecord {
                skill_name: skill_name.to_string(),
                usage_count: 0,
                last_used: now,
                first_used: now,
                total_time_ms: 0,
            });
        
        record.usage_count += 1;
        record.last_used = now;
        record.total_time_ms += execution_time_ms;
        
        // Limit record size
        if self.usage_data.len() > self.config.max_records {
            // Remove oldest entries (BTreeMap is sorted)
            if let Some(key) = self.usage_data.keys().next().cloned() {
                self.usage_data.remove(&key);
            }
        }
        
        Ok(())
    }
    
    /// Get usage record for a skill
    pub fn get_usage(&self, skill_name: &str) -> Option<&UsageRecord> {
        self.usage_data.get(skill_name)
    }
    
    /// Get all usage records
    pub fn get_all_usage(&self) -> Vec<&UsageRecord> {
        self.usage_data.values().collect()
    }
    
    /// Get top used skills
    pub fn top_used(&self, limit: usize) -> Vec<&UsageRecord> {
        let mut records: Vec<&UsageRecord> = self.usage_data.values().collect();
        records.sort_by(|a, b| b.usage_count.cmp(&a.usage_count));
        records.into_iter().take(limit).collect()
    }
    
    /// Save usage data to file
    pub fn save(&self) -> Result<()> {
        let content = self.to_yaml();
        fs::write(&self.config.storage_path, &content)?;
        Ok(())
    }
    
    /// Load usage data from file
    pub fn load(&mut self) -> Result<()> {
        if !self.config.storage_path.exists() {
            return Ok(());
        }
        
        let content = fs::read_to_string(&self.config.storage_path)?;
        self.from_yaml(&content);
        
        Ok(())
    }
    
    /// Convert to YAML format
    fn to_yaml(&self) -> String {
        let mut yaml = String::from("skills:\n");
        
        for (name, record) in &self.usage_data {
            yaml.push_str(&format!("  {}:\n", name));
            yaml.push_str(&format!("    usage_count: {}\n", record.usage_count));
            yaml.push_str(&format!("    first_used: {}\n", record.first_used.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
            yaml.push_str(&format!("    last_used: {}\n", record.last_used.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
            yaml.push_str(&format!("    total_time_ms: {}\n", record.total_time_ms));
        }
        
        yaml
    }
    
    /// Parse from YAML format
    fn from_yaml(&mut self, content: &str) {
        let mut current_skill: Option<String> = None;
        let mut current_indent_level: usize = 0;
        
        for line in content.lines() {
            // Skip empty lines and the skills: header
            if line.trim().is_empty() || line.trim().starts_with("skills:") {
                continue;
            }
            
            let trimmed = line.trim();
            
            // Check indentation level to determine if we're at skill name or field level
            let indent = line.len() - line.trim_start().len();
            
            if indent == 2 && trimmed.ends_with(':') {
                // This is a skill name line (2-space indent, ends with colon)
                if let Some(skill_name_str) = trimmed.strip_suffix(':').map(|s| s.trim().to_string()) {
                    let skill_name = skill_name_str.clone();
                    current_skill = Some(skill_name.clone());
                    self.usage_data.insert(
                        skill_name_str,
                        UsageRecord {
                            usage_count: 0,
                            last_used: UNIX_EPOCH,
                            first_used: UNIX_EPOCH,
                            total_time_ms: 0,
                            skill_name: skill_name,
                        },
                    );
                }
            } else if indent >= 4 && current_skill.is_some() {
                // This is a field line (4+ space indent) under a skill
                if let Some((key, value)) = trimmed.split_once(':') {
                    let value = value.trim().parse::<u64>().ok();
                    if let Some(value) = value {
                        if let Some(ref skill_name) = current_skill {
                            if let Some(record) = self.usage_data.get_mut(skill_name) {
                                match key.trim() {
                                    "usage_count" => record.usage_count = value,
                                    "first_used" => record.first_used = UNIX_EPOCH + std::time::Duration::from_secs(value),
                                    "last_used" => record.last_used = UNIX_EPOCH + std::time::Duration::from_secs(value),
                                    "total_time_ms" => record.total_time_ms = value,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    /// Get the total number of skills tracked
    pub fn skill_count(&self) -> usize {
        self.usage_data.len()
    }
    
    /// Get total usage count across all skills
    pub fn total_usage_count(&self) -> u64 {
        self.usage_data.values().map(|r| r.usage_count).sum()
    }
    
    /// Clear all usage data
    pub fn clear(&mut self) {
        self.usage_data.clear();
    }
}

impl Default for SkillUsageTracker {
    fn default() -> Self {
        Self::new(UsageConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_usage_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_record_usage() {
        let temp_dir = temp_dir("record");
        let config = UsageConfig {
            storage_path: temp_dir.join("usage.yaml"),
            ..Default::default()
        };
        
        let mut tracker = SkillUsageTracker::new(config);
        
        tracker.record_usage("test-skill", 100).unwrap();
        tracker.record_usage("test-skill", 50).unwrap();
        
        let record = tracker.get_usage("test-skill").unwrap();
        assert_eq!(record.usage_count, 2);
        assert_eq!(record.total_time_ms, 150);
    }

    #[test]
    fn test_top_used() {
        let temp_dir = temp_dir("top_used");
        let config = UsageConfig {
            storage_path: temp_dir.join("usage.yaml"),
            ..Default::default()
        };
        
        let mut tracker = SkillUsageTracker::new(config);
        
        tracker.record_usage("skill1", 10).unwrap();
        tracker.record_usage("skill2", 20).unwrap();
        tracker.record_usage("skill3", 30).unwrap();
        
        let top = tracker.top_used(2);
        // All skills have usage_count=1, so they're sorted alphabetically
        // First two are skill1 and skill2
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].skill_name, "skill1");
        assert_eq!(top[1].skill_name, "skill2");
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = temp_dir("save_load");
        let config = UsageConfig {
            storage_path: temp_dir.join("usage.yaml"),
            ..Default::default()
        };
        
        let mut tracker1 = SkillUsageTracker::new(config.clone());
        tracker1.record_usage("test-skill", 100).unwrap();
        tracker1.save().unwrap();
        
        let mut tracker2 = SkillUsageTracker::new(config);
        tracker2.load().unwrap();
        
        let record = tracker2.get_usage("test-skill").unwrap();
        assert_eq!(record.usage_count, 1, "Usage count should be 1 after one record");
    }

    #[test]
    fn test_clear() {
        let temp_dir = temp_dir("clear");
        let config = UsageConfig {
            storage_path: temp_dir.join("usage.yaml"),
            ..Default::default()
        };
        
        let mut tracker = SkillUsageTracker::new(config);
        
        tracker.record_usage("skill1", 10).unwrap();
        tracker.record_usage("skill2", 20).unwrap();
        
        assert_eq!(tracker.skill_count(), 2);
        
        tracker.clear();
        
        assert_eq!(tracker.skill_count(), 0);
    }

    #[test]
    fn test_total_usage_count() {
        let temp_dir = temp_dir("total");
        let config = UsageConfig {
            storage_path: temp_dir.join("usage.yaml"),
            ..Default::default()
        };
        
        let mut tracker = SkillUsageTracker::new(config);
        
        tracker.record_usage("skill1", 10).unwrap();
        tracker.record_usage("skill2", 20).unwrap();
        tracker.record_usage("skill3", 30).unwrap();
        
        // Each record increments usage_count by 1, so total is 3
        assert_eq!(tracker.total_usage_count(), 3);
    }
}
