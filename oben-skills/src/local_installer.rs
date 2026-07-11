/// Skill Local Installer - Install skills from local paths
/// 
/// Maps to `hermes-agent/skills_hub.py` local installation functionality
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Local install configuration
#[derive(Debug, Clone)]
pub struct LocalInstallConfig {
    /// Directory to install skills to
    pub install_dir: PathBuf,
    /// Whether to copy (true) or symlink (false) skills
    pub copy_mode: bool,
}

impl Default for LocalInstallConfig {
    fn default() -> Self {
        Self {
            install_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skills"))
                .unwrap_or_else(|| PathBuf::from("./skills")),
            copy_mode: true,
        }
    }
}

/// Result of a local installation
#[derive(Debug, Clone)]
pub struct LocalInstallResult {
    /// Skill name
    pub skill_name: String,
    /// Installed path
    pub path: PathBuf,
    /// Source path
    pub source_path: PathBuf,
}

/// Local skill installer
pub struct LocalInstaller {
    config: LocalInstallConfig,
}

impl LocalInstaller {
    /// Create a new local installer
    pub fn new(config: LocalInstallConfig) -> Self {
        Self { config }
    }
    
    /// Install a skill from a local path
    pub fn install(&self, source_path: &Path) -> Result<LocalInstallResult> {
        if !source_path.exists() {
            anyhow::bail!("Source path does not exist: {:?}", source_path);
        }
        
        // Determine skill name from source path
        let skill_name = self.determine_skill_name(source_path)?;
        let install_path = self.config.install_dir.join(&skill_name);
        
        // Create install directory
        fs::create_dir_all(&install_path)?;
        
        if self.config.copy_mode {
            // Copy the entire directory
            self.copy_skill(source_path, &install_path)?;
        } else {
            // Create a symlink
            self.symlink_skill(source_path, &install_path)?;
        }
        
        // Create metadata file
        self.create_metadata(&install_path, source_path)?;
        
        Ok(LocalInstallResult {
            skill_name,
            path: install_path,
            source_path: source_path.to_path_buf(),
        })
    }
    
    /// Determine skill name from source path
    fn determine_skill_name(&self, source_path: &Path) -> Result<String> {
        // Try to get name from SKILL.md first
        let skill_md = source_path.join("SKILL.md");
        if skill_md.exists() {
            if let Ok(content) = fs::read_to_string(&skill_md) {
                if let Some(name) = Self::extract_name_from_content(&content) {
                    return Ok(name);
                }
            }
        }
        
        // Fallback to directory name
        source_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Cannot determine skill name from path: {:?}", source_path))
    }
    
    /// Extract name from SKILL.md content
    fn extract_name_from_content(content: &str) -> Option<String> {
        // Look for name in YAML frontmatter
        if content.starts_with("---") {
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
        }
        
        None
    }
    
    /// Copy skill directory to install location
    fn copy_skill(&self, source: &Path, dest: &Path) -> Result<()> {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let file_name = entry.file_name();
            let dest_path = dest.join(&file_name);
            
            if file_type.is_dir() {
                fs::create_dir_all(&dest_path)?;
                self.copy_skill(&entry.path(), &dest_path)?;
            } else if file_type.is_file() {
                fs::copy(entry.path(), &dest_path)?;
            }
        }
        Ok(())
    }
    
    /// Create symlink to source directory
    #[cfg(unix)]
    fn symlink_skill(&self, source: &Path, dest: &Path) -> Result<()> {
        std::os::unix::fs::symlink(source, dest)?;
        Ok(())
    }
    
    #[cfg(windows)]
    fn symlink_skill(&self, source: &Path, dest: &Path) -> Result<()> {
        std::os::windows::fs::symlink_dir(source, dest)?;
        Ok(())
    }
    
    /// Create metadata file in installed skill
    fn create_metadata(&self, install_path: &Path, source_path: &Path) -> Result<()> {
        let metadata_path = install_path.join("LOCAL.md");
        let content = format!(
            r#"# Local Skill

This skill was installed from a local path:

```text
{}
```

Installation mode: {}

To update this skill, run:
```bash
oben skills update local/{}
```
"#,
            source_path.display(),
            if self.config.copy_mode { "copy" } else { "symlink" },
            install_path.file_name().unwrap_or_default().to_string_lossy()
        );
        
        fs::write(metadata_path, content)?;
        Ok(())
    }
    
    /// Get the installation directory
    pub fn install_dir(&self) -> &Path {
        &self.config.install_dir
    }
    
    /// List all locally installed skills
    pub fn list_installed(&self) -> Result<Vec<LocalInstallResult>> {
        let mut results = Vec::new();
        
        if !self.config.install_dir.exists() {
            return Ok(results);
        }
        
        for entry in fs::read_dir(&self.config.install_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let skill_dir = entry.path();
                let metadata = skill_dir.join("LOCAL.md");
                
                if metadata.exists() {
                    if let Some(name) = entry.file_name().to_str() {
                        results.push(LocalInstallResult {
                            skill_name: name.to_string(),
                            path: skill_dir.clone(),
                            source_path: PathBuf::new(), // Would need to read metadata to get source
                        });
                    }
                }
            }
        }
        
        Ok(results)
    }
    
    /// Uninstall a locally installed skill
    pub fn uninstall(&self, skill_name: &str) -> Result<bool> {
        let skill_path = self.config.install_dir.join(skill_name);
        
        if !skill_path.exists() {
            return Ok(false);
        }
        
        fs::remove_dir_all(&skill_path)?;
        Ok(true)
    }
}

impl Default for LocalInstaller {
    fn default() -> Self {
        Self::new(LocalInstallConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_local_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_determine_skill_name_from_file() {
        let temp_dir = temp_dir("from_file");
        let source_dir = temp_dir.join("skill-dir");
        fs::create_dir_all(&source_dir).unwrap();
        
        let skill_md = source_dir.join("SKILL.md");
        let content = "---\nname: my-skill\n---\n";
        fs::write(&skill_md, content).unwrap();
        
        let config = LocalInstallConfig::default();
        let installer = LocalInstaller::new(config);
        
        let name = installer.determine_skill_name(&source_dir).unwrap();
        assert_eq!(name, "my-skill");
    }

    #[test]
    fn test_determine_skill_name_from_dir() {
        let temp_dir = temp_dir("from_dir");
        let source_dir = temp_dir.join("my-skill-dir");
        fs::create_dir_all(&source_dir).unwrap();
        
        let config = LocalInstallConfig::default();
        let installer = LocalInstaller::new(config);
        
        let name = installer.determine_skill_name(&source_dir).unwrap();
        assert_eq!(name, "my-skill-dir");
    }

    #[test]
    fn test_install_local_skill() {
        let temp_dir = temp_dir("install");
        let source_dir = temp_dir.join("source");
        fs::create_dir_all(&source_dir.join("subdir")).unwrap();
        fs::write(source_dir.join("SKILL.md"), "---\nname: test-skill\n---\n").unwrap();
        fs::write(source_dir.join("subdir").join("file.txt"), "content").unwrap();
        
        let install_dir = temp_dir.join("installations");
        let config = LocalInstallConfig {
            install_dir: install_dir.clone(),
            ..Default::default()
        };
        
        let installer = LocalInstaller::new(config);
        
        let result = installer.install(&source_dir).unwrap();
        
        assert_eq!(result.skill_name, "test-skill");
        assert!(result.path.join("SKILL.md").exists());
    }

    #[test]
    fn test_uninstall_local_skill() {
        let temp_dir = temp_dir("uninstall");
        let source_dir = temp_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("SKILL.md"), "---\nname: test\n---\n").unwrap();
        
        let install_dir = temp_dir.join("installations");
        let config = LocalInstallConfig {
            install_dir: install_dir.clone(),
            ..Default::default()
        };
        
        let installer = LocalInstaller::new(config);
        let _ = installer.install(&source_dir).unwrap();
        
        assert!(install_dir.join("test").exists());
        
        let removed = installer.uninstall("test").unwrap();
        assert!(removed);
        assert!(!install_dir.join("test").exists());
    }

    #[test]
    fn test_copy_mode() {
        let temp_dir = temp_dir("copy_mode");
        let source_dir = temp_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("SKILL.md"), "---\nname: test\n---\n").unwrap();
        
        let install_dir = temp_dir.join("installations");
        let config = LocalInstallConfig {
            install_dir: install_dir.clone(),
            copy_mode: true,
            ..Default::default()
        };
        
        let installer = LocalInstaller::new(config);
        let _ = installer.install(&source_dir).unwrap();
        
        // After copy mode, source should still exist
        assert!(source_dir.exists());
        // And install should have copied content
        assert!(install_dir.join("test").join("SKILL.md").exists());
    }
}
