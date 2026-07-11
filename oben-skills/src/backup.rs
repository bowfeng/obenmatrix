/// Curator Backup - Periodic skill state backup
/// 
/// Maps to `hermes-agent/curator_backup.py` functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Backup configuration
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// Backup directory path
    pub backup_dir: PathBuf,
    /// Maximum number of backups to keep
    pub max_backups: usize,
    /// Auto-create backup on save
    pub auto_backup: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            backup_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/backup"))
                .unwrap_or_else(|| PathBuf::from("./backup")),
            max_backups: 10,
            auto_backup: true,
        }
    }
}

/// Skill state backup record
#[derive(Debug, Clone)]
pub struct BackupRecord {
    /// Timestamp of backup
    pub timestamp: SystemTime,
    /// Number of skills backed up
    pub skill_count: usize,
    /// Total size in bytes
    pub total_size: u64,
    /// Backup path
    pub backup_path: PathBuf,
}

/// Curator backup manager
pub struct CuratorBackup {
    config: BackupConfig,
    backup_index: BTreeMap<SystemTime, BackupRecord>,
}

impl CuratorBackup {
    /// Create a new CuratorBackup manager
    pub fn new(config: BackupConfig) -> Self {
        let backup_dir = config.backup_dir.clone();
        fs::create_dir_all(&backup_dir).ok();
        
        Self {
            config,
            backup_index: BTreeMap::new(),
        }
    }
    
    /// Create backup of current skill state
    pub fn create_backup(&mut self, skills_dir: &Path) -> Result<BackupRecord> {
        let timestamp = SystemTime::now();
        let backup_name = format!("backup_{}", timestamp.duration_since(UNIX_EPOCH)?.as_secs());
        let backup_path = self.config.backup_dir.join(&backup_name);
        
        fs::create_dir_all(&backup_path)?;
        
        let mut skill_count = 0;
        let mut total_size: u64 = 0;
        
        if skills_dir.exists() {
            for entry in fs::read_dir(skills_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    skill_count += 1;
                    
                    // Copy skill directory to backup
                    let skill_name = entry.file_name();
                    let dest_dir = backup_path.join(&skill_name);
                    fs::create_dir_all(&dest_dir)?;
                    
                    // Copy SKILL.md if exists
                    let skill_md = entry.path().join("SKILL.md");
                    if skill_md.exists() {
                        let dest_md = dest_dir.join("SKILL.md");
                        let size = fs::copy(&skill_md, &dest_md)?;
                        total_size += size;
                    }
                }
            }
        }
        
        let record = BackupRecord {
            timestamp,
            skill_count,
            total_size,
            backup_path: backup_path.clone(),
        };
        
        // Save record metadata
        let metadata_path = backup_path.join("metadata.json");
        let metadata = serde_json::json!({
            "timestamp": timestamp.duration_since(UNIX_EPOCH)?.as_secs(),
            "skill_count": skill_count,
            "total_size": total_size
        });
        fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)?;
        
        self.backup_index.insert(timestamp, record.clone());
        
        // Clean old backups if needed
        self.cleanup_old_backups()?;
        
        Ok(record)
    }
    
    /// Restore from a specific backup
    pub fn restore_from_backup(&self, backup_path: &Path, skills_dir: &Path) -> Result<usize> {
        if !backup_path.exists() {
            anyhow::bail!("Backup path does not exist: {:?}", backup_path);
        }
        
        fs::create_dir_all(skills_dir)?;
        
        let mut restored_count = 0;
        
        for entry in fs::read_dir(backup_path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_name = entry.file_name();
                let dest_dir = skills_dir.join(&skill_name);
                
                // Copy skill directory
                if dest_dir.exists() {
                    fs::remove_dir_all(&dest_dir)?;
                }
                fs::create_dir_all(&dest_dir)?;
                
                // Copy SKILL.md if exists
                let source_md = entry.path().join("SKILL.md");
                let dest_md = dest_dir.join("SKILL.md");
                if source_md.exists() {
                    fs::copy(&source_md, &dest_md)?;
                }
                
                restored_count += 1;
            }
        }
        
        Ok(restored_count)
    }
    
    /// List all available backups
    pub fn list_backups(&self) -> Vec<BackupRecord> {
        self.backup_index.values().cloned().collect()
    }
    
    /// Get the most recent backup
    pub fn latest_backup(&self) -> Option<&BackupRecord> {
        self.backup_index.values().next_back()
    }
    
    /// Delete a specific backup
    pub fn delete_backup(&mut self, backup_path: &Path) -> Result<bool> {
        let timestamp = self.find_backup_timestamp(backup_path)?;
        
        if fs::remove_dir_all(backup_path).is_ok() {
            self.backup_index.remove(&timestamp);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Find timestamp for a backup path
    fn find_backup_timestamp(&self, backup_path: &Path) -> Result<SystemTime> {
        for (timestamp, record) in &self.backup_index {
            if record.backup_path == *backup_path {
                return Ok(*timestamp);
            }
        }
        anyhow::bail!("Backup not found in index: {:?}", backup_path)
    }
    
    /// Clean up old backups keeping only max_backups
    fn cleanup_old_backups(&mut self) -> Result<()> {
        while self.backup_index.len() > self.config.max_backups {
            if let Some(oldest_time) = self.backup_index.keys().next().cloned() {
                if let Some(record) = self.backup_index.remove(&oldest_time) {
                    let _ = fs::remove_dir_all(&record.backup_path);
                }
            } else {
                break;
            }
        }
        Ok(())
    }
    
    /// Get the number of backups
    pub fn backup_count(&self) -> usize {
        self.backup_index.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_backup_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_backup_config_default() {
        let config = BackupConfig::default();
        assert!(config.auto_backup);
        assert_eq!(config.max_backups, 10);
    }

    #[test]
    fn test_curator_backup_new() {
        let temp_dir = temp_dir("new_backup");
        let config = BackupConfig {
            backup_dir: temp_dir,
            ..Default::default()
        };
        
        let backup = CuratorBackup::new(config);
        assert_eq!(backup.backup_count(), 0);
    }

    #[test]
    fn test_create_backup_empty_skills() {
        let temp_dir = temp_dir("empty_backup");
        let skills_dir = temp_dir.join("skills");
        let backup_dir = temp_dir.join("backups");
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        let record = backup.create_backup(&skills_dir).unwrap();
        
        assert_eq!(record.skill_count, 0);
        assert!(record.backup_path.exists());
    }

    #[test]
    fn test_create_backup_with_skills() {
        let temp_dir = temp_dir("with_skills");
        let skills_dir = temp_dir.join("skills");
        let backup_dir = temp_dir.join("backups");
        
        fs::create_dir_all(&skills_dir).unwrap();
        
        // Create a test skill
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test Skill").unwrap();
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        let record = backup.create_backup(&skills_dir).unwrap();
        
        assert_eq!(record.skill_count, 1);
        assert!(record.total_size > 0);
    }

    #[test]
    fn test_list_backups() {
        let temp_dir = temp_dir("list_backups");
        let backup_dir = temp_dir.join("backups");
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            max_backups: 5,
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        
        // Create multiple backups
        for i in 0..3 {
            let skills_dir = temp_dir.join(format!("skills_{}", i));
            fs::create_dir_all(&skills_dir).unwrap();
            let _ = backup.create_backup(&skills_dir);
        }
        
        let backups = backup.list_backups();
        assert_eq!(backups.len(), 3);
    }

    #[test]
    fn test_latest_backup() {
        let temp_dir = temp_dir("latest_backup");
        let backup_dir = temp_dir.join("backups");
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        let skills_dir = temp_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        
        let _ = backup.create_backup(&skills_dir);
        
        let latest = backup.latest_backup();
        assert!(latest.is_some());
    }

    #[test]
    fn test_restore_from_backup() {
        let temp_dir = temp_dir("restore_backup");
        let backup_dir = temp_dir.join("backups");
        let skills_dir = temp_dir.join("skills");
        let restore_dir = temp_dir.join("restored");
        
        fs::create_dir_all(&skills_dir).unwrap();
        
        // Create a test skill
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test Skill").unwrap();
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        let record = backup.create_backup(&skills_dir).unwrap();
        
        // Restore
        let restored = backup.restore_from_backup(&record.backup_path, &restore_dir).unwrap();
        assert_eq!(restored, 1);
        assert!(restore_dir.join("test-skill").exists());
    }

    #[test]
    fn test_delete_backup() {
        let temp_dir = temp_dir("delete_backup");
        let backup_dir = temp_dir.join("backups");
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        let skills_dir = temp_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        
        let record = backup.create_backup(&skills_dir).unwrap();
        
        assert!(backup.delete_backup(&record.backup_path).unwrap());
        assert_eq!(backup.backup_count(), 0);
    }

    #[test]
    fn test_cleanup_old_backups() {
        let temp_dir = temp_dir("cleanup");
        let backup_dir = temp_dir.join("backups");
        
        let config = BackupConfig {
            backup_dir: backup_dir.clone(),
            max_backups: 2,
            ..Default::default()
        };
        
        let mut backup = CuratorBackup::new(config);
        
        // Create 4 backups
        for i in 0..4 {
            let skills_dir = temp_dir.join(format!("skills_{}", i));
            fs::create_dir_all(&skills_dir).unwrap();
            let _ = backup.create_backup(&skills_dir);
        }
        
        assert_eq!(backup.backup_count(), 2); // Should be capped at 2
    }
}
