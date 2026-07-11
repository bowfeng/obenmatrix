/// Skill Commands - CLI commands from skills
/// 
/// Maps to `hermes-agent/skill_commands.py` functionality
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Skill command metadata
#[derive(Debug, Clone)]
pub struct CommandMetadata {
    /// Command name
    pub name: String,
    /// Command description
    pub description: String,
    /// Command usage
    pub usage: String,
    /// Skill that defines this command
    pub skill_name: String,
}

/// Skill commands manager
pub struct SkillCommands {
    commands: BTreeMap<String, CommandMetadata>,
    commands_dir: PathBuf,
}

impl SkillCommands {
    /// Create a new SkillCommands manager
    pub fn new(commands_dir: impl Into<PathBuf>) -> Self {
        Self {
            commands: BTreeMap::new(),
            commands_dir: commands_dir.into(),
        }
    }
    
    /// Load all skill commands from directory
    pub fn load_all(&mut self) -> Result<usize> {
        self.commands.clear();
        
        if !self.commands_dir.exists() {
            return Ok(0);
        }
        
        let mut count = 0;
        
        for entry in fs::read_dir(&self.commands_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                if let Some(metadata) = Self::parse_command_file(&entry.path())? {
                    self.commands.insert(metadata.name.clone(), metadata);
                    count += 1;
                }
            }
        }
        
        Ok(count)
    }
    
    /// Parse a command file
    fn parse_command_file(path: &Path) -> Result<Option<CommandMetadata>> {
        let content = fs::read_to_string(path)?;
        
        // Parse YAML frontmatter
        if !content.starts_with("---") {
            return Ok(None);
        }
        
        let rest = content.strip_prefix("---").ok_or_else(|| {
            anyhow::anyhow!("Failed to strip YAML frontmatter delimiter")
        })?;
        
        let end_pos = rest.find("---").ok_or_else(|| {
            anyhow::anyhow!("Failed to find end of YAML frontmatter")
        })?;
        
        let yaml_content = &rest[..end_pos];
        
        let mut name = String::new();
        let mut description = String::new();
        let mut usage = String::new();
        let mut skill_name = String::new();
        
        for line in yaml_content.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("name:") {
                name = trimmed
                    .strip_prefix("name:")
                    .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                    .unwrap_or_default()
                    .to_string();
            } else if trimmed.starts_with("description:") {
                description = trimmed
                    .strip_prefix("description:")
                    .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                    .unwrap_or_default()
                    .to_string();
            } else if trimmed.starts_with("usage:") {
                usage = trimmed
                    .strip_prefix("usage:")
                    .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                    .unwrap_or_default()
                    .to_string();
            } else if trimmed.starts_with("skill:") {
                skill_name = trimmed
                    .strip_prefix("skill:")
                    .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                    .unwrap_or_default()
                    .to_string();
            }
        }
        
        if name.is_empty() {
            return Ok(None);
        }
        
        // Extract skill name from file path if not in frontmatter
        if skill_name.is_empty() {
            skill_name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        
        Ok(Some(CommandMetadata {
            name,
            description,
            usage,
            skill_name,
        }))
    }
    
    /// Get a command by name
    pub fn get(&self, name: &str) -> Option<&CommandMetadata> {
        self.commands.get(name)
    }
    
    /// List all commands
    pub fn list(&self) -> Vec<&CommandMetadata> {
        self.commands.values().collect()
    }
    
    /// Check if a command exists
    pub fn exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }
    
    /// Reload commands from directory
    pub fn reload(&mut self) -> Result<usize> {
        self.load_all()
    }
    
    /// Get the number of loaded commands
    pub fn count(&self) -> usize {
        self.commands.len()
    }
}

impl Default for SkillCommands {
    fn default() -> Self {
        Self::new(std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".agents/commands"))
            .unwrap_or_else(|| PathBuf::from("./commands")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_commands_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_parse_command_file_valid() {
        let temp_dir = temp_dir("valid");
        let cmd_file = temp_dir.join("test-command");
        
        let content = r#"---
name: test-cmd
description: A test command
usage: test-cmd [options]
skill: test-skill
---

Command body"#;
        
        fs::write(&cmd_file, content).unwrap();
        
        let result = SkillCommands::parse_command_file(&cmd_file).unwrap();
        assert!(result.is_some());
        
        let cmd = result.unwrap();
        assert_eq!(cmd.name, "test-cmd");
        assert_eq!(cmd.description, "A test command");
        assert_eq!(cmd.usage, "test-cmd [options]");
        assert_eq!(cmd.skill_name, "test-skill");
    }

    #[test]
    fn test_parse_command_file_no_frontmatter() {
        let temp_dir = temp_dir("no_frontmatter");
        let cmd_file = temp_dir.join("test-command");
        fs::write(&cmd_file, "Just some text").unwrap();
        
        let result = SkillCommands::parse_command_file(&cmd_file).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_all_empty() {
        let temp_dir = temp_dir("load_empty");
        let mut commands = SkillCommands::new(&temp_dir);
        
        let count = commands.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_load_all_with_commands() {
        let temp_dir = temp_dir("load_with");
        let mut commands = SkillCommands::new(&temp_dir);
        
        // Create a command file
        let cmd_file = temp_dir.join("test-command");
        let content = "---\nname: test-cmd\ndescription: Test\n---\n";
        fs::write(&cmd_file, content).unwrap();
        
        let count = commands.load_all().unwrap();
        assert_eq!(count, 1);
        assert!(commands.exists("test-cmd"));
    }

    #[test]
    fn test_get_command() {
        let temp_dir = temp_dir("get");
        let mut commands = SkillCommands::new(&temp_dir);
        
        let cmd_file = temp_dir.join("test-cmd");
        let content = "---\nname: test-cmd\ndescription: Test\n---\n";
        fs::write(&cmd_file, content).unwrap();
        
        commands.load_all().unwrap();
        
        let cmd = commands.get("test-cmd");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().description, "Test");
    }

    #[test]
    fn test_list_commands() {
        let temp_dir = temp_dir("list");
        let mut commands = SkillCommands::new(&temp_dir);
        
        // Create multiple command files
        for i in 0..3 {
            let cmd_file = temp_dir.join(format!("cmd-{}", i));
            let content = format!("---\nname: cmd-{}\ndescription: Test {}\n---\n", i, i);
            fs::write(&cmd_file, content).unwrap();
        }
        
        let count = commands.load_all().unwrap();
        assert_eq!(count, 3);
        
        let list = commands.list();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_reload_commands() {
        let temp_dir = temp_dir("reload");
        let mut commands = SkillCommands::new(&temp_dir);
        
        // Initial load (empty)
        commands.load_all().unwrap();
        assert_eq!(commands.count(), 0);
        
        // Add a command
        let cmd_file = temp_dir.join("test-cmd");
        let content = "---\nname: test-cmd\ndescription: Test\n---\n";
        fs::write(&cmd_file, content).unwrap();
        
        // Reload
        let count = commands.reload().unwrap();
        assert_eq!(count, 1);
        assert!(commands.exists("test-cmd"));
    }
}
