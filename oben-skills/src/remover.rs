/// Skill Remover - Remove and archive installed skills
///
/// Maps to `hermes-agent/skills_hub.py` uninstall and archive functionality
use anyhow::Result;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

/// Removal options
#[derive(Debug, Clone)]
pub struct RemoveOptions {
    /// Force removal even if skill is in use
    pub force: bool,
    /// Also remove skill data
    pub remove_data: bool,
    /// Dry run (don't actually remove)
    pub dry_run: bool,
}

impl Default for RemoveOptions {
    fn default() -> Self {
        Self {
            force: false,
            remove_data: false,
            dry_run: false,
        }
    }
}

/// Archive options
#[derive(Debug, Clone)]
pub struct ArchiveOptions {
    /// Path to absorb the skill into (consolidation target)
    pub absorb_into: Option<String>,
    /// Also remove skill data after archiving
    pub remove_data: bool,
    /// Dry run (don't actually archive)
    pub dry_run: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            absorb_into: None,
            remove_data: false,
            dry_run: false,
        }
    }
}

/// Removal result
#[derive(Debug, Clone)]
pub struct RemoveResult {
    /// Skill that was removed
    pub skill_name: String,
    /// Path that was removed
    pub path: PathBuf,
    /// Whether removal succeeded
    pub success: bool,
    /// Optional error message if removal failed
    pub error: Option<String>,
}

/// Archive result
#[derive(Debug, Clone)]
pub struct ArchiveResult {
    /// Skill that was archived
    pub skill_name: String,
    /// Path where skill was moved
    pub path: PathBuf,
    /// Whether archiving succeeded
    pub success: bool,
    /// Optional error message if archiving failed
    pub error: Option<String>,
    /// Timestamp when absorption was recorded
    pub absorption_timestamp: chrono::DateTime<Utc>,
    /// Optional skill name this was absorbed into
    pub absorbed_into: Option<String>,
}

/// Skill remover
pub struct SkillRemover {
    skills_dir: PathBuf,
    data_dir: Option<PathBuf>,
}

impl SkillRemover {
    /// Create a new skill remover
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            data_dir: None,
        }
    }

    /// Set the data directory to remove
    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    /// Remove a skill by name
    pub fn remove(&self, skill_name: &str, options: &RemoveOptions) -> Result<RemoveResult> {
        let skill_path = self.skills_dir.join(skill_name);

        if !skill_path.exists() {
            return Ok(RemoveResult {
                skill_name: skill_name.to_string(),
                path: skill_path,
                success: false,
                error: Some(format!("Skill not found: {}", skill_name)),
            });
        }

        if options.dry_run {
            return Ok(RemoveResult {
                skill_name: skill_name.to_string(),
                path: skill_path,
                success: true,
                error: None,
            });
        }

        // Check if force removal is needed
        if !options.force {
            if self.is_skill_in_use(skill_name)? {
                return Ok(RemoveResult {
                    skill_name: skill_name.to_string(),
                    path: skill_path,
                    success: false,
                    error: Some(format!("Skill is currently in use: {}", skill_name)),
                });
            }
        }

        // Remove skill directory
        fs::remove_dir_all(&skill_path)?;

        // Remove data if requested
        if options.remove_data {
            if let Some(ref data_dir) = self.data_dir {
                let skill_data = data_dir.join(skill_name);
                if skill_data.exists() {
                    fs::remove_dir_all(&skill_data)?;
                }
            }
        }

        Ok(RemoveResult {
            skill_name: skill_name.to_string(),
            path: skill_path,
            success: true,
            error: None,
        })
    }

    /// Archive a skill by moving it to .archive directory
    /// Optionally record absorption metadata
    pub fn archive(&self, skill_name: &str, options: &ArchiveOptions) -> Result<ArchiveResult> {
        let skill_path = self.skills_dir.join(skill_name);

        if !skill_path.exists() {
            return Ok(ArchiveResult {
                skill_name: skill_name.to_string(),
                path: skill_path,
                success: false,
                error: Some(format!("Skill not found: {}", skill_name)),
                absorption_timestamp: Utc::now(),
                absorbed_into: None,
            });
        }

        if options.dry_run {
            return Ok(ArchiveResult {
                skill_name: skill_name.to_string(),
                path: skill_path.clone(),
                success: true,
                error: None,
                absorption_timestamp: Utc::now(),
                absorbed_into: options.absorb_into.clone(),
            });
        }

        let archive_dir = self.skills_dir.join(".archive");
        fs::create_dir_all(&archive_dir)?;

        let archive_path = archive_dir.join(skill_name);
        fs::rename(&skill_path, &archive_path)?;

        if options.remove_data {
            if let Some(ref data_dir) = self.data_dir {
                let skill_data = data_dir.join(skill_name);
                if skill_data.exists() {
                    fs::remove_dir_all(&skill_data)?;
                }
            }
        }

        Ok(ArchiveResult {
            skill_name: skill_name.to_string(),
            path: archive_path,
            success: true,
            error: None,
            absorption_timestamp: Utc::now(),
            absorbed_into: options.absorb_into.clone(),
        })
    }

    /// Check if a skill is currently in use
    fn is_skill_in_use(&self, _skill_name: &str) -> Result<bool> {
        // For now, always return false (not in use)
        // In a real implementation, this would check for active processes, locks, etc.
        Ok(false)
    }

    /// Remove multiple skills
    pub fn remove_many(&self, skill_names: &[&str], options: &RemoveOptions) -> Result<Vec<RemoveResult>> {
        let mut results = Vec::new();

        for skill_name in skill_names {
            results.push(self.remove(skill_name, options)?);
        }

        Ok(results)
    }

    /// Remove all skills
    pub fn remove_all(&self, options: &RemoveOptions) -> Result<usize> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;

        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(_name) = entry.file_name().to_str() {
                    if !options.dry_run {
                        fs::remove_dir_all(entry.path())?;
                    }
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Archive multiple skills
    pub fn archive_many(&self, skill_names: &[&str], options: &ArchiveOptions) -> Result<Vec<ArchiveResult>> {
        let mut results = Vec::new();

        for skill_name in skill_names {
            results.push(self.archive(skill_name, options)?);
        }

        Ok(results)
    }

    /// Get the skills directory
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    /// Get the data directory if configured
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }
}

impl Default for SkillRemover {
    fn default() -> Self {
        Self::new(std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".agents/skills"))
            .unwrap_or_else(|| PathBuf::from("./skills")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_remove_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn setup_archive_test_dir(name: &str) -> PathBuf {
        let temp_dir = temp_dir(name);
        let skills_dir = temp_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        skills_dir
    }

    #[test]
    fn test_remove_skill_not_found() {
        let skills_dir = setup_archive_test_dir("not_found");
        let remover = SkillRemover::new(&skills_dir);
        let options = RemoveOptions::default();

        let result = remover.remove("nonexistent", &options).unwrap();

        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_remove_skill_success() {
        let skills_dir = setup_archive_test_dir("success");
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test").unwrap();

        let remover = SkillRemover::new(&skills_dir);
        let options = RemoveOptions::default();

        let result = remover.remove("test-skill", &options).unwrap();

        assert!(result.success);
        assert!(!skills_dir.join("test-skill").exists());
    }

    #[test]
    fn test_remove_with_data_dir() {
        let temp_dir = temp_dir("with_data");
        let skills_dir = temp_dir.join("skills");
        let data_dir = temp_dir.join("data");

        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let data_skill_dir = data_dir.join("test-skill");
        fs::create_dir_all(&data_skill_dir).unwrap();
        fs::write(data_skill_dir.join("data.txt"), "data").unwrap();

        let remover = SkillRemover::new(&skills_dir).with_data_dir(&data_dir);
        let options = RemoveOptions {
            remove_data: true,
            ..Default::default()
        };

        let _ = remover.remove("test-skill", &options).unwrap();

        assert!(!skills_dir.join("test-skill").exists());
        assert!(!data_dir.join("test-skill").exists());
    }

    #[test]
    fn test_remove_all() {
        let temp_dir = temp_dir("remove_all");
        let skills_dir = temp_dir.join("skills");

        fs::create_dir_all(&skills_dir).unwrap();

        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
        }

        let remover = SkillRemover::new(&skills_dir);
        let count = remover.remove_all(&RemoveOptions::default()).unwrap();

        assert_eq!(count, 3);
        assert!(skills_dir.exists());
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            assert!(!skill_dir.exists());
        }
    }

    #[test]
    fn test_dry_run() {
        let skills_dir = setup_archive_test_dir("dry_run");
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let remover = SkillRemover::new(&skills_dir);
        let options = RemoveOptions {
            dry_run: true,
            ..Default::default()
        };

        let result = remover.remove("test-skill", &options).unwrap();

        assert!(result.success);
        assert!(skill_dir.exists());
    }

    #[test]
    fn test_archive_skill_not_found() {
        let skills_dir = setup_archive_test_dir("archive_not_found");
        let remover = SkillRemover::new(&skills_dir);
        let options = ArchiveOptions::default();

        let result = remover.archive("nonexistent", &options).unwrap();

        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.absorption_timestamp.timestamp() > 0);
        assert!(result.absorbed_into.is_none());
    }

    #[test]
    fn test_archive_skill_success() {
        let skills_dir = setup_archive_test_dir("archive_success");
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test").unwrap();

        let remover = SkillRemover::new(&skills_dir);
        let options = ArchiveOptions::default();

        let result = remover.archive("test-skill", &options).unwrap();

        assert!(result.success);
        assert!(!skills_dir.join("test-skill").exists());
        assert!(skills_dir.join(".archive/test-skill").exists());
        assert!(result.absorption_timestamp.timestamp() > 0);
        assert!(result.absorbed_into.is_none());
    }

    #[test]
    fn test_archive_with_absorb_into() {
        let skills_dir = setup_archive_test_dir("archive_absorb");
        let source_dir = skills_dir.join("source-skill");
        fs::create_dir_all(&source_dir).unwrap();

        let remover = SkillRemover::new(&skills_dir);
        let options = ArchiveOptions {
            absorb_into: Some("target-skill".to_string()),
            ..Default::default()
        };

        let result = remover.archive("source-skill", &options).unwrap();

        assert!(result.success);
        assert!(result.absorbed_into.is_some());
        assert_eq!(result.absorbed_into, Some("target-skill".to_string()));
    }

    #[test]
    fn test_archive_dry_run() {
        let skills_dir = setup_archive_test_dir("archive_dry_run");
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let remover = SkillRemover::new(&skills_dir);
        let options = ArchiveOptions {
            dry_run: true,
            ..Default::default()
        };

        let result = remover.archive("test-skill", &options).unwrap();

        assert!(result.success);
        assert!(skill_dir.exists());
        assert!(result.absorbed_into.is_none());
    }

    #[test]
    fn test_archive_remove_data() {
        let temp_dir = temp_dir("archive_remove_data");
        let skills_dir = temp_dir.join("skills");
        let data_dir = temp_dir.join("data");

        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let data_skill_dir = data_dir.join("test-skill");
        fs::create_dir_all(&data_skill_dir).unwrap();
        fs::write(data_skill_dir.join("data.txt"), "data").unwrap();

        let remover = SkillRemover::new(&skills_dir).with_data_dir(&data_dir);
        let options = ArchiveOptions {
            remove_data: true,
            ..Default::default()
        };

        let _ = remover.archive("test-skill", &options).unwrap();

        assert!(!skills_dir.join("test-skill").exists());
        assert!(!data_dir.join("test-skill").exists());
    }
}
