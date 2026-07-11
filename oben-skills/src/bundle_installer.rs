/// Bundle Skill Installer - Install skills from bundles
/// 
/// Maps to `hermes-agent/skill_bundles.py` installation functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Bundle installation result
#[derive(Debug, Clone)]
pub struct BundleInstallResult {
    /// Bundle name
    pub bundle_name: String,
    /// Number of skills installed
    pub skills_installed: usize,
    /// Map of skill names to their paths
    pub installed_skills: BTreeMap<String, PathBuf>,
}

/// Bundle installer configuration
#[derive(Debug, Clone)]
pub struct BundleInstallConfig {
    /// Directory to install skills to
    pub install_dir: PathBuf,
    /// Bundle storage directory
    pub bundles_dir: PathBuf,
}

impl Default for BundleInstallConfig {
    fn default() -> Self {
        Self {
            install_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/skills"))
                .unwrap_or_else(|| PathBuf::from("./skills")),
            bundles_dir: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/bundles"))
                .unwrap_or_else(|| PathBuf::from("./bundles")),
        }
    }
}

/// Bundle installer
pub struct BundleInstaller {
    config: BundleInstallConfig,
}

impl BundleInstaller {
    /// Create a new bundle installer
    pub fn new(config: BundleInstallConfig) -> Self {
        Self { config }
    }
    
    /// Install a bundle from a YAML file
    pub fn install_from_file(&self, bundle_path: &Path) -> Result<BundleInstallResult> {
        if !bundle_path.exists() {
            anyhow::bail!("Bundle file does not exist: {:?}", bundle_path);
        }
        
        let content = fs::read_to_string(bundle_path)?;
        let bundle_data = Self::parse_bundle_yaml(&content)?;
        
        self.install_bundle(&bundle_data)
    }
    
    /// Install a bundle from YAML content
    pub fn install_from_yaml(&self, yaml_content: &str) -> Result<BundleInstallResult> {
        let bundle_data = Self::parse_bundle_yaml(yaml_content)?;
        self.install_bundle(&bundle_data)
    }
    
    /// Parse bundle YAML content
    fn parse_bundle_yaml(content: &str) -> Result<BundleData> {
        let mut name = String::new();
        let mut skills = Vec::new();
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("name:") {
                name = trimmed
                    .strip_prefix("name:")
                    .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                    .unwrap_or_default()
                    .to_string();
            } else if trimmed.starts_with("- ") {
                let skill = trimmed.strip_prefix("- ").unwrap_or_default().trim();
                if !skill.is_empty() {
                    skills.push(skill.to_string());
                }
            }
        }
        
        if name.is_empty() {
            anyhow::bail!("Bundle YAML missing 'name' field");
        }
        
        Ok(BundleData { name, skills })
    }
    
    /// Install all skills from a bundle
    fn install_bundle(&self, bundle: &BundleData) -> Result<BundleInstallResult> {
        fs::create_dir_all(&self.config.install_dir)?;
        fs::create_dir_all(&self.config.bundles_dir)?;
        
        let mut installed_skills = BTreeMap::new();
        let mut skills_installed = 0;
        
        for skill_name in &bundle.skills {
            // For now, create placeholder skills
            // In a real implementation, this would download or copy actual skills
            let skill_path = self.config.install_dir.join(skill_name);
            fs::create_dir_all(&skill_path)?;
            
            let skill_md = skill_path.join("SKILL.md");
            let content = format!(
                r#"---
name: {}
source: bundle:{}
---

# {}

This skill was installed as part of the bundle: {}

To manage this skill, use:
```bash
oben skills enable {}
oben skills disable {}
oben skills remove {}
```
"#,
                skill_name, bundle.name, skill_name, bundle.name, skill_name, skill_name, skill_name
            );
            
            fs::write(&skill_md, content)?;
            installed_skills.insert(skill_name.clone(), skill_path.clone());
            skills_installed += 1;
        }
        
        // Save bundle info
        self.save_bundle_info(bundle, &installed_skills)?;
        
        Ok(BundleInstallResult {
            bundle_name: bundle.name.clone(),
            skills_installed,
            installed_skills,
        })
    }
    
    /// Save bundle information for tracking
    fn save_bundle_info(&self, bundle: &BundleData, skills: &BTreeMap<String, PathBuf>) -> Result<()> {
        let bundle_dir = self.config.bundles_dir.join(&bundle.name);
        fs::create_dir_all(&bundle_dir)?;
        
        let info_path = bundle_dir.join("info.yaml");
        let mut info_content = format!(
            r#"name: {}
skills:""",
            bundle.name
        );
        
        for (skill_name, path) in skills {
            info_content.push_str(&format!("\n  - name: {}\n    path: {}", skill_name, path.display()));
        }
        
        fs::write(&info_path, info_content)?;
        Ok(())
    }
    
    /// List installed bundles
    pub fn list_bundles(&self) -> Result<Vec<String>> {
        let mut bundles = Vec::new();
        
        if !self.config.bundles_dir.exists() {
            return Ok(bundles);
        }
        
        for entry in fs::read_dir(&self.config.bundles_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    bundles.push(name.to_string());
                }
            }
        }
        
        Ok(bundles)
    }
    
    /// Uninstall a bundle by name
    pub fn uninstall(&self, bundle_name: &str) -> Result<bool> {
        let bundle_dir = self.config.bundles_dir.join(bundle_name);
        
        if !bundle_dir.exists() {
            return Ok(false);
        }
        
        // Remove bundle info
        fs::remove_dir_all(&bundle_dir)?;
        
        // TODO: Also remove the actual skills (or mark them as unmanaged)
        // For now, just return success
        Ok(true)
    }
    
    /// Get bundle info
    pub fn get_bundle_info(&self, bundle_name: &str) -> Result<Option<BundleData>> {
        let bundle_dir = self.config.bundles_dir.join(bundle_name);
        
        if !bundle_dir.exists() {
            return Ok(None);
        }
        
        let info_path = bundle_dir.join("info.yaml");
        if !info_path.exists() {
            return Ok(None);
        }
        
        let content = fs::read_to_string(&info_path)?;
        let bundle_data = Self::parse_bundle_yaml(&content)?;
        
        Ok(Some(bundle_data))
    }
}

/// Bundle data structure
#[derive(Debug, Clone)]
pub struct BundleData {
    pub name: String,
    pub skills: Vec<String>,
}

impl Default for BundleInstaller {
    fn default() -> Self {
        Self::new(BundleInstallConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_bundle_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_parse_bundle_yaml() {
        let yaml = r#"name: test-bundle
skills:
  - skill1
  - skill2
  - skill3"#;

        let bundle = BundleInstaller::parse_bundle_yaml(yaml).unwrap();
        
        assert_eq!(bundle.name, "test-bundle");
        assert_eq!(bundle.skills.len(), 3);
        assert_eq!(bundle.skills[0], "skill1");
    }

    #[test]
    fn test_install_bundle() {
        let temp_dir = temp_dir("install");
        let install_dir = temp_dir.join("skills");
        let bundles_dir = temp_dir.join("bundles");
        
        let config = BundleInstallConfig {
            install_dir: install_dir.clone(),
            bundles_dir: bundles_dir.clone(),
            ..Default::default()
        };
        
        let installer = BundleInstaller::new(config);
        let yaml = r#"name: test-bundle
skills:
  - skill1
  - skill2"#;

        let result = installer.install_from_yaml(yaml).unwrap();
        
        assert_eq!(result.bundle_name, "test-bundle");
        assert_eq!(result.skills_installed, 2);
        assert!(result.installed_skills.contains_key("skill1"));
        assert!(result.installed_skills.contains_key("skill2"));
    }

    #[test]
    fn test_list_bundles_empty() {
        let temp_dir = temp_dir("list_empty");
        let bundles_dir = temp_dir.join("bundles");
        
        let config = BundleInstallConfig {
            install_dir: temp_dir.clone(),
            bundles_dir: bundles_dir.clone(),
            ..Default::default()
        };
        
        let installer = BundleInstaller::new(config);
        
        let bundles = installer.list_bundles().unwrap();
        assert!(bundles.is_empty());
    }

    #[test]
    fn test_list_bundles_with_bundles() {
        let temp_dir = temp_dir("list_with");
        let install_dir = temp_dir.join("skills");
        let bundles_dir = temp_dir.join("bundles");
        
        let config = BundleInstallConfig {
            install_dir,
            bundles_dir: bundles_dir.clone(),
            ..Default::default()
        };
        
        let installer = BundleInstaller::new(config);
        
        // Install a bundle
        let yaml = r#"name: test-bundle
skills:
  - skill1"#;
        let _ = installer.install_from_yaml(yaml);
        
        let bundles = installer.list_bundles().unwrap();
        assert!(bundles.contains(&"test-bundle".to_string()));
    }

    #[test]
    fn test_uninstall_bundle() {
        let temp_dir = temp_dir("uninstall");
        let install_dir = temp_dir.join("skills");
        let bundles_dir = temp_dir.join("bundles");
        
        let config = BundleInstallConfig {
            install_dir,
            bundles_dir: bundles_dir.clone(),
            ..Default::default()
        };
        
        let installer = BundleInstaller::new(config);
        
        let yaml = r#"name: test-bundle
skills:
  - skill1"#;
        let _ = installer.install_from_yaml(yaml);
        
        assert!(bundles_dir.join("test-bundle").exists());
        
        let removed = installer.uninstall("test-bundle").unwrap();
        assert!(removed);
        assert!(!bundles_dir.join("test-bundle").exists());
    }
}
