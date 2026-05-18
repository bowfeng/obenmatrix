/// Skill curation — creating and improving skills from experience.
/// Maps to `agent/curator.py` and skill creation logic in Hermes.

use oben_models::Skill;
use std::collections::HashMap;
use tracing::info;

/// Curator that learns from completed conversations.
pub struct SkillCurator {
    /// Tracks which tools are used most frequently.
    tool_usage: HashMap<String, usize>,
    /// Skills that have been learned.
    learned_skills: Vec<Skill>,
}

impl SkillCurator {
    pub fn new() -> Self {
        Self {
            tool_usage: HashMap::new(),
            learned_skills: vec![],
        }
    }

    /// Record a tool usage event.
    pub fn record_tool_usage(&mut self, tool_name: &str) {
        *self.tool_usage.entry(tool_name.to_string()).or_insert(0) += 1;
    }

    /// Get top-used tools.
    pub fn top_tools(&self, limit: usize) -> Vec<(String, usize)> {
        let mut usage: Vec<_> = self.tool_usage.iter().map(|(k, v)| (k.clone(), *v)).collect();
        usage.sort_by(|a, b| b.1.cmp(&a.1));
        usage.truncate(limit);
        usage
    }

    /// Record a completed workflow that could become a skill.
    /// In a full implementation, this would call the LLM to extract a skill
    /// from the conversation history.
    pub fn review_conversation(&mut self, conversation_id: &str, tool_usage: &[String]) -> Option<Skill> {
        // Simple heuristic: if a tool was used 3+ times, suggest creating a skill
        for (tool_name, &count) in &self.tool_usage {
            if count >= 3 {
                info!("Suggesting skill for frequently used tool: {} (used {} times)", tool_name, count);
                let skill = Skill::builder(format!("auto-skill-{}", tool_name))
                    .description(format!("Automates repeated use of '{}' tool", tool_name))
                    .category("auto-generated".to_string())
                    .instructions(format!(
                        "When the user needs to perform tasks related to '{}', use the appropriate tool calls directly.",
                        tool_name
                    ))
                    .build();
                self.learned_skills.push(skill.clone());
                return Some(skill);
            }
        }
        None
    }

    /// List all learned skills.
    pub fn learned_skills(&self) -> &[Skill] {
        &self.learned_skills
    }

    /// Add a user-created skill.
    pub fn add_skill(&mut self, skill: Skill) {
        info!("Added skill: {}", skill.name);
        self.learned_skills.push(skill);
    }
}

impl Default for SkillCurator {
    fn default() -> Self {
        Self::new()
    }
}
