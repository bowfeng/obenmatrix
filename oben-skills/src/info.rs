/// Skill Info - Show skill details
/// 
/// Maps to `hermes-agent/skills_hub.py` info functionality
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Skill information display
#[derive(Debug, Clone)]
pub struct SkillInfoDisplay {
    /// Skill name
    pub name: String,
    /// Skill description
    pub description: String,
    /// Skill version
    pub version: String,
    /// Source URL
    pub source_url: String,
    /// Installation date
    pub installed_at: String,
    /// Last updated
    pub last_updated: String,
    /// Tags
    pub tags: Vec<String>,
    /// Author
    pub author: String,
    /// Dependencies
    pub dependencies: Vec<String>,
    /// Category
    pub category: String,
}

/// Skill info provider
pub struct SkillInfoProvider {
    skills_dir: PathBuf,
}

impl SkillInfoProvider {
    /// Create a new skill info provider
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
        }
    }
    
    /// Get info for a specific skill
    pub fn get_skill_info(&self, skill_name: &str) -> Result<Option<SkillInfoDisplay>> {
        let skill_path = self.skills_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(None);
        }
        
        let skill_md = skill_path.join("SKILL.md");
        if !skill_md.exists() {
            return Ok(None);
        }
        
        let content = fs::read_to_string(&skill_md)?;
        
        // Parse frontmatter
        let (name, description, version, source_url, installed_at, last_updated, tags, author, dependencies, category) = 
            self.parse_skill_content(&content, &skill_path)?;
        
        Ok(Some(SkillInfoDisplay {
            name,
            description,
            version,
            source_url,
            installed_at,
            last_updated,
            tags,
            author,
            dependencies,
            category,
        }))
    }
    
    /// Parse skill content from SKILL.md
    fn parse_skill_content(
        &self,
        content: &str,
        skill_path: &Path,
    ) -> Result<(String, String, String, String, String, String, Vec<String>, String, Vec<String>, String)> {
        let mut name = String::new();
        let mut description = String::new();
        let mut version = String::new();
        let mut source_url = String::new();
        let mut installed_at = String::new();
        let mut last_updated = String::new();
        let mut tags = Vec::new();
        let mut author = String::new();
        let mut dependencies = Vec::new();
        let mut category = String::new();
        
        // Try to parse YAML frontmatter
        if content.starts_with("---") {
            let rest = content.strip_prefix("---").unwrap_or_default();
            let end_pos = rest.find("---").unwrap_or(rest.len());
            let yaml_content = &rest[..end_pos];
            
            for line in yaml_content.lines() {
                let trimmed = line.trim();
                
                if trimmed.starts_with("name:") {
                    name = trimmed
                        .strip_prefix("name:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                } else if trimmed.starts_with("description:") {
                    description = trimmed
                        .strip_prefix("description:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                } else if trimmed.starts_with("version:") {
                    version = trimmed
                        .strip_prefix("version:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                } else if trimmed.starts_with("source:") {
                    source_url = trimmed
                        .strip_prefix("source:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                } else if trimmed.starts_with("tags:") {
                    // Parse tag list
                    let tags_str = trimmed.strip_prefix("tags:").unwrap_or_default().trim();
                    if tags_str.starts_with('[') && tags_str.ends_with(']') {
                        let inner = &tags_str[1..tags_str.len()-1];
                        tags = inner.split(',').map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string()).collect();
                    }
                } else if trimmed.starts_with("author:") {
                    author = trimmed
                        .strip_prefix("author:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                } else if trimmed.starts_with("dependencies:") {
                    // Parse dependency list
                    let deps_str = trimmed.strip_prefix("dependencies:").unwrap_or_default().trim();
                    if deps_str.starts_with('[') && deps_str.ends_with(']') {
                        let inner = &deps_str[1..deps_str.len()-1];
                        dependencies = inner.split(',').map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string()).collect();
                    }
                } else if trimmed.starts_with("category:") {
                    category = trimmed
                        .strip_prefix("category:")
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .unwrap_or_default()
                        .to_string();
                }
            }
        }
        
        // Set defaults if not in frontmatter
        if name.is_empty() {
            name = skill_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        
        // Set timestamps based on file metadata
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        if installed_at.is_empty() {
            installed_at = now.to_string();
        }
        
        if last_updated.is_empty() {
            last_updated = now.to_string();
        }
        
        Ok((
            name,
            description,
            version,
            source_url,
            installed_at,
            last_updated,
            tags,
            author,
            dependencies,
            category,
        ))
    }
    
    /// List all skills with basic info
    pub fn list_all(&self) -> Result<Vec<(String, String)>> {
        let mut skills = Vec::new();
        
        if !self.skills_dir.exists() {
            return Ok(skills);
        }
        
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_path = entry.path();
                let skill_md = skill_path.join("SKILL.md");
                
                if skill_md.exists() {
                    if let Some(name) = skill_path.file_name().and_then(|n| n.to_str()) {
                        // Get description from frontmatter
                        let content = fs::read_to_string(&skill_md)?;
                        let description = self.parse_skill_content(&content, &skill_path)
                            .map(|(_, desc, _, _, _, _, _, _, _, _)| desc)
                            .unwrap_or_default();
                        
                        skills.push((name.to_string(), description));
                    }
                }
            }
        }
        
        Ok(skills)
    }
    
    /// Display skill info in a formatted way
    pub fn display_skill_info(&self, skill_name: &str) -> Result<String> {
        let info = self.get_skill_info(skill_name)?
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;
        
        let output = format!(
            r#"
=== {} ===

Description: {}

Version: {}
Source: {}

Installed at: {}
Last updated: {}

Author: {}
Category: {}

Tags: {}
Dependencies: {}
"#,
            info.name,
            info.description,
            info.version,
            info.source_url,
            info.installed_at,
            info.last_updated,
            info.author,
            info.category,
            if info.tags.is_empty() {
                "None".to_string()
            } else {
                info.tags.join(", ")
            },
            if info.dependencies.is_empty() {
                "None".to_string()
            } else {
                info.dependencies.join(", ")
            }
        );
        
        Ok(output)
    }
}

impl Default for SkillInfoProvider {
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
        let dir = std::env::temp_dir().join(format!("oben_info_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_get_skill_info_not_found() {
        let temp_dir = temp_dir("not_found");
        let provider = SkillInfoProvider::new(&temp_dir);
        
        let result = provider.get_skill_info("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_skill_info_valid() {
        let temp_dir = temp_dir("valid");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let skill_md = format!(r#"---
name: test-skill
description: A test skill
version: "1.0.0"
source: local
tags: [test, example]
author: test-author
category: utilities
---

Skill body"#);
        
        fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();
        
        let provider = SkillInfoProvider::new(&temp_dir);
        let info = provider.get_skill_info("test-skill").unwrap();
        
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.name, "test-skill");
        assert_eq!(info.description, "A test skill");
        assert_eq!(info.version, "1.0.0");
    }

    #[test]
    fn test_display_skill_info() {
        let temp_dir = temp_dir("display");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let skill_md = format!(r#"---
name: test-skill
description: A test skill
version: "1.0.0"
source: local
author: test-author
category: utilities
---

Skill body"#);
        
        fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();
        
        let provider = SkillInfoProvider::new(&temp_dir);
        let output = provider.display_skill_info("test-skill").unwrap();
        
        assert!(output.contains("test-skill"));
        assert!(output.contains("A test skill"));
    }

    #[test]
    fn test_list_all() {
        let temp_dir = temp_dir("list");
        
        for i in 0..3 {
            let skill_dir = temp_dir.join(format!("skill-{}", i));
            fs::create_dir_all(&skill_dir).unwrap();
            let skill_md = format!(r#"---
name: skill-{}
description: Skill {}
---"#, i, i);
            fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();
        }
        
        let provider = SkillInfoProvider::new(&temp_dir);
        let skills = provider.list_all().unwrap();
        
        assert_eq!(skills.len(), 3);
    }
}
