/// Load skills from YAML/TXT files.
/// Maps to how Hermes loads skill files from the `skills/` directory.

use anyhow::Result;
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
    pub fn load_all(&self) -> Result<Vec<oben_models::Skill>> {
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

    fn load_from_dir(&self, dir: &Path) -> Option<oben_models::Skill> {
        let name = dir.file_name()?.to_str()?;
        let skill_dir = dir;

        // Try to find a SKILL.md or skill.yaml
        let skill_file = skill_dir.join("SKILL.md");
        let yaml_file = skill_dir.join("SKILL.yaml");

        if skill_file.exists() {
            let content = std::fs::read_to_string(&skill_file).ok()?;
            Some(oben_models::Skill::builder(name)
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
                Some(oben_models::Skill::builder(name)
                    .description(format!("Skill: {}", name))
                    .category(dir.parent()?.file_name()?.to_str()?.to_string())
                    .instructions(content)
                    .build())
            } else {
                Some(oben_models::Skill::builder(name)
                    .description(format!("Skill: {}", name))
                    .category(dir.parent()?.file_name()?.to_str()?.to_string())
                    .instructions("(no instructions found)")
                    .build())
            }
        }
    }

    fn load_file(&self, path: &Path) -> Option<oben_models::Skill> {
        let content = std::fs::read_to_string(path).ok()?;
        let name = path.file_stem()?.to_str()?;

        // Check if it's YAML
        if path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            serde_yaml::from_str(&content).ok()
        } else {
            Some(oben_models::Skill::builder(name)
                .description(content.lines().next().unwrap_or("").to_string())
                .instructions(content)
                .build())
        }
    }
}

/// Default skills that come with the system.
pub fn builtin_skills() -> Vec<oben_models::Skill> {
    vec![
        oben_models::Skill::builder("general")
            .description("General-purpose conversation and task assistance")
            .category("core")
            .instructions("You are a helpful AI assistant. Help the user accomplish their goals efficiently and accurately.")
            .enabled(true)
            .auto_use(true)
            .build(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_skill_loader_new_is_empty() {
        let loader = SkillLoader::new();
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_skill_loader_txt_file() {
        let dir = temp_dir("txt_skill");
        fs::write(dir.join("test-skill.txt"), "This is a skill description.").unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test-skill");
        assert_eq!(skills[0].description, "This is a skill description.");
        assert_eq!(skills[0].instructions, "This is a skill description.");
    }

    #[test]
    fn test_skill_loader_md_file() {
        let dir = temp_dir("md_skill");
        fs::write(dir.join("test-skill.md"), "# Skill\nInstructions here.").unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test-skill");
        assert!(skills[0].instructions.contains("# Skill"));
    }

    #[test]
    fn test_skill_loader_txt_no_instructions() {
        let dir = temp_dir("txt_no_inst");
        fs::write(dir.join("test-skill.txt"), "").unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test-skill");
        assert_eq!(skills[0].description, "");
    }

    #[test]
    fn test_skill_loader_skips_non_skill_files() {
        let dir = temp_dir("skip_files");
        fs::write(dir.join("data.json"), "{}" ).unwrap();
        fs::write(dir.join("readme.md"), "readme").unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        // Only .md file should be loaded
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "readme");
    }

    #[test]
    fn test_skill_loader_loads_directories_as_skills() {
        // Directories are also loaded as skills (via SKILL.md / SKILL.yaml / README.md)
        let dir = temp_dir("load_dirs");
        fs::create_dir(dir.join("subdir")).unwrap();
        fs::write(dir.join("skill.md"), "instructions").unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        // skill.md -> "skill" skill, subdir directory -> "subdir" skill (no instructions)
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_skill_loader_loads_txt_with_long_content() {
        let dir = temp_dir("txt_long");
        let long_content = "First line.\nSecond line of instructions.";
        fs::write(dir.join("long-skill.txt"), long_content).unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "long-skill");
        assert_eq!(skills[0].description, "First line.");
        assert_eq!(skills[0].instructions, long_content);
    }

    #[test]
    fn test_skill_loader_nonexistent_dir() {
        let mut loader = SkillLoader::new();
        loader.add_dir("/tmp/nonexistent_dir_12345");
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_builtin_skills_returns_one() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "general");
        assert_eq!(skills[0].category, "core");
    }
}
