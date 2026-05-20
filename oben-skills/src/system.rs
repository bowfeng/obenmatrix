/// Skill system — integration of skills into the agent's behavior.
/// Maps to `agent/skill_utils.py`, `agent/skill_preprocessing.py`.

use crate::skill_preprocessing::PreprocessingConfig;
use oben_models::Skill;
use tracing::info;

/// Skill manager that controls which skills are active and how they're applied.
pub struct SkillManager {
    skills: Vec<Skill>,
    config: PreprocessingConfig,
    skill_dir: Option<std::path::PathBuf>,
    session_id: Option<String>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: vec![],
            config: PreprocessingConfig::default(),
            skill_dir: None,
            session_id: None,
        }
    }

    /// Set preprocessing configuration.
    pub fn with_preprocessing_config(mut self, config: PreprocessingConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the skills directory (for ${HERMES_SKILL_DIR} substitution).
    pub fn with_skill_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.skill_dir = Some(dir);
        self
    }

    /// Set session ID (for ${HERMES_SESSION_ID} substitution).
    pub fn with_session_id(mut self, id: String) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Load skills from a loader.
    pub fn load_skills(&mut self, skills: Vec<Skill>) {
        let enabled: Vec<Skill> = skills.into_iter().filter(|s| s.enabled).collect();
        info!("Loaded {} enabled skills", enabled.len());
        self.skills = enabled;
    }

    /// Get instructions from all enabled skills, to be prepended to system prompt.
    pub fn build_skill_instructions(&self) -> String {
        let mut instructions = String::from("\n## Available Skills\n");
        for skill in &self.skills {
            if skill.auto_use {
                instructions.push_str(&format!(
                    "### {} ({})\n{}\n",
                    skill.name,
                    skill.category,
                    skill.instructions
                ));
            }
        }
        instructions
    }

    /// Get instructions from all enabled skills (including manual ones).
    pub fn build_all_skill_instructions(&self) -> String {
        let mut instructions = String::from("\n## Available Skills\n");
        for skill in &self.skills {
            instructions.push_str(&format!(
                "### {} ({})\n{}\n",
                skill.name,
                skill.category,
                skill.instructions
            ));
        }
        instructions
    }

    /// Check if a skill should be auto-used.
    pub fn auto_use_skills(&self) -> Vec<&Skill> {
        self.skills.iter().filter(|s| s.auto_use).collect()
    }

    /// Enable a skill by name.
    pub fn enable_skill(&mut self, name: &str) -> bool {
        if let Some(skill) = self.skills.iter_mut().find(|s| s.name == name) {
            skill.enabled = true;
            info!("Enabled skill: {}", name);
            true
        } else {
            false
        }
    }

    /// Disable a skill by name.
    pub fn disable_skill(&mut self, name: &str) -> bool {
        if let Some(skill) = self.skills.iter_mut().find(|s| s.name == name) {
            skill.enabled = false;
            info!("Disabled skill: {}", name);
            true
        } else {
            false
        }
    }

    /// List all skills.
    pub fn list_skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Skill count.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// List skill names for display.
    pub fn list_skill_names(&self) -> Vec<String> {
        self.skills.iter().map(|s| s.name.clone()).collect()
    }

    /// List skills by category.
    pub fn skills_by_category(&self) -> Vec<(&str, Vec<&Skill>)> {
        let mut categories: std::collections::HashMap<&str, Vec<&Skill>> = std::collections::HashMap::new();
        for skill in &self.skills {
            categories.entry(&skill.category).or_default().push(skill);
        }
        categories.into_iter().collect()
    }
}

impl Default for SkillManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::builtin_skills;
    use super::*;

    #[test]
    fn test_skill_manager_new_is_empty() {
        let mgr = SkillManager::new();
        assert_eq!(mgr.count(), 0);
        assert!(mgr.list_skills().is_empty());
    }

    #[test]
    fn test_skill_manager_load_skills() {
        let mut mgr = SkillManager::new();
        let skills = vec![Skill::builder("test-skill").metadata(None)
            .description("A test skill")
            .category("testing")
            .instructions("Do test things")
            .enabled(true)
            .auto_use(true)
            .build()];
        mgr.load_skills(skills);
        assert_eq!(mgr.count(), 1);
    }

    #[test]
    fn test_skill_manager_filters_disabled() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("enabled").metadata(None)
                .description("enabled skill")
                .category("test")
                .instructions("do stuff")
                .enabled(true)
                .build(),
            Skill::builder("disabled").metadata(None)
                .description("disabled skill")
                .category("test")
                .instructions("do stuff")
                .enabled(false)
                .build(),
        ]);
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.list_skills()[0].name, "enabled");
    }

    #[test]
    fn test_skill_manager_build_instructions() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("auto-skill").metadata(None)
                .description("Auto-use skill")
                .category("core")
                .instructions("You must do this.")
                .enabled(true)
                .auto_use(true)
                .build(),
            Skill::builder("manual-skill").metadata(None)
                .description("Manual skill")
                .category("core")
                .instructions("Use when needed.")
                .enabled(true)
                .auto_use(false)
                .build(),
        ]);
        let instructions = mgr.build_skill_instructions();
        assert!(instructions.contains("## Available Skills"));
        assert!(instructions.contains("auto-skill"));
        assert!(instructions.contains("You must do this."));
        assert!(!instructions.contains("manual-skill"));
    }

    #[test]
    fn test_skill_manager_build_all_instructions() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("skill-a").metadata(None)
                .description("Skill A")
                .category("core")
                .instructions("Do A.")
                .enabled(true)
                .build(),
            Skill::builder("skill-b").metadata(None)
                .description("Skill B")
                .category("core")
                .instructions("Do B.")
                .enabled(true)
                .build(),
        ]);
        let instructions = mgr.build_all_skill_instructions();
        assert!(instructions.contains("skill-a"));
        assert!(instructions.contains("skill-b"));
    }

    #[test]
    fn test_skill_manager_enable_skill() {
        let mut mgr = SkillManager::new();
        // Load an enabled skill so it's in the list
        mgr.load_skills(vec![Skill::builder("test-skill").metadata(None)
            .description("Test")
            .category("core")
            .instructions("Instructions")
            .enabled(true)
            .build()]);
        mgr.disable_skill("test-skill");
        assert!(!mgr.list_skills()[0].enabled);
        assert!(mgr.enable_skill("test-skill"));
        assert!(mgr.list_skills()[0].enabled);
    }

    #[test]
    fn test_skill_manager_disable_skill() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![Skill::builder("test-skill").metadata(None)
            .description("Test")
            .category("core")
            .instructions("Instructions")
            .enabled(true)
            .build()]);
        assert!(mgr.disable_skill("test-skill"));
        assert!(!mgr.list_skills()[0].enabled);
    }

    #[test]
    fn test_skill_manager_enable_unknown() {
        let mut mgr = SkillManager::new();
        assert!(!mgr.enable_skill("nonexistent"));
    }

    #[test]
    fn test_skill_manager_auto_use_skills() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("auto").metadata(None)
                .description("auto-use")
                .category("core")
                .instructions("Do auto.")
                .enabled(true)
                .auto_use(true)
                .build(),
            Skill::builder("manual").metadata(None)
                .description("manual")
                .category("core")
                .instructions("Manual.")
                .enabled(true)
                .auto_use(false)
                .build(),
        ]);
        let auto_skills = mgr.auto_use_skills();
        assert_eq!(auto_skills.len(), 1);
        assert_eq!(auto_skills[0].name, "auto");
    }

    #[test]
    fn test_builtin_skills() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "general");
        assert_eq!(skills[0].category, "core");
        assert!(skills[0].enabled);
        assert!(skills[0].auto_use);
    }

    #[test]
    fn test_skill_builder_defaults() {
        let skill = Skill::builder("defaults-test").metadata(None).build();
        assert_eq!(skill.name, "defaults-test");
        assert!(skill.enabled);
        assert!(!skill.auto_use);
        assert_eq!(skill.description, "");
        assert_eq!(skill.category, "");
        assert_eq!(skill.instructions, "");
    }

    #[test]
    fn test_skill_builder_full() {
        let skill = Skill::builder("full-test").metadata(None)
            .description("Full description")
            .category("testing")
            .instructions("Full instructions")
            .enabled(false)
            .auto_use(true)
            .build();
        assert_eq!(skill.description, "Full description");
        assert_eq!(skill.category, "testing");
        assert_eq!(skill.instructions, "Full instructions");
        assert!(!skill.enabled);
        assert!(skill.auto_use);
    }

    #[test]
    fn test_skill_manager_list_skill_names() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("skill-a").metadata(None).description("A").build(),
            Skill::builder("skill-b").metadata(None).description("B").build(),
        ]);
        let names = mgr.list_skill_names();
        assert_eq!(names, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn test_skill_manager_skills_by_category() {
        let mut mgr = SkillManager::new();
        mgr.load_skills(vec![
            Skill::builder("skill-a").metadata(None).description("A").category("cat1").build(),
            Skill::builder("skill-b").metadata(None).description("B").category("cat1").build(),
            Skill::builder("skill-c").metadata(None).description("C").category("cat2").build(),
        ]);
        let categories = mgr.skills_by_category();
        assert_eq!(categories.len(), 2);
        let cat1 = categories.iter().find(|(name, _)| *name == "cat1").unwrap();
        assert_eq!(cat1.1.len(), 2);
        let cat2 = categories.iter().find(|(name, _)| *name == "cat2").unwrap();
        assert_eq!(cat2.1.len(), 1);
    }
}
