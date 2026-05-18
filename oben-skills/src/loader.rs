/// Load skills from YAML/TXT files.
/// Maps to how Hermes loads skill files from the `skills/` directory.

use anyhow::Result;
use oben_models::{Skill, Tool, ToolParameters, ToolParameter};
use std::path::{Path, PathBuf};
use tracing::{info, debug};

/// Skill loader — reads skill definitions from disk.
pub struct SkillLoader {
    skill_dirs: Vec<PathBuf>,
}

impl SkillLoader {
    pub fn new() -> Self {
        Self {
            skill_dirs: vec![],
        }
    }

    /// Add a directory to search for skills.
    pub fn add_dir(&mut self, dir: impl Into<PathBuf>) {
        self.skill_dirs.push(dir.into());
    }

    /// Load all skills from configured directories.
    pub fn load_all(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();

        for dir in &self.skill_dirs {
            if !dir.exists() {
                debug!("Skill directory not found: {}", dir.display());
                continue;
            }

            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    // Each subdirectory is a skill category
                    if let Some(skill) = self.load_from_dir(&path) {
                        skills.push(skill);
                    }
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "yaml" || ext == "yml" || ext == "txt" || ext == "md" {
                        if let Some(skill) = self.load_file(&path) {
                            skills.push(skill);
                        }
                    }
                }
            }
        }

        info!("Loaded {} skills from {} directories", skills.len(), self.skill_dirs.len());
        Ok(skills)
    }

    fn load_from_dir(&self, dir: &Path) -> Option<Skill> {
        let name = dir.file_name()?.to_str()?;
        let skill_dir = dir;

        // Try to find a SKILL.md or skill.yaml
        let skill_file = skill_dir.join("SKILL.md");
        let yaml_file = skill_dir.join("SKILL.yaml");

        if skill_file.exists() {
            let content = std::fs::read_to_string(&skill_file).ok()?;
            Some(Skill::builder(name)
                .description(format!("Skill: {}", name))
                .category(dir.parent()?.file_name()?.to_str()?.to_string())
                .instructions(content)
                .build())
        } else if yaml_file.exists() {
            let content = std::fs::read_to_string(&yaml_file).ok()?;
            serde_yaml::from_str(&content).ok()
        } else {
            // Use README as instructions
            let readme = skill_dir.join("README.md");
            if readme.exists() {
                let content = std::fs::read_to_string(&readme).ok()?;
                Some(Skill::builder(name)
                    .description(format!("Skill: {}", name))
                    .category(dir.parent()?.file_name()?.to_str()?.to_string())
                    .instructions(content)
                    .build())
            } else {
                Some(Skill::builder(name)
                    .description(format!("Skill: {}", name))
                    .category(dir.parent()?.file_name()?.to_str()?.to_string())
                    .instructions("(no instructions found)")
                    .build())
            }
        }
    }

    fn load_file(&self, path: &Path) -> Option<Skill> {
        let content = std::fs::read_to_string(path).ok()?;
        let name = path.file_stem()?.to_str()?;

        // Check if it's YAML
        if path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            serde_yaml::from_str(&content).ok()
        } else {
            Some(Skill::builder(name)
                .description(content.lines().next().unwrap_or("").to_string())
                .instructions(content)
                .build())
        }
    }
}

/// Default skills that come with the system.
pub fn builtin_skills() -> Vec<Skill> {
    vec![
        Skill::builder("general")
            .description("General-purpose conversation and task assistance")
            .category("core")
            .instructions("You are a helpful AI assistant. Help the user accomplish their goals efficiently and accurately.")
            .enabled(true)
            .auto_use(true)
            .build(),
    ]
}
