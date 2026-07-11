/// Skill Lister - List installed skills
/// 
/// Maps to `hermes-agent/skills_hub.py` list functionality
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Skill info
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// Skill name
    pub name: String,
    /// Skill description
    pub description: String,
    /// Path to skill
    pub path: PathBuf,
    /// Source type (local, github, url, bundle)
    pub source: Option<String>,
    /// Version
    pub version: Option<String>,
    /// Last updated timestamp
    pub last_updated: Option<u64>,
}

/// Lister options
#[derive(Debug, Clone)]
pub struct ListerOptions {
    /// Only show enabled skills
    pub enabled_only: bool,
    /// Sort by name
    pub sort_by_name: bool,
}

impl Default for ListerOptions {
    fn default() -> Self {
        Self {
            enabled_only: false,
            sort_by_name: true,

        }
    }
}

/// Skill lister
pub struct SkillLister {
    skills_dir: PathBuf,
}

impl SkillLister {
    /// Create a new skill lister
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
        }
    }
    
    /// List all skills
    pub fn list_all(&self) -> Result<Vec<SkillInfo>> {
        let mut skills = Vec::new();
        
        if !self.skills_dir.exists() {
            return Ok(skills);
        }
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_dir = entry.path();
                
                // Skip hidden directories
                if !self.should_include_directory(&skill_dir) {
                    continue;
                }
                
                if let Some(info) = self.read_skill_info(&skill_dir)? {
                    skills.push(info);
                }
            }
        }
        
        // Sort by name if requested
        if self.should_sort() {
            skills.sort_by(|a, b| a.name.cmp(&b.name));
        }
        
        Ok(skills)
    }
    
    /// Read skill information from a skill directory
    fn read_skill_info(&self, skill_dir: &Path) -> Result<Option<SkillInfo>> {
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            return Ok(None);
        }
        
        let content = fs::read_to_string(&skill_md)?;
        
        // Extract info from frontmatter
        let name = self.extract_name(&content)
            .or_else(|| skill_dir.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        
        let description = self.extract_description(&content).unwrap_or_default();
        
        Ok(Some(SkillInfo {
            name,
            description,
            path: skill_dir.to_path_buf(),
            source: self.extract_source(&content),
            version: self.extract_version(&content),
            last_updated: None, // Would need to read file metadata
        }))
    }
    
    /// Extract name from SKILL.md content
    fn extract_name(&self, content: &str) -> Option<String> {
        if !content.starts_with("---") {
            return None;
        }
        
        let rest = content.strip_prefix("---")?;
        let end_pos = rest.find("---")?;
        let yaml_content = &rest[..end_pos];
        
        for line in yaml_content.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "name" {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
        
        None
    }
    
    /// Extract description from SKILL.md content
    fn extract_description(&self, content: &str) -> Option<String> {
        if !content.starts_with("---") {
            return None;
        }
        
        let rest = content.strip_prefix("---")?;
        let end_pos = rest.find("---")?;
        let yaml_content = &rest[..end_pos];
        
        for line in yaml_content.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "description" {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
        
        None
    }
    
    /// Extract source from SKILL.md content
    fn extract_source(&self, content: &str) -> Option<String> {
        if !content.starts_with("---") {
            return None;
        }
        
        let rest = content.strip_prefix("---")?;
        let end_pos = rest.find("---")?;
        let yaml_content = &rest[..end_pos];
        
        for line in yaml_content.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "source" {
                    return Some(value.trim().trim_matches('"').trim_matches('\'').to_string());
                }
            }
        }
        
        None
    }
    
    /// Extract version from SKILL.md content
    fn extract_version(&self, content: &str) -> Option<String> {
        if !content.starts_with("---") {
            return None;
        }
        
        let rest = content.strip_prefix("---")?;
        let end_pos = rest.find("---")?;
        let yaml_content = &rest[..end_pos];
        
        for line in yaml_content.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "version" {
                    return Some(value.trim().trim_matches('"').trim_matches('\'').to_string());
                }
            }
        }
        
        None
    }
    
    /// Check if a directory should be included
    fn should_include_directory(&self, dir: &Path) -> bool {
        // Skip hidden directories (starting with .)
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                return false;
            }
        }
        
        true
    }
    
    /// Should we sort the results?
    fn should_sort(&self) -> bool {
        true // Always sort by name
    }
    
    /// Count total skills
    pub fn count(&self) -> Result<usize> {
        Ok(self.list_all()?.len())
    }
    
    /// Check if a skill exists
    pub fn exists(&self, name: &str) -> Result<bool> {
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(skill_name) = entry.file_name().to_str() {
                    if skill_name == name {
                        let skill_md = entry.path().join("SKILL.md");
                        if skill_md.exists() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        
        Ok(false)
    }
    
    /// Get skills by source type
    pub fn by_source(&self, source: &str) -> Result<Vec<SkillInfo>> {
        let all = self.list_all()?;
        Ok(all.into_iter().filter(|s| {
            s.source.as_ref().map(|src| src == source).unwrap_or(false)
        }).collect())
    }
    
    /// Get skills by name pattern
    pub fn by_name_pattern(&self, pattern: &str) -> Result<Vec<SkillInfo>> {
        let all = self.list_all()?;
        let regex = regex::Regex::new(pattern)
            .map_err(|e| anyhow::anyhow!("Invalid regex pattern: {}", e))?;
        
        Ok(all.into_iter().filter(|s| regex.is_match(&s.name)).collect())
    }
}

impl Default for SkillLister {
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
        let dir = std::env::temp_dir().join(format!("oben_lister_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_list_all_empty() {
        let temp_dir = temp_dir("list_empty");
        let lister = SkillLister::new(&temp_dir);
        
        let skills = lister.list_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_list_all_with_skills() {
        let temp_dir = temp_dir("list_with");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: test-skill\ndescription: A test skill\n---\n").unwrap();
        
        let lister = SkillLister::new(&temp_dir);
        let skills = lister.list_all().unwrap();
        
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test-skill");
        assert_eq!(skills[0].description, "A test skill");
    }

    #[test]
    fn test_count() {
        let temp_dir = temp_dir("count");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: test-skill\n---\n").unwrap();
        
        let lister = SkillLister::new(&temp_dir);
        let count = lister.count().unwrap();
        
        assert_eq!(count, 1);
    }

    #[test]
    fn test_exists() {
        let temp_dir = temp_dir("exists");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: test-skill\n---\n").unwrap();
        
        let lister = SkillLister::new(&temp_dir);
        
        assert!(lister.exists("test-skill").unwrap());
        assert!(!lister.exists("nonexistent").unwrap());
    }

    #[test]
    fn test_sorting() {
        let temp_dir = temp_dir("sort");
        
        // Create skills in reverse alphabetical order
        for name in ["zebra", "alpha", "beta"].iter() {
            let skill_dir = temp_dir.join(name);
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(skill_dir.join("SKILL.md"), format!("---\nname: {}\n---\n", name)).unwrap();
        }
        
        let lister = SkillLister::new(&temp_dir);
        let skills = lister.list_all().unwrap();
        
        // Should be sorted alphabetically
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "zebra"]);
    }

    #[test]
    fn test_by_source() {
        let temp_dir = temp_dir("by_source");
        
        // Create skills with different sources
        let skill_dir = temp_dir.join("github-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: github-skill\nsource: github\n---\n").unwrap();
        
        let local_dir = temp_dir.join("local-skill");
        fs::create_dir_all(&local_dir).unwrap();
        fs::write(local_dir.join("SKILL.md"), "---\nname: local-skill\nsource: local\n---\n").unwrap();
        
        let lister = SkillLister::new(&temp_dir);
        
        let github_skills = lister.by_source("github").unwrap();
        assert_eq!(github_skills.len(), 1);
        assert_eq!(github_skills[0].name, "github-skill");
    }
}
