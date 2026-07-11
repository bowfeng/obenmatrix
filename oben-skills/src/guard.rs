/// Skills Guard - Validation and safety checks for skills
/// 
/// Maps to `hermes-agent/skills_guard.py` functionality
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Validation error types
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// Path traversal attempt detected
    PathTraversal,
    /// Invalid skill name format
    InvalidName(String),
    /// Malicious content detected
    MaliciousContent(String),
    /// Missing required field
    MissingField(String),
    /// Size exceeds limit
    TooLarge(u64, u64),
}

/// Safety check results
#[derive(Debug, Clone)]
pub struct SafetyCheck {
    /// Whether the check passed
    pub passed: bool,
    /// Error if check failed
    pub error: Option<ValidationError>,
    /// Details about the check
    pub details: String,
}

/// Skills guard configuration
#[derive(Debug, Clone)]
pub struct GuardConfig {
    /// Maximum skill file size in bytes
    pub max_size: u64,
    /// Maximum path depth
    pub max_depth: usize,
    /// Blocked patterns in skill names
    pub blocked_name_patterns: Vec<String>,
    /// Blocked content patterns
    pub blocked_content_patterns: Vec<String>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            max_size: 1024 * 1024, // 1MB
            max_depth: 10,
            blocked_name_patterns: vec![
                "..".to_string(),
                "/".to_string(),
                "\\".to_string(),
            ],
            blocked_content_patterns: vec![
                "rm -rf".to_string(),
                "format".to_string(),
                "sudo".to_string(),
                "eval(".to_string(),
            ],
        }
    }
}

/// Skills Guard manager
pub struct SkillsGuard {
    config: GuardConfig,
}

impl SkillsGuard {
    /// Create a new SkillsGuard
    pub fn new(config: GuardConfig) -> Self {
        Self { config }
    }
    
    /// Validate a skill name
    pub fn validate_name(&self, name: &str) -> SafetyCheck {
        // Check for empty name
        if name.is_empty() {
            return SafetyCheck {
                passed: false,
                error: Some(ValidationError::InvalidName("name is empty".to_string())),
                details: "Skill name cannot be empty".to_string(),
            };
        }
        
        // Check for path traversal
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return SafetyCheck {
                passed: false,
                error: Some(ValidationError::PathTraversal),
                details: "Path traversal detected in skill name".to_string(),
            };
        }
        
        // Check for blocked patterns
        for pattern in &self.config.blocked_name_patterns {
            if name.contains(pattern) {
                return SafetyCheck {
                    passed: false,
                    error: Some(ValidationError::InvalidName(name.to_string())),
                    details: format!("Skill name contains blocked pattern: {}", pattern),
                };
            }
        }
        
        // Check name format (alphanumeric, underscore, hyphen only)
        let valid_name: HashSet<char> = ('a'..='z')
            .chain('0'..='9')
            .chain(['_', '-', '.'])
            .chain('A'..='Z')
            .collect();
        
        if !name.chars().all(|c| valid_name.contains(&c)) {
            return SafetyCheck {
                passed: false,
                error: Some(ValidationError::InvalidName(name.to_string())),
                details: "Skill name contains invalid characters".to_string(),
            };
        }
        
        SafetyCheck {
            passed: true,
            error: None,
            details: format!("Name '{}' is valid", name),
        }
    }
    
    /// Check file size
    pub fn check_size(&self, path: &Path) -> SafetyCheck {
        let metadata = match fs_extra::dir::get_size(path) {
            Ok(size) => size,
            Err(e) => {
                return SafetyCheck {
                    passed: false,
                    error: Some(ValidationError::TooLarge(0, self.config.max_size)),
                    details: format!("Failed to get size: {}", e),
                };
            }
        };
        
        let size = metadata as u64;
        
        if size > self.config.max_size {
            return SafetyCheck {
                passed: false,
                error: Some(ValidationError::TooLarge(size, self.config.max_size)),
                details: format!("Skill size {} exceeds maximum {}", size, self.config.max_size),
            };
        }
        
        SafetyCheck {
            passed: true,
            error: None,
            details: format!("Skill size {} is within limit", size),
        }
    }
    
    /// Scan content for malicious patterns
    pub fn scan_content(&self, content: &str, source: &str) -> SafetyCheck {
        for pattern in &self.config.blocked_content_patterns {
            if content.contains(pattern) {
                return SafetyCheck {
                    passed: false,
                    error: Some(ValidationError::MaliciousContent(pattern.to_string())),
                    details: format!(
                        "Malicious pattern '{}' detected in {}",
                        pattern, source
                    ),
                };
            }
        }
        
        SafetyCheck {
            passed: true,
            error: None,
            details: format!("Content from {} passed safety check", source),
        }
    }
    
    /// Validate a skill directory
    pub fn validate_skill_dir(&self, dir: &Path) -> Result<SafetyCheck> {
        // Check directory exists
        if !dir.exists() {
            return Ok(SafetyCheck {
                passed: false,
                error: Some(ValidationError::MissingField("directory".to_string())),
                details: "Skill directory does not exist".to_string(),
            });
        }
        
        // Check directory name
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        
        let name_check = self.validate_name(name);
        if !name_check.passed {
            return Ok(name_check);
        }
        
        // Check size
        let size_check = self.check_size(dir);
        if !size_check.passed {
            return Ok(size_check);
        }
        
        // Check for SKILL.md
        let skill_md = dir.join("SKILL.md");
        if skill_md.exists() {
            let content = std::fs::read_to_string(&skill_md)?;
            let content_check = self.scan_content(&content, "SKILL.md");
            if !content_check.passed {
                return Ok(content_check);
            }
        }
        
        Ok(SafetyCheck {
            passed: true,
            error: None,
            details: format!("Skill directory '{}' is valid", dir.display()),
        })
    }
    
    /// Validate a skill file
    pub fn validate_skill_file(&self, file: &Path) -> Result<SafetyCheck> {
        if !file.exists() {
            return Ok(SafetyCheck {
                passed: false,
                error: Some(ValidationError::MissingField("file".to_string())),
                details: "Skill file does not exist".to_string(),
            });
        }
        
        let content = std::fs::read_to_string(file)?;
        let check = self.scan_content(&content, &file.display().to_string());
        
        Ok(check)
    }
}

impl Default for SkillsGuard {
    fn default() -> Self {
        Self::new(GuardConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_guard_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_validate_name_valid() {
        let guard = SkillsGuard::default();
        
        let result = guard.validate_name("my-skill");
        assert!(result.passed);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_validate_name_path_traversal() {
        let guard = SkillsGuard::default();
        
        let result = guard.validate_name("../etc/passwd");
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::PathTraversal)));
    }

    #[test]
    fn test_validate_name_empty() {
        let guard = SkillsGuard::default();
        
        let result = guard.validate_name("");
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::InvalidName(_))));
    }

    #[test]
    fn test_validate_name_invalid_chars() {
        let guard = SkillsGuard::default();
        
        let result = guard.validate_name("my skill!");
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::InvalidName(_))));
    }

    #[test]
    fn test_check_size_within_limit() {
        let guard = SkillsGuard::default();
        let temp_dir = temp_dir("size_within");
        let test_file = temp_dir.join("test.txt");
        fs::write(&test_file, "small content").unwrap();
        
        let result = guard.check_size(&temp_dir);
        assert!(result.passed);
    }

    #[test]
    fn test_check_size_exceeds_limit() {
        let mut config = GuardConfig::default();
        config.max_size = 10; // 10 bytes
        
        let guard = SkillsGuard::new(config);
        let temp_dir = temp_dir("size_exceeds");
        let test_file = temp_dir.join("test.txt");
        fs::write(&test_file, "this is larger than 10 bytes").unwrap();
        
        let result = guard.check_size(&temp_dir);
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::TooLarge(_, _))));
    }

    #[test]
    fn test_scan_content_safe() {
        let guard = SkillsGuard::default();
        let content = "# Safe skill\nThis is a safe skill.";
        
        let result = guard.scan_content(content, "test-skill");
        assert!(result.passed);
    }

    #[test]
    fn test_scan_content_malicious() {
        let guard = SkillsGuard::default();
        
        let result = guard.scan_content("eval(rm -rf /)", "test");
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::MaliciousContent(_))));
    }

    #[test]
    fn test_validate_skill_dir_valid() {
        let guard = SkillsGuard::default();
        let temp_dir = temp_dir("dir_valid");
        let skill_dir = temp_dir.join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Test Skill").unwrap();
        
        let result = guard.validate_skill_dir(&skill_dir).unwrap();
        assert!(result.passed);
    }

    #[test]
    fn test_validate_skill_dir_missing() {
        let guard = SkillsGuard::default();
        let temp_dir = temp_dir("dir_missing");
        let skill_dir = temp_dir.join("nonexistent");
        
        let result = guard.validate_skill_dir(&skill_dir).unwrap();
        assert!(!result.passed);
        assert!(matches!(result.error, Some(ValidationError::MissingField(_))));
    }
}
