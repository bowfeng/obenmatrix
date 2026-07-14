/// Skill Enable/Disable - Enable and disable skills
/// 
/// Maps to `hermes-agent/skills_hub.py` enable/disable functionality
use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Enable/disable options
#[derive(Debug, Clone)]
pub struct EnableDisableOptions {
    /// Include dependencies
    pub include_deps: bool,
    /// Dry run
    pub dry_run: bool,
}

impl Default for EnableDisableOptions {
    fn default() -> Self {
        Self {
            include_deps: false,
            dry_run: false,
        }
    }
}

/// State of a skill
#[derive(Debug, Clone, PartialEq)]
pub enum SkillState {
    /// Skill is enabled
    Enabled,
    /// Skill is disabled
    Disabled,
}

/// Skill state manager
pub struct SkillStateManager {
    skills_dir: PathBuf,
    state_dir: PathBuf,
}

impl SkillStateManager {
    /// Create a new skill state manager
    pub fn new(skills_dir: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            state_dir: state_dir.into(),
        }
    }
    
    /// Enable a skill
    pub fn enable(&self, skill_name: &str, options: &EnableDisableOptions) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        if options.dry_run {
            return Ok(true);
        }
        
        // Remove disabled marker if it exists
        let disabled_marker = self.state_dir.join(format!("{}.disabled", skill_name));
        if disabled_marker.exists() {
            fs::remove_file(&disabled_marker)?;
        }
        
        // Mark as enabled in state file
        self.update_state_file(skill_name, true)?;
        
        Ok(true)
    }
    
    /// Disable a skill
    pub fn disable(&self, skill_name: &str, options: &EnableDisableOptions) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        if options.dry_run {
            return Ok(true);
        }
        
        // Create disabled marker
        fs::create_dir_all(&self.state_dir)?;
        let disabled_marker = self.state_dir.join(format!("{}.disabled", skill_name));
        fs::write(&disabled_marker, "disabled")?;
        
        // Mark as disabled in state file
        self.update_state_file(skill_name, false)?;
        
        Ok(true)
    }
    
    /// Check if a skill is enabled
    pub fn is_enabled(&self, skill_name: &str) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        // Check for disabled marker
        let disabled_marker = self.state_dir.join(format!("{}.disabled", skill_name));
        if disabled_marker.exists() {
            return Ok(false);
        }
        
        // Check state file - skills are enabled by default unless explicitly disabled
        Ok(self.get_state(skill_name).unwrap_or(true))
    }
    
    /// Update the state file
    fn update_state_file(&self, skill_name: &str, enabled: bool) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        
        let _state_file = self.state_dir.join("skills_state.yaml");
        let mut state = self.load_state();
        
        if enabled {
            state.insert(skill_name.to_string());
        } else {
            state.remove(skill_name);
        }
        
        self.save_state(&state)?;
        
        Ok(())
    }
    
    /// Load state from file
    fn load_state(&self) -> BTreeSet<String> {
        let enabled = self.state_dir.join("skills_state.yaml");
        
        if !enabled.exists() {
            return BTreeSet::new();
        }
        
        let content = fs::read_to_string(&enabled).unwrap_or_default();
        
        // Parse YAML content (simplified)
        let mut set = BTreeSet::new();
        for line in content.lines() {
            if line.trim().starts_with("- ") {
                let skill = line.trim().strip_prefix("- ").unwrap_or_default();
                if !skill.is_empty() {
                    set.insert(skill.to_string());
                }
            }
        }
        
        set
    }
    
    /// Save state to file
    fn save_state(&self, state: &BTreeSet<String>) -> Result<()> {
        let state_file = self.state_dir.join("skills_state.yaml");
        fs::create_dir_all(self.state_dir.parent().unwrap_or(&self.state_dir))?;
        
        let content = state
            .iter()
            .map(|s| format!("- {}", s))
            .collect::<Vec<_>>()
            .join("\n");
        
        fs::write(&state_file, content)?;
        Ok(())
    }
    
    /// Get the state of a skill
    fn get_state(&self, skill_name: &str) -> Option<bool> {
        let state = self.load_state();
        state.contains(skill_name).then_some(true)
    }
    
    /// List all enabled skills
    pub fn list_enabled(&self) -> Result<Vec<String>> {
        let mut enabled = Vec::new();
        
        if !self.skills_dir.exists() {
            return Ok(enabled);
        }
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if self.is_enabled(name)? {
                        enabled.push(name.to_string());
                    }
                }
            }
        }
        
        Ok(enabled)
    }
    
    /// List all disabled skills
    pub fn list_disabled(&self) -> Result<Vec<String>> {
        let mut disabled = Vec::new();
        
        if !self.skills_dir.exists() {
            return Ok(disabled);
        }
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if !self.is_enabled(name)? {
                        disabled.push(name.to_string());
                    }
                }
            }
        }
        
        Ok(disabled)
    }
    
    /// Enable all skills
    pub fn enable_all(&self, options: &EnableDisableOptions) -> Result<usize> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }
        
        let mut count = 0;
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if self.enable(name, options)? {
                        count += 1;
                    }
                }
            }
        }
        
        Ok(count)
    }
    
    /// Disable all skills
    pub fn disable_all(&self, options: &EnableDisableOptions) -> Result<usize> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }
        
        let mut count = 0;
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if self.disable(name, options)? {
                        count += 1;
                    }
                }
            }
        }
        
        Ok(count)
    }
    
    /// Get the skills directory
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }
    
    /// Get the state directory
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }
    
    pub fn pin(&self, skill_name: &str) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        fs::create_dir_all(&self.state_dir)?;
        let pinned_marker = self.state_dir.join(format!("{}.pinned", skill_name));
        fs::write(&pinned_marker, skill_name)?;
        
        Ok(true)
    }
    
    pub fn unpin(&self, skill_name: &str) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        let pinned_marker = self.state_dir.join(format!("{}.pinned", skill_name));
        if !pinned_marker.exists() {
            return Ok(true);
        }
        
        fs::remove_file(&pinned_marker)?;
        
        Ok(true)
    }
    
    pub fn is_pinned(&self, skill_name: &str) -> Result<bool> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        let pinned_marker = self.state_dir.join(format!("{}.pinned", skill_name));
        Ok(pinned_marker.exists())
    }
    
    pub fn get_pinned_skills(&self) -> Result<Vec<String>> {
        if !self.state_dir.exists() {
            return Ok(Vec::new());
        }
        
        let mut pinned = Vec::new();
        
        for entry in fs::read_dir(&self.state_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            if let Some(name_str) = file_name.to_str() {
                if name_str.ends_with(".pinned") {
                    let skill_name = name_str.trim_end_matches(".pinned");
                    if self.skills_dir.join(skill_name).exists() {
                        pinned.push(skill_name.to_string());
                    }
                }
            }
        }
        
        Ok(pinned)
    }
}

impl Default for SkillStateManager {
    fn default() -> Self {
        Self::new(
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skills"))
                .unwrap_or_else(|| PathBuf::from("./skills")),
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skill_states"))
                .unwrap_or_else(|| PathBuf::from("./skill_states")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_state_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_enable_skill() {
        let temp_dir = temp_dir("enable");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        let options = EnableDisableOptions::default();
        
        // Skills are enabled by default
        assert!(manager.is_enabled("test-skill").unwrap());
        
        // Dry run enable just returns true without modifying state
        let result = manager.enable("test-skill", &options).unwrap();
        assert!(result);
        
        // Still enabled (default) since dry_run doesn't change state
        assert!(manager.is_enabled("test-skill").unwrap());
        
        // Actually enable (idempotent - already enabled)
        manager.enable("test-skill", &EnableDisableOptions { dry_run: false, ..Default::default() }).unwrap();
        assert!(manager.is_enabled("test-skill").unwrap());
    }

    #[test]
    fn test_disable_skill() {
        let temp_dir = temp_dir("disable");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        let options = EnableDisableOptions::default();
        
        manager.disable("test-skill", &options).unwrap();
        
        assert!(!manager.is_enabled("test-skill").unwrap());
    }

    #[test]
    fn test_dry_run() {
        let temp_dir = temp_dir("dry_run");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        // Dry run disable
        let result = manager.disable("test-skill", &EnableDisableOptions {
            dry_run: true,
            ..Default::default()
        }).unwrap();
        
        assert!(result);
        // File should not exist after dry run
        assert!(!state_dir.join("test-skill.disabled").exists());
    }

    #[test]
    fn test_list_enabled_disabled() {
        let temp_dir = temp_dir("list");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        // Create skills
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
        }
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        // Disable some skills
        for i in 0..2 {
            let _ = manager.disable(&format!("skill-{}", i), &EnableDisableOptions::default());
        }
        
        let enabled = manager.list_enabled().unwrap();
        let disabled = manager.list_disabled().unwrap();
        
        assert_eq!(enabled.len(), 1); // skill-2 is enabled
        assert_eq!(disabled.len(), 2); // skill-0 and skill-1 are disabled
    }

    #[test]
    fn test_pin_skill() {
        let temp_dir = temp_dir("pin");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        assert!(!manager.is_pinned("test-skill").unwrap());
        
        manager.pin("test-skill").unwrap();
        
        assert!(manager.is_pinned("test-skill").unwrap());
        
        let pinned_marker = state_dir.join("test-skill.pinned");
        assert!(pinned_marker.exists());
    }

    #[test]
    fn test_unpin_skill() {
        let temp_dir = temp_dir("unpin");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        manager.pin("test-skill").unwrap();
        assert!(manager.is_pinned("test-skill").unwrap());
        
        manager.unpin("test-skill").unwrap();
        
        assert!(!manager.is_pinned("test-skill").unwrap());
        
        let pinned_marker = state_dir.join("test-skill.pinned");
        assert!(!pinned_marker.exists());
    }

    #[test]
    fn test_is_pinned_nonexistent_skill() {
        let temp_dir = temp_dir("is_pinned_nonexistent");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        let result = manager.is_pinned("nonexistent-skill").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_get_pinned_skills() {
        let temp_dir = temp_dir("get_pinned");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        for i in 0..3 {
            let skill_dir = skills_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
        }
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        let mut pinned = manager.get_pinned_skills().unwrap();
        assert!(pinned.is_empty());
        
        manager.pin("skill-0").unwrap();
        manager.pin("skill-2").unwrap();
        
        pinned = manager.get_pinned_skills().unwrap();
        assert_eq!(pinned.len(), 2);
        assert!(pinned.contains(&"skill-0".to_string()));
        assert!(pinned.contains(&"skill-2".to_string()));
    }

    #[test]
    fn test_unpin_nonexistent_pinned() {
        let temp_dir = temp_dir("unpin_nonexistent");
        let skills_dir = temp_dir.join("skills");
        let state_dir = temp_dir.join("state");
        
        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let manager = SkillStateManager::new(&skills_dir, &state_dir);
        
        manager.unpin("test-skill").unwrap();
        
        let result = manager.unpin("test-skill").unwrap();
        assert!(result);
    }
}
