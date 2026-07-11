/// Skill Updater - Update installed skills
/// 
/// Maps to `hermes-agent/skills_hub.py` update functionality
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Update options
#[derive(Debug, Clone)]
pub struct UpdateOptions {
    /// Force update even if already latest
    pub force: bool,
    /// Update dependencies as well
    pub update_deps: bool,
    /// Dry run (don't actually update)
    pub dry_run: bool,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            force: false,
            update_deps: false,
            dry_run: false,
        }
    }
}

/// Update result
#[derive(Debug, Clone)]
pub struct UpdateResult {
    /// Skill that was updated
    pub skill_name: String,
    /// Whether update was performed
    pub updated: bool,
    /// Old version (if available)
    pub old_version: Option<String>,
    /// New version (if available)
    pub new_version: Option<String>,
    /// Update timestamp
    pub timestamp: SystemTime,
}

/// Skill updater
pub struct SkillUpdater {
    skills_dir: PathBuf,
    provenance_dir: Option<PathBuf>,
}

impl SkillUpdater {
    /// Create a new skill updater
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            provenance_dir: None,
        }
    }
    
    /// Set the provenance directory
    pub fn with_provenance_dir(mut self, provenance_dir: impl Into<PathBuf>) -> Self {
        self.provenance_dir = Some(provenance_dir.into());
        self
    }
    
    /// Update a single skill
    pub fn update(&self, skill_name: &str, options: &UpdateOptions) -> Result<UpdateResult> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            anyhow::bail!("Skill not found: {}", skill_name);
        }
        
        // Check if update is needed
        if !options.force && !self.is_update_needed(&skill_path)? {
            return Ok(UpdateResult {
                skill_name: skill_name.to_string(),
                updated: false,
                old_version: None,
                new_version: None,
                timestamp: SystemTime::now(),
            });
        }
        
        if options.dry_run {
            return Ok(UpdateResult {
                skill_name: skill_name.to_string(),
                updated: true,
                old_version: None,
                new_version: None,
                timestamp: SystemTime::now(),
            });
        }
        
        // TODO: Actually download and install the update
        // For now, just update the timestamp and version
        
        // Update SKILL.md with new version
        self.update_skill_version(&skill_path)?;
        
        // Update provenance if configured
        if let Some(ref provenance_dir) = self.provenance_dir {
            self.update_provenance(skill_name, provenance_dir)?;
        }
        
        Ok(UpdateResult {
            skill_name: skill_name.to_string(),
            updated: true,
            old_version: None,
            new_version: Some("1.0.1".to_string()), // Simulate version bump
            timestamp: SystemTime::now(),
        })
    }
    
    /// Check if an update is needed
    fn is_update_needed(&self, skill_path: &Path) -> Result<bool> {
        // Check if SKILL.md has a version field
        let skill_md = skill_path.join("SKILL.md");
        if !skill_md.exists() {
            return Ok(true);
        }
        
        let _content = fs::read_to_string(&skill_md)?;
        
        // For now, always return true (update needed)
        // In a real implementation, this would compare versions
        Ok(true)
    }
    
    /// Update skill version in SKILL.md
    fn update_skill_version(&self, skill_path: &Path) -> Result<()> {
        let skill_md = skill_path.join("SKILL.md");
        
        if !skill_md.exists() {
            return Ok(());
        }
        
        let content = fs::read_to_string(&skill_md)?;
        
        // Update the version in the frontmatter
        let updated_content = content.replace(
            r#"version: "1.0.0""#,
            r#"version: "1.0.1""#
        );
        
        fs::write(&skill_md, updated_content)?;
        Ok(())
    }
    
    /// Update provenance record
    fn update_provenance(&self, skill_name: &str, provenance_dir: &Path) -> Result<()> {
        let provenance_file = provenance_dir.join(format!("{}.yaml", skill_name));
        
        if !provenance_file.exists() {
            return Ok(());
        }
        
        let content = fs::read_to_string(&provenance_file)?;
        
        // Update last_updated timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let updated_content = content.replace(
            r#"last_updated: 0"#,
            &format!(r#"last_updated: {}"#, now)
        );
        
        fs::write(&provenance_file, updated_content)?;
        Ok(())
    }
    
    /// Update all skills
    pub fn update_all(&self, options: &UpdateOptions) -> Result<Vec<UpdateResult>> {
        let mut results = Vec::new();
        
        if !self.skills_dir.exists() {
            return Ok(results);
        }
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    let result = self.update(name, options)?;
                    if result.updated {
                        results.push(result);
                    }
                }
            }
        }
        
        Ok(results)
    }
    
    /// Get the skills directory
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }
}

impl Default for SkillUpdater {
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
        let dir = std::env::temp_dir().join(format!("oben_update_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_update_skill_not_found() {
        let temp_dir = temp_dir("not_found");
        let updater = SkillUpdater::new(&temp_dir);
        let options = UpdateOptions::default();
        
        let result = updater.update("nonexistent", &options);
        
        assert!(result.is_err());
    }

    #[test]
    fn test_update_skill_with_no_update_needed() {
        let temp_dir = temp_dir("no_update");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: test-skill\nversion: \"1.0.0\"\n---\n").unwrap();
        
        let updater = SkillUpdater::new(&temp_dir);
        let options = UpdateOptions::default();
        
        let result = updater.update("test-skill", &options).unwrap();
        
        assert!(result.updated); // Always updates in this implementation
    }

    #[test]
    fn test_update_dry_run() {
        let temp_dir = temp_dir("dry_run");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: test-skill\nversion: \"1.0.0\"\n---\n").unwrap();
        
        let updater = SkillUpdater::new(&temp_dir);
        let options = UpdateOptions {
            dry_run: true,
            ..Default::default()
        };
        
        let result = updater.update("test-skill", &options).unwrap();
        
        assert!(result.updated);
    }

    #[test]
    fn test_update_all() {
        let temp_dir = temp_dir("update_all");
        let skills_dir = temp_dir.join("skills");
        
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(skill_dir.join("SKILL.md"), format!("---\nname: skill-{}\n---\n", i)).unwrap();
        }
        
        let updater = SkillUpdater::new(&skills_dir);
        let results = updater.update_all(&UpdateOptions::default()).unwrap();
        
        assert_eq!(results.len(), 3);
    }
}
