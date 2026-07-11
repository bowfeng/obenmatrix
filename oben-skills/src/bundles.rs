/// Skill Bundles - Grouping skills together
/// 
/// Maps to `hermes-agent/skill_bundles.py` functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A skill bundle - a collection of related skills
#[derive(Debug, Clone)]
pub struct SkillBundle {
    pub name: String,
    pub description: String,
    pub skills: Vec<String>,
    pub version: String,
}

impl SkillBundle {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            skills: Vec::new(),
            version: "1.0.0".to_string(),
        }
    }
    
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
    
    pub fn add_skill(mut self, skill: impl Into<String>) -> Self {
        self.skills.push(skill.into());
        self
    }
    
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
}

/// Skill Bundle Manager - handles bundle installation and management
pub struct SkillBundles {
    bundles_dir: PathBuf,
    installed_bundles: BTreeMap<String, SkillBundle>,
}

impl SkillBundles {
    /// Create a new SkillBundles manager
    pub fn new(bundles_dir: impl Into<PathBuf>) -> Self {
        let bundles_dir = bundles_dir.into();
        Self {
            bundles_dir,
            installed_bundles: BTreeMap::new(),
        }
    }
    
    /// Load all installed bundles
    pub fn load(&mut self) -> Result<Vec<SkillBundle>> {
        self.installed_bundles.clear();
        
        if !self.bundles_dir.exists() {
            return Ok(Vec::new());
        }
        
        for entry in fs::read_dir(&self.bundles_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") 
                || path.extension().and_then(|e| e.to_str()) == Some("yml") {
                if let Ok(bundle) = Self::load_bundle_from_file(&path) {
                    self.installed_bundles.insert(bundle.name.clone(), bundle);
                }
            }
        }
        
        Ok(self.installed_bundles.values().cloned().collect())
    }
    
    /// Load a bundle from a YAML file
    fn load_bundle_from_file(path: &Path) -> Result<SkillBundle> {
        let content = fs::read_to_string(path)?;
        Self::load_bundle_from_yaml(&content)
    }
    
    /// Load a bundle from YAML content
    fn load_bundle_from_yaml(content: &str) -> Result<SkillBundle> {
        let mut name = "unknown".to_string();
        let mut description = String::new();
        let mut skills = Vec::new();
        let mut version = "1.0.0".to_string();
        let mut in_skills_list = false;
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            // Check if we're at the start of the skills list
            if trimmed.starts_with("skills:") {
                in_skills_list = true;
                continue;
            }
            
            // If we're in the skills list and find a list item
            if in_skills_list {
                if let Some(skill) = trimmed.strip_prefix("- ") {
                    let skill = skill.trim().trim_matches('"').trim_matches('\'');
                    if !skill.is_empty() {
                        skills.push(skill.to_string());
                    }
                } else if trimmed.starts_with("name:") || trimmed.starts_with("description:") || trimmed.starts_with("version:") {
                    // End of skills list
                    in_skills_list = false;
                }
            }
            
            // Parse top-level fields
            if !in_skills_list && trimmed.starts_with("skills:") {
                continue;
            }
            
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                
                if key == "name" && !in_skills_list {
                    name = value.trim_matches('"').trim_matches('\'').to_string();
                } else if key == "description" && !in_skills_list {
                    description = value.trim_matches('"').trim_matches('\'').to_string();
                } else if key == "version" && !in_skills_list {
                    version = value.trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
        
        Ok(SkillBundle {
            name,
            description,
            skills,
            version,
        })
    }
    
    /// Install a bundle from a YAML file
    pub fn install_from_file(&mut self, path: impl AsRef<Path>) -> Result<SkillBundle> {
        let path = path.as_ref();
        let bundle = Self::load_bundle_from_file(path)?;
        
        // Create bundles directory if it doesn't exist
        fs::create_dir_all(&self.bundles_dir)?;
        
        // Copy bundle file to bundles directory
        let dest_path = self.bundles_dir.join(format!("{}.yaml", bundle.name));
        fs::copy(path, &dest_path)?;
        
        self.installed_bundles.insert(bundle.name.clone(), bundle.clone());
        
        Ok(bundle)
    }
    
    /// Install a bundle from YAML content
    pub fn install_from_yaml(&mut self, content: &str) -> Result<SkillBundle> {
        let bundle = Self::load_bundle_from_yaml(content)?;
        
        fs::create_dir_all(&self.bundles_dir)?;
        
        let yaml_content = format!(
            r#"name: {}
description: {}
version: {}
skills:
  {}
"#,
            bundle.name,
            bundle.description,
            bundle.version,
            bundle.skills.iter().map(|s| format!("  - {}", s)).collect::<Vec<_>>().join("\n")
        );
        
        let dest_path = self.bundles_dir.join(format!("{}.yaml", bundle.name));
        fs::write(&dest_path, &yaml_content)?;
        
        self.installed_bundles.insert(bundle.name.clone(), bundle.clone());
        
        Ok(bundle)
    }
    
    /// Uninstall a bundle by name
    pub fn uninstall(&mut self, name: &str) -> Result<bool> {
        if !self.installed_bundles.contains_key(name) {
            return Ok(false);
        }
        
        let bundle_path = self.bundles_dir.join(format!("{}.yaml", name));
        if bundle_path.exists() {
            fs::remove_file(&bundle_path)?;
        }
        
        self.installed_bundles.remove(name);
        Ok(true)
    }
    
    /// Get a bundle by name
    pub fn get(&self, name: &str) -> Option<&SkillBundle> {
        self.installed_bundles.get(name)
    }
    
    /// List all installed bundles
    pub fn list(&self) -> Vec<SkillBundle> {
        self.installed_bundles.values().cloned().collect()
    }
    
    /// Check if a bundle is installed
    pub fn is_installed(&self, name: &str) -> bool {
        self.installed_bundles.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_bundles_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_skill_bundle_new() {
        let bundle = SkillBundle::new("test-bundle");
        assert_eq!(bundle.name, "test-bundle");
        assert!(bundle.description.is_empty());
        assert!(bundle.skills.is_empty());
        assert_eq!(bundle.version, "1.0.0");
    }

    #[test]
    fn test_skill_bundle_builder() {
        let bundle = SkillBundle::new("my-bundle")
            .description("A test bundle")
            .add_skill("skill1")
            .add_skill("skill2")
            .version("2.0.0");
        
        assert_eq!(bundle.name, "my-bundle");
        assert_eq!(bundle.description, "A test bundle");
        assert_eq!(bundle.skills, vec!["skill1", "skill2"]);
        assert_eq!(bundle.version, "2.0.0");
    }

    #[test]
    fn test_load_bundle_from_yaml() {
        let yaml = r#"name: test-bundle
description: A test bundle
version: 1.5.0
skills:
  - skill1
  - skill2
"#;
        
        let bundle = SkillBundles::load_bundle_from_yaml(yaml).unwrap();
        
        assert_eq!(bundle.name, "test-bundle");
        assert_eq!(bundle.description, "A test bundle");
        assert_eq!(bundle.version, "1.5.0");
        assert_eq!(bundle.skills, vec!["skill1", "skill2"]);
    }

    #[test]
    fn test_load_bundle_from_yaml_minimal() {
        let yaml = "name: minimal-bundle\n";
        
        let bundle = SkillBundles::load_bundle_from_yaml(yaml).unwrap();
        
        assert_eq!(bundle.name, "minimal-bundle");
        assert!(bundle.description.is_empty());
        assert!(bundle.skills.is_empty());
    }

    #[test]
    fn test_bundle_install_from_yaml() {
        let temp_dir = temp_dir("install_yaml");
        let mut manager = SkillBundles::new(&temp_dir);
        
        let yaml = r#"name: test-bundle
description: Test bundle
version: 1.0.0
skills:
  - skill1
"#;
        
        let bundle = manager.install_from_yaml(yaml).unwrap();
        
        assert!(manager.is_installed("test-bundle"));
        assert_eq!(bundle.skills, vec!["skill1"]);
        
        // Verify file was created
        let bundle_file = temp_dir.join("test-bundle.yaml");
        assert!(bundle_file.exists());
    }

    #[test]
    fn test_bundle_uninstall() {
        let temp_dir = temp_dir("uninstall");
        let mut manager = SkillBundles::new(&temp_dir);
        
        let yaml = "name: test-bundle\nskills:\n  - skill1\n";
        manager.install_from_yaml(yaml).unwrap();
        
        assert!(manager.is_installed("test-bundle"));
        
        let removed = manager.uninstall("test-bundle").unwrap();
        assert!(removed);
        assert!(!manager.is_installed("test-bundle"));
        
        // Verify file was deleted
        let bundle_file = temp_dir.join("test-bundle.yaml");
        assert!(!bundle_file.exists());
    }

    #[test]
    fn test_bundle_list() {
        let temp_dir = temp_dir("list_bundles");
        let mut manager = SkillBundles::new(&temp_dir);
        
        let yaml1 = "name: bundle1\n";
        let yaml2 = "name: bundle2\n";
        
        manager.install_from_yaml(yaml1).unwrap();
        manager.install_from_yaml(yaml2).unwrap();
        
        let bundles = manager.list();
        assert_eq!(bundles.len(), 2);
        let names: Vec<&str> = bundles.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"bundle1"));
        assert!(names.contains(&"bundle2"));
    }
}
