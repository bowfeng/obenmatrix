/// Skill Category Manager - Manage skill categories
/// 
/// Maps to `hermes-agent/skills_hub.py` category management functionality
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// Category definition
#[derive(Debug, Clone)]
pub struct Category {
    /// Category name
    pub name: String,
    /// Category description
    pub description: String,
    /// Skills in this category
    pub skills: BTreeSet<String>,
    /// Category icon (for UI)
    pub icon: Option<String>,
    /// Parent category (for hierarchical categories)
    pub parent: Option<String>,
}

impl Category {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            skills: BTreeSet::new(),
            icon: None,
            parent: None,
        }
    }
    
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
    
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    
    pub fn parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
    
    pub fn add_skill(mut self, skill: impl Into<String>) -> Self {
        self.skills.insert(skill.into());
        self
    }
}

/// Category configuration
#[derive(Debug, Clone)]
pub struct CategoryConfig {
    /// Storage path for categories
    pub storage_path: PathBuf,
    /// Default categories to create if none exist
    pub default_categories: Vec<Category>,
}

impl Default for CategoryConfig {
    fn default() -> Self {
        Self {
            storage_path: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".agents/categories.yaml"))
                .unwrap_or_else(|| PathBuf::from("./categories.yaml")),
            default_categories: vec![
                Category::new("development").description("Development tools and utilities"),
                Category::new("analysis").description("Data analysis and processing"),
                Category::new("automation").description("Automated tasks and workflows"),
                Category::new("communication").description("Communication and messaging"),
                Category::new("utilities").description("General utility functions"),
            ],
        }
    }
}

/// Category manager
pub struct SkillCategoryManager {
    config: CategoryConfig,
    categories: BTreeMap<String, Category>,
}

impl SkillCategoryManager {
    /// Create a new category manager
    pub fn new(config: CategoryConfig) -> Self {
        let storage_path = config.storage_path.clone();
        fs::create_dir_all(storage_path.parent().unwrap_or(&storage_path)).ok();
        
        Self {
            config,
            categories: BTreeMap::new(),
        }
    }
    
    /// Add a category
    pub fn add_category(&mut self, category: Category) -> Result<()> {
        self.categories.insert(category.name.clone(), category);
        Ok(())
    }
    
    /// Remove a category
    pub fn remove_category(&mut self, name: &str) -> Result<bool> {
        Ok(self.categories.remove(name).is_some())
    }
    
    /// Add a skill to a category
    pub fn add_skill_to_category(&mut self, category_name: &str, skill_name: &str) -> Result<bool> {
        if let Some(category) = self.categories.get_mut(category_name) {
            category.skills.insert(skill_name.to_string());
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Remove a skill from a category
    pub fn remove_skill_from_category(&mut self, category_name: &str, skill_name: &str) -> Result<bool> {
        if let Some(category) = self.categories.get_mut(category_name) {
            Ok(category.skills.remove(skill_name))
        } else {
            Ok(false)
        }
    }
    
    /// Get a category by name
    pub fn get_category(&self, name: &str) -> Option<&Category> {
        self.categories.get(name)
    }
    
    /// Get all categories
    pub fn get_all_categories(&self) -> Vec<&Category> {
        self.categories.values().collect()
    }
    
    /// Get all skills in a category
    pub fn get_category_skills(&self, category_name: &str) -> Option<Vec<String>> {
        self.categories.get(category_name).map(|c| c.skills.iter().cloned().collect())
    }
    
    /// Get all skills across all categories
    pub fn get_all_skills(&self) -> BTreeSet<String> {
        let mut all_skills = BTreeSet::new();
        for category in self.categories.values() {
            for skill in &category.skills {
                all_skills.insert(skill.clone());
            }
        }
        all_skills
    }
    
    /// Get categories that contain a skill
    pub fn get_skill_categories(&self, skill_name: &str) -> Vec<&Category> {
        self.categories
            .values()
            .filter(|c| c.skills.contains(skill_name))
            .collect()
    }
    
    /// Load categories from file
    pub fn load(&mut self) -> Result<()> {
        if !self.config.storage_path.exists() {
            // Create default categories
            let categories = self.config.default_categories.clone();
            for category in categories {
                self.add_category(category)?;
            }
            return Ok(());
        }
        
        let content = fs::read_to_string(&self.config.storage_path)?;
        self.from_yaml(&content);
        
        Ok(())
    }
    
    /// Save categories to file
    pub fn save(&self) -> Result<()> {
        let content = self.to_yaml();
        fs::write(&self.config.storage_path, &content)?;
        Ok(())
    }
    
    /// Convert to YAML format
    fn to_yaml(&self) -> String {
        let mut yaml = String::from("categories:\n");
        
        for (name, category) in &self.categories {
            yaml.push_str(&format!("  {}:\n", name));
            if !category.description.is_empty() {
                yaml.push_str(&format!("    description: {}\n", category.description));
            }
            
            if !category.skills.is_empty() {
                yaml.push_str("    skills:\n");
                for skill in &category.skills {
                    yaml.push_str(&format!("      - {}\n", skill));
                }
            }
        }
        
        yaml
    }
    
    /// Parse from YAML format
    fn from_yaml(&mut self, content: &str) {
        let mut current_category: Option<String> = None;
        
        for line in content.lines() {
            // Skip header and empty lines
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("categories:") {
                continue;
            }
            
            // Category name (2-space indent, ends with :)
            if line.starts_with("  ") && !line.trim_start().starts_with("    ") && line.trim().ends_with(':') {
                let cat_name = line.trim_start().strip_suffix(':').unwrap_or("").to_string();
                current_category = Some(cat_name.clone());
                self.categories.entry(cat_name.clone()).or_insert_with(|| {
                    Category::new(cat_name)
                });
            } else if let Some(ref cat_name) = current_category {
                if let Some((key, value)) = trimmed.split_once(':') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    
                    if let Some(category) = self.categories.get_mut(cat_name) {
                        match key {
                            "description" => category.description = value.to_string(),
                            "skills" => {},
                            _ => {}
                        }
                    }
                } else if trimmed.starts_with("- ") {
                    if let Some(skill_name) = trimmed.strip_prefix("- ").map(|s| s.trim().to_string()) {
                        if let Some(category) = self.categories.get_mut(cat_name) {
                            category.skills.insert(skill_name);
                        }
                    }
                }
            }
        }
    }
    
    /// Get the number of categories
    pub fn category_count(&self) -> usize {
        self.categories.len()
    }
    
    /// Get the total number of skills across all categories
    pub fn skill_count(&self) -> usize {
        self.categories.values().map(|c| c.skills.len()).sum()
    }
    
    /// Rename a category
    pub fn rename_category(&mut self, old_name: &str, new_name: &str) -> Result<bool> {
        if let Some(category) = self.categories.remove(old_name) {
            let mut renamed = category;
            renamed.name = new_name.to_string();
            self.categories.insert(new_name.to_string(), renamed);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Merge categories
    pub fn merge_categories(&mut self, from_name: &str, to_name: &str) -> Result<bool> {
        if let (Some(from_cat), Some(to_cat)) = (
            self.categories.remove(from_name),
            self.categories.get_mut(to_name),
        ) {
            for skill in from_cat.skills {
                to_cat.skills.insert(skill);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Default for SkillCategoryManager {
    fn default() -> Self {
        Self::new(CategoryConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_category_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_add_category() {
        let _temp_dir = temp_dir("add");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        
        manager.add_category(Category::new("test-category").description("A test category")).unwrap();
        
        assert!(manager.get_category("test-category").is_some());
    }

    #[test]
    fn test_add_skill_to_category() {
        let temp_dir = temp_dir("skill");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        manager.add_category(Category::new("test-category")).unwrap();
        
        manager.add_skill_to_category("test-category", "skill1").unwrap();
        
        let skills = manager.get_category_skills("test-category").unwrap();
        assert!(skills.contains(&"skill1".to_string()));
    }

    #[test]
    fn test_get_skill_categories() {
        let _temp_dir = temp_dir("skill_cats");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        manager.add_category(Category::new("category1")).unwrap();
        manager.add_category(Category::new("category2")).unwrap();
        
        manager.add_skill_to_category("category1", "skill1").unwrap();
        manager.add_skill_to_category("category2", "skill1").unwrap();
        
        let categories = manager.get_skill_categories("skill1");
        assert_eq!(categories.len(), 2);
    }

    #[test]
    fn test_get_all_skills() {
        let _temp_dir = temp_dir("all_skills");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        manager.add_category(Category::new("category1")).unwrap();
        manager.add_category(Category::new("category2")).unwrap();
        
        manager.add_skill_to_category("category1", "skill1").unwrap();
        manager.add_skill_to_category("category1", "skill2").unwrap();
        manager.add_skill_to_category("category2", "skill3").unwrap();
        
        let all_skills = manager.get_all_skills();
        assert_eq!(all_skills.len(), 3);
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = temp_dir("save_load");
        let config = CategoryConfig {
            storage_path: temp_dir.join("categories.yaml"),
            ..Default::default()
        };
        
        let mut manager1 = SkillCategoryManager::new(config.clone());
        manager1.add_category(Category::new("test-category")).unwrap();
        manager1.add_skill_to_category("test-category", "skill1").unwrap();
        manager1.save().unwrap();
        
        let mut manager2 = SkillCategoryManager::new(config);
        manager2.load().unwrap();
        
        assert!(manager2.get_category("test-category").is_some());
    }

    #[test]
    fn test_rename_category() {
        let _temp_dir = temp_dir("rename");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        manager.add_category(Category::new("old-name")).unwrap();
        
        let result = manager.rename_category("old-name", "new-name").unwrap();
        assert!(result);
        assert!(manager.get_category("new-name").is_some());
        assert!(manager.get_category("old-name").is_none());
    }

    #[test]
    fn test_merge_categories() {
        let temp_dir = temp_dir("merge");
        let config = CategoryConfig::default();
        
        let mut manager = SkillCategoryManager::new(config);
        manager.add_category(Category::new("cat1")).unwrap();
        manager.add_category(Category::new("cat2")).unwrap();
        
        manager.add_skill_to_category("cat1", "skill1").unwrap();
        manager.add_skill_to_category("cat2", "skill2").unwrap();
        
        let result = manager.merge_categories("cat1", "cat2").unwrap();
        assert!(result);
        
        let skills = manager.get_category_skills("cat2").unwrap();
        assert!(skills.contains(&"skill1".to_string()));
        assert!(skills.contains(&"skill2".to_string()));
    }
}
