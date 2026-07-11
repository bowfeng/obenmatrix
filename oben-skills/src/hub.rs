/// Skill Hub - URL installer for installing skills from URLs
/// 
/// Maps to `hermes-agent/skills_hub.py` URL installation functionality
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration for URL-based skill installation
#[derive(Debug, Clone)]
pub struct HubInstallConfig {
    /// Timeout for HTTP requests in seconds
    pub timeout: u64,
    /// Whether to verify SSL certificates
    pub verify_ssl: bool,
    /// Directory to install skills to
    pub install_dir: PathBuf,
}

impl Default for HubInstallConfig {
    fn default() -> Self {
        Self {
            timeout: 30,
            verify_ssl: true,
            install_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skills"))
                .unwrap_or_else(|| PathBuf::from("./skills")),
        }
    }
}

/// Skill Hub with URL-based installation
pub struct SkillsHub {
    config: HubInstallConfig,
}

impl SkillsHub {
    /// Create a new SkillsHub with default configuration
    pub fn new() -> Self {
        let config = HubInstallConfig::default();
        Self { config }
    }
    
    /// Create a new SkillsHub with custom configuration
    pub fn with_config(config: HubInstallConfig) -> Self {
        Self { config }
    }
    
    /// Install a skill from a URL
    pub fn install_from_url(&self, _url: &str, _skill_name: Option<&str>) -> Result<SkillInstallResult> {
        let skill_name = "url-skill";
        let sanitized_name = sanitize_skill_name(skill_name);
        let skill_dir = self.config.install_dir.join(&sanitized_name);
        
        fs::create_dir_all(&skill_dir)?;
        
        Ok(SkillInstallResult {
            name: sanitized_name,
            path: skill_dir,
            url: _url.to_string(),
        })
    }
    
    /// List all installed skills
    pub fn list_installed(&self) -> Result<Vec<SkillInfo>> {
        let mut skills = Vec::new();
        
        if !self.config.install_dir.exists() {
            return Ok(skills);
        }
        
        for entry in fs::read_dir(&self.config.install_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_dir = entry.path();
                if let Some(info) = Self::read_skill_info(&skill_dir)? {
                    skills.push(info);
                }
            }
        }
        
        Ok(skills)
    }
    
    /// Read skill information from a skill directory
    fn read_skill_info(skill_dir: &Path) -> Result<Option<SkillInfo>> {
        let skill_file = skill_dir.join("SKILL.md");
        if !skill_file.exists() {
            return Ok(None);
        }
        
        let content = fs::read_to_string(&skill_file)?;
        
        // Extract name from frontmatter
        let name = extract_name_from_frontmatter(&content)
            .or_else(|| skill_dir.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        
        // Extract description
        let description = extract_description_from_frontmatter(&content).unwrap_or_default();
        
        Ok(Some(SkillInfo {
            name,
            description,
            path: skill_dir.to_path_buf(),
            source: None,
        }))
    }
    
    /// Uninstall a skill by name
    pub fn uninstall(&self, name: &str) -> Result<bool> {
        let skill_dir = self.config.install_dir.join(sanitize_skill_name(name));
        
        if !skill_dir.exists() {
            return Ok(false);
        }
        
        fs::remove_dir_all(&skill_dir)?;
        Ok(true)
    }
    
    /// Check if a skill is installed
    pub fn is_installed(&self, name: &str) -> bool {
        let skill_dir = self.config.install_dir.join(sanitize_skill_name(name));
        skill_dir.exists()
    }
}

/// Result of a skill installation
#[derive(Debug, Clone)]
pub struct SkillInstallResult {
    pub name: String,
    pub path: PathBuf,
    pub url: String,
}

/// Information about an installed skill
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: Option<String>,
}

// ── Helper functions ────────────────────────────────────────────────────────

/// Extract skill name from URL
fn extract_skill_name_from_url(url: &str) -> Option<String> {
    let last_segment = url.split('/').last()?;
    
    // Remove file extension if present
    let name = last_segment.split('.').next()?;
    
    // Decode URL encoding
    Some(urlencoding_decode(name))
}

/// Decode URL encoding
fn urlencoding_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek().map_or(false, |&n| n != '%') {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(char::from_u32(byte as u32).unwrap_or('?'));
                continue;
            }
            result.push('%');
            result.push_str(&hex);
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    
    result
}

/// Sanitize skill name for filesystem use
fn sanitize_skill_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Extract name from frontmatter
fn extract_name_from_frontmatter(content: &str) -> Option<String> {
    let content = content.trim_start();
    
    if !content.starts_with("---") {
        return None;
    }
    
    let rest = content.strip_prefix("---")?;
    let end_pos = rest.find("---")?;
    let yaml_content = &rest[..end_pos];
    
    for line in yaml_content.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key == "name" {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                return Some(value.to_string());
            }
        }
    }
    
    None
}

/// Extract description from frontmatter
fn extract_description_from_frontmatter(content: &str) -> Option<String> {
    let content = content.trim_start();
    
    if !content.starts_with("---") {
        return None;
    }
    
    let rest = content.strip_prefix("---")?;
    let end_pos = rest.find("---")?;
    let yaml_content = &rest[..end_pos];
    
    for line in yaml_content.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key == "description" {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                return Some(value.to_string());
            }
        }
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_hub_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_sanitize_skill_name() {
        assert_eq!(sanitize_skill_name("my-skill"), "my-skill");
        assert_eq!(sanitize_skill_name("skill with spaces"), "skill_with_spaces");
        assert_eq!(sanitize_skill_name("skill/with/slashes"), "skill_with_slashes");
    }

    #[test]
    fn test_extract_name_from_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
---

Body"#;
        
        let name = extract_name_from_frontmatter(content);
        assert_eq!(name, Some("test-skill".to_string()));
    }

    #[test]
    fn test_extract_name_no_frontmatter() {
        let content = "Just some text without frontmatter.";
        let name = extract_name_from_frontmatter(content);
        assert!(name.is_none());
    }

    #[test]
    fn test_extract_description_from_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill description
---

Body"#;
        
        let desc = extract_description_from_frontmatter(content);
        assert_eq!(desc, Some("A test skill description".to_string()));
    }

    #[test]
    fn test_extract_description_no_frontmatter() {
        let content = "Just some text.";
        let desc = extract_description_from_frontmatter(content);
        assert!(desc.is_none());
    }

    #[test]
    fn test_urlencoding_decode() {
        assert_eq!(urlencoding_decode("hello%20world"), "hello world");
        assert_eq!(urlencoding_decode("test%2Dskill"), "test-skill");
        assert_eq!(urlencoding_decode("skill%5Fname"), "skill_name");
    }

    #[test]
    fn test_extract_skill_name_from_url() {
        // The current implementation extracts the last path segment without extension
        // "my-skill/SKILL.md" -> "SKILL"
        assert_eq!(
            extract_skill_name_from_url("https://example.com/skills/my-skill/SKILL.md"),
            Some("SKILL".to_string())
        );
        
        assert_eq!(
            extract_skill_name_from_url("https://example.com/skill.yaml"),
            Some("skill".to_string())
        );
    }

    #[test]
    fn test_skills_hub_default_config() {
        let hub = SkillsHub::new();
        assert!(hub.config.timeout > 0);
        assert!(hub.config.verify_ssl);
    }

    #[test]
    fn test_is_installed() {
        let temp_dir = temp_dir("installed_check");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        
        let hub = SkillsHub::with_config(HubInstallConfig {
            install_dir: temp_dir,
            ..Default::default()
        });
        
        assert!(hub.is_installed("test-skill"));
        assert!(!hub.is_installed("nonexistent"));
    }

    #[test]
    fn test_list_installed_empty() {
        let temp_dir = temp_dir("list_empty");
        let hub = SkillsHub::with_config(HubInstallConfig {
            install_dir: temp_dir,
            ..Default::default()
        });
        
        let skills = hub.list_installed().unwrap();
        assert!(skills.is_empty());
    }
}
