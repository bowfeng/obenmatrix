/// Skill system — integration of skills into the agent's behavior.
/// Maps to `agent/skill_utils.py`, `agent/skill_preprocessing.py`.

use anyhow::Result;
use oben_models::Skill;
use tracing::info;

/// Skill manager that controls which skills are active and how they're applied.
pub struct SkillManager {
    skills: Vec<Skill>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: vec![],
        }
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
}

impl Default for SkillManager {
    fn default() -> Self {
        Self::new()
    }
}
