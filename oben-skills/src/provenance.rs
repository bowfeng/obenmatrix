/// Skill Provenance - Tracking skill origin and history
/// 
/// Maps to `hermes-agent/skill_provenance.py` functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Provenance data for a skill
#[derive(Debug, Clone)]
pub struct SkillProvenance {
    /// Source type (url, git, local, bundle, etc.)
    pub source_type: String,
    /// Source URL or path
    pub source_url: String,
    /// Installation timestamp
    pub installed_at: SystemTime,
    /// Last updated timestamp
    pub last_updated: SystemTime,
    /// Version (if available)
    pub version: Option<String>,
    /// SHA hash of the skill content (for integrity)
    pub content_hash: Option<String>,
}

impl SkillProvenance {
    pub fn new(source_type: impl Into<String>, source_url: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            source_url: source_url.into(),
            installed_at: SystemTime::now(),
            last_updated: SystemTime::now(),
            version: None,
            content_hash: None,
        }
    }
    
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
    
    pub fn content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }
    
    pub fn update_timestamps(mut self) -> Self {
        self.last_updated = SystemTime::now();
        self
    }
}

/// Provenance storage
#[derive(Debug, Clone)]
pub struct ProvenanceStorage {
    /// Path to the provenance storage file
    storage_path: PathBuf,
    /// In-memory cache of provenance data
    cache: BTreeMap<String, SkillProvenance>,
}

impl ProvenanceStorage {
    /// Create a new provenance storage
    pub fn new(storage_path: impl Into<PathBuf>) -> Self {
        let storage_path = storage_path.into();
        Self {
            storage_path,
            cache: BTreeMap::new(),
        }
    }
    
    /// Load provenance from storage
    pub fn load(&mut self) -> Result<Vec<SkillProvenance>> {
        self.cache.clear();
        
        if !self.storage_path.exists() {
            return Ok(Vec::new());
        }
        
        let content = fs::read_to_string(&self.storage_path)?;
        
        // Parse YAML content (simplified for now)
        self.parse_provenance(&content);
        
        Ok(self.cache.values().cloned().collect())
    }
    
    /// Parse provenance from YAML content
    fn parse_provenance(&mut self, content: &str) {
        let mut current_skill: Option<String> = None;
        
        for line in content.lines() {
            if line.starts_with("skills:") {
                continue;
            }
            
            let trimmed = line.trim();
            
            // Check for skill entry start: "- name: skill-name"
            if trimmed.starts_with("- name:") {
                if let Some(skill_name) = trimmed.strip_prefix("- name:").map(|s| s.trim()) {
                    current_skill = Some(skill_name.to_string());
                    self.cache.insert(
                        current_skill.clone().unwrap(),
                        SkillProvenance::new("unknown", "unknown"),
                    );
                }
                continue;
            }
            
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');
                
                if let Some(ref skill_name) = current_skill {
                    if let Some(provenance) = self.cache.get_mut(skill_name) {
                        match key {
                            "source_type" => provenance.source_type = value.to_string(),
                            "source_url" => provenance.source_url = value.to_string(),
                            "installed_at" => {
                                if let Ok(timestamp) = value.parse::<u64>() {
                                    provenance.installed_at = UNIX_EPOCH + std::time::Duration::from_secs(timestamp);
                                }
                            }
                            "last_updated" => {
                                if let Ok(timestamp) = value.parse::<u64>() {
                                    provenance.last_updated = UNIX_EPOCH + std::time::Duration::from_secs(timestamp);
                                }
                            }
                            "version" => provenance.version = Some(value.to_string()),
                            "content_hash" => provenance.content_hash = Some(value.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    /// Save provenance to storage
    pub fn save(&self) -> Result<()> {
        let content = self.to_yaml();
        fs::create_dir_all(self.storage_path.parent().unwrap())?;
        fs::write(&self.storage_path, &content)?;
        Ok(())
    }
    
    /// Convert to YAML format
    fn to_yaml(&self) -> String {
        let mut yaml = String::from("skills:\n");
        
        for (name, provenance) in &self.cache {
            yaml.push_str(&format!("  - name: {}\n", name));
            yaml.push_str(&format!("    source_type: {}\n", provenance.source_type));
            yaml.push_str(&format!("    source_url: {}\n", provenance.source_url));
            
            if let Ok(timestamp) = provenance.installed_at.duration_since(UNIX_EPOCH) {
                yaml.push_str(&format!("    installed_at: {}\n", timestamp.as_secs()));
            }
            
            if let Ok(timestamp) = provenance.last_updated.duration_since(UNIX_EPOCH) {
                yaml.push_str(&format!("    last_updated: {}\n", timestamp.as_secs()));
            }
            
            if let Some(ref version) = provenance.version {
                yaml.push_str(&format!("    version: {}\n", version));
            }
            
            if let Some(ref hash) = provenance.content_hash {
                yaml.push_str(&format!("    content_hash: {}\n", hash));
            }
        }
        
        yaml
    }
    
    /// Record provenance for a skill
    pub fn record(&mut self, skill_name: &str, provenance: SkillProvenance) {
        self.cache.insert(skill_name.to_string(), provenance);
    }
    
    /// Get provenance for a skill
    pub fn get(&self, skill_name: &str) -> Option<&SkillProvenance> {
        self.cache.get(skill_name)
    }
    
    /// Remove provenance for a skill
    pub fn remove(&mut self, skill_name: &str) -> bool {
        self.cache.remove(skill_name).is_some()
    }
    
    /// Check if a skill has provenance recorded
    pub fn has_provenance(&self, skill_name: &str) -> bool {
        self.cache.contains_key(skill_name)
    }
    
    /// Clear all provenance data
    pub fn clear(&mut self) {
        self.cache.clear();
    }
    
    /// List all skills with provenance
    pub fn list(&self) -> Vec<SkillProvenance> {
        self.cache.values().cloned().collect()
    }
}

impl Default for ProvenanceStorage {
    fn default() -> Self {
        let storage_path = std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".agents/skills_provenance.yaml"))
            .unwrap_or_else(|| PathBuf::from("./skills_provenance.yaml"));
        
        Self::new(storage_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_provenance_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_skill_provenance_new() {
        let provenance = SkillProvenance::new("url", "https://example.com/skill");
        
        assert_eq!(provenance.source_type, "url");
        assert_eq!(provenance.source_url, "https://example.com/skill");
        assert!(provenance.version.is_none());
        assert!(provenance.content_hash.is_none());
    }

    #[test]
    fn test_skill_provenance_builder() {
        let provenance = SkillProvenance::new("git", "https://github.com/user/skill")
            .version("1.0.0")
            .content_hash("abc123def456");
        
        assert_eq!(provenance.source_type, "git");
        assert_eq!(provenance.version, Some("1.0.0".to_string()));
        assert_eq!(provenance.content_hash, Some("abc123def456".to_string()));
    }

    #[test]
    fn test_provenance_storage_default() {
        let storage = ProvenanceStorage::default();
        assert!(storage.storage_path.exists() || storage.storage_path.parent().is_some());
    }

    #[test]
    fn test_record_and_get_provenance() {
        let temp_dir = temp_dir("record_get");
        let storage_path = temp_dir.join("provenance.yaml");
        let mut storage = ProvenanceStorage::new(&storage_path);
        
        let provenance = SkillProvenance::new("url", "https://example.com/skill");
        storage.record("test-skill", provenance);
        
        let retrieved = storage.get("test-skill");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().source_type, "url");
    }

    #[test]
    fn test_remove_provenance() {
        let temp_dir = temp_dir("remove");
        let storage_path = temp_dir.join("provenance.yaml");
        let mut storage = ProvenanceStorage::new(&storage_path);
        
        let provenance = SkillProvenance::new("url", "https://example.com/skill");
        storage.record("test-skill", provenance);
        
        assert!(storage.has_provenance("test-skill"));
        
        let removed = storage.remove("test-skill");
        assert!(removed);
        assert!(!storage.has_provenance("test-skill"));
    }

    #[test]
    fn test_list_provenance() {
        let temp_dir = temp_dir("list");
        let storage_path = temp_dir.join("provenance.yaml");
        let mut storage = ProvenanceStorage::new(&storage_path);
        
        storage.record("skill1", SkillProvenance::new("url", "https://example.com/1"));
        storage.record("skill2", SkillProvenance::new("git", "https://github.com/2"));
        
        let list = storage.list();
        assert_eq!(list.len(), 2);
        
        let names: Vec<&str> = list.iter().map(|p| {
            if p.source_url.contains("1") { "skill1" } else { "skill2" }
        }).collect();
        
        assert!(names.contains(&"skill1"));
        assert!(names.contains(&"skill2"));
    }

    #[test]
    fn test_to_yaml() {
        let temp_dir = temp_dir("yaml");
        let storage_path = temp_dir.join("provenance.yaml");
        let mut storage = ProvenanceStorage::new(&storage_path);
        
        let provenance = SkillProvenance::new("url", "https://example.com/skill")
            .version("1.0.0")
            .content_hash("abc123");
        
        storage.record("test-skill", provenance);
        
        let yaml = storage.to_yaml();
        assert!(yaml.contains("skills:"));
        assert!(yaml.contains("test-skill"));
        assert!(yaml.contains("source_type: url"));
        assert!(yaml.contains("version: 1.0.0"));
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = temp_dir("save_load");
        let storage_path = temp_dir.join("provenance.yaml");
        
        {
            let mut storage = ProvenanceStorage::new(&storage_path);
            let provenance = SkillProvenance::new("url", "https://example.com/skill");
            storage.record("test-skill", provenance);
            storage.save().unwrap();
        }
        
        let mut storage = ProvenanceStorage::new(&storage_path);
        storage.load().unwrap();
        
        assert!(storage.has_provenance("test-skill"));
    }

    #[test]
    fn test_clear_provenance() {
        let temp_dir = temp_dir("clear");
        let storage_path = temp_dir.join("provenance.yaml");
        let mut storage = ProvenanceStorage::new(&storage_path);
        
        storage.record("skill1", SkillProvenance::new("url", "https://example.com/1"));
        storage.record("skill2", SkillProvenance::new("git", "https://github.com/2"));
        
        assert_eq!(storage.list().len(), 2);
        
        storage.clear();
        
        assert!(storage.list().is_empty());
    }
}
