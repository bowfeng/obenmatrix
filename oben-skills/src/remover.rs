/// Skill Remover - Remove installed skills
/// 
/// Maps to `hermes-agent/skills_hub.py` uninstall functionality
use anyhow::Result;
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

    #[test]
    fn test_remove_skill_not_found() {
        let temp_dir = temp_dir("not_found");
        let remover = SkillRemover::new(&temp_dir);
        let options = RemoveOptions::default();
        
        let result = remover.remove("nonexistent", &options).unwrap();
        
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_remove_skill_success() {
        let temp_dir = temp_dir("success");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test").unwrap();
        
        let remover = SkillRemover::new(&temp_dir);
        let options = RemoveOptions::default();
        
        let result = remover.remove("test-skill", &options).unwrap();
        
        assert!(result.success);
        assert!(!temp_dir.join("test-skill").exists());
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
        
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
        }
        
        let remover = SkillRemover::new(&skills_dir);
        let count = remover.remove_all(&RemoveOptions::default()).unwrap();
        
        assert_eq!(count, 3);
        // The directory should still exist, just empty
        assert!(skills_dir.exists());
        // Verify all skill directories were removed
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            assert!(!skill_dir.exists());
        }
    }

    #[test]
    fn test_dry_run() {
        let temp_dir = temp_dir("dry_run");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let remover = SkillRemover::new(&temp_dir);
        let options = RemoveOptions {
            dry_run: true,
            ..Default::default()
        };
        
        let result = remover.remove("test-skill", &options).unwrap();
        
        assert!(result.success);
        assert!(skill_dir.exists()); // Should still exist after dry run
    }
}
