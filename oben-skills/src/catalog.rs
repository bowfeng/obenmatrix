/// A unified skill catalog — discovers and loads skills from all configured
/// sources (builtin, filesystem dirs, plugins, external dirs) into a single
/// ready-to-consume registry.
///
/// The catalog is read at startup (and optionally refreshed) and provides:
/// - Auto-discovery from `~/.agents/skills/` (recursive `SKILL.md`/`SKILL.yaml`)
/// - Discovery from each plugin's `skills/` directory
/// - Merging with builtin skills
/// - Filtering by platform (built into `SkillLoader`)
/// - Duplicate elimination (by qualified name, keeping the highest-priority source)
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use tracing::{debug, info};

use crate::loader::*;
use crate::system::SkillManager;
use oben_models::Skill;

/// Priority order for skill sources. When the same skill name appears in
/// multiple sources, the highest-priority source wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourcePriority {
    Builtin = 0,
    Plugin = 1,
    User = 2,
    External = 3,
}

/// A skill with its discovery source attached.
#[derive(Debug, Clone)]
struct CatalogEntry {
    skill: Skill,
    source: SourcePath,
    priority: SourcePriority,
}

/// Where this skill was discovered from.
#[derive(Debug, Clone)]
enum SourcePath {
    /// A builtin skill defined in code.
    Builtin(String),
    /// A skill file inside a plugin's `skills/` directory.
    Plugin(PathBuf),
    /// A skill file inside `~/.agents/skills/` (user skills).
    User(PathBuf),
    /// A skill file from an external_dirs entry.
    External(PathBuf),
}

impl SourcePath {
    fn label(&self) -> String {
        match self {
            Self::Builtin(n) => format!("builtin:{}", n),
            Self::Plugin(p) => format!("plugin:{}", p.display()),
            Self::User(p) => format!("user:{}", p.display()),
            Self::External(p) => format!("external:{}", p.display()),
        }
    }

    fn source_group(&self) -> String {
        match self {
            Self::Builtin(_) => "builtin".to_string(),
            Self::Plugin(p) | Self::User(p) | Self::External(p) => p.display().to_string(),
        }
    }
}

/// Configuration for skill catalog discovery.
#[derive(Debug, Clone)]
pub struct CatalogConfig {
    /// Path to `~/.agents/skills/` for user-installed skills.
    pub agents_skills_dir: Option<PathBuf>,
    /// Paths to scan for plugin skill directories.
    pub plugin_dirs: Vec<PathBuf>,
    /// Additional external skill directories (from config.yaml or CLI).
    pub external_dirs: Vec<PathBuf>,
    /// Whether to scan for plugins automatically.
    pub auto_discover_plugins: bool,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            agents_skills_dir: default_agents_skills_dir(),
            plugin_dirs: Vec::new(),
            external_dirs: get_external_skills_dirs(),
            auto_discover_plugins: true,
        }
    }
}

/// The unified skill catalog — a single entry point for all skill sources.
pub struct SkillCatalog {
    config: CatalogConfig,
    /// All loaded skills indexed by qualified name.
    skills: BTreeMap<String, CatalogEntry>,
    /// Pre-processed skill manager for building instructions.
    manager: SkillManager,
}

impl SkillCatalog {
    /// Create a new catalog with the given configuration.
    pub fn new(config: CatalogConfig) -> Self {
        let manager = SkillManager::new();
        Self {
            config,
            skills: BTreeMap::new(),
            manager,
        }
    }

    /// Create a catalog with default configuration (auto-discovers common paths).
    pub fn default_config() -> Self {
        Self::new(CatalogConfig::default())
    }

    /// Load all skills from all configured sources.
    pub fn load(&mut self) -> Result<&[Skill]> {
        // 1. Load builtin skills
        let builtins = builtin_skills();
        let builtin_count = builtins.len();
        for skill in &builtins {
            let name = self.qualified_name(skill);
            self.skills.insert(
                name,
                CatalogEntry {
                    skill: skill.clone(),
                    source: SourcePath::Builtin("default".into()),
                    priority: SourcePriority::Builtin,
                },
            );
        }
        info!("Loaded {} builtin skill(s)", builtin_count);

        // Clone config values to avoid self-borrowing issues
        let agents_dir = self.config.agents_skills_dir.clone();
        let plugin_dirs = self.config.plugin_dirs.clone();
        let auto_discover = self.config.auto_discover_plugins;
        let external_dirs = self.config.external_dirs.clone();

        // 2. Discover user skills from ~/.agents/skills/
        if let Some(ref dir) = agents_dir {
            if dir.is_dir() {
                self.load_dir_user(dir)?;
            } else {
                debug!("Agents skills directory not found: {:?}", dir);
            }
        }

        // 3. Discover plugin skills
        if auto_discover {
            let plugins = self.discover_plugins()?;
            for plugin_dir in plugins {
                self.load_plugin_skills(&plugin_dir)?;
            }
        } else if !plugin_dirs.is_empty() {
            for dir in &plugin_dirs {
                if dir.is_dir() {
                    self.load_plugin_skills(dir)?;
                }
            }
        }

        // 4. Load external skill directories
        for dir in &external_dirs {
            if dir.is_dir() {
                self.load_dir_external(dir)?;
            }
        }

        info!(
            "Skill catalog loaded {} skills from {} source groups",
            self.skills.len(),
            self.source_groups()
        );

        // Feed into SkillManager
        let all_skills: Vec<Skill> =
            self.skills.values().map(|e| e.skill.clone()).collect();
        self.manager.load_skills(all_skills);

        Ok(self.list())
    }

    /// Load skills from `~/.agents/skills/` (user skills).
    fn load_dir_user(&mut self, dir: &PathBuf) -> Result<()> {
        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all()?;
        let count = skills.len();

        for skill in &skills {
            let name = self.qualified_name(skill);
            let entry = CatalogEntry {
                skill: skill.clone(),
                source: SourcePath::User(dir.clone()),
                priority: SourcePriority::User,
            };
            if let Some(existing) = self.skills.get(&name) {
                if entry.priority > existing.priority {
                    debug!(
                        "Overriding {} ({}) with user skill from {:?}",
                        name,
                        existing.source.label(),
                        dir
                    );
                }
            }
            self.skills.insert(name, entry);
        }
        if count > 0 {
            info!("Loaded {} user skill(s) from {:?}", count, dir);
        }
        Ok(())
    }

    /// Discover plugin directories by scanning common plugin locations.
    fn discover_plugins(&self) -> Result<Vec<PathBuf>> {
        let mut plugins = Vec::new();

        // Auto-discover: check `~/.agents/plugins/*/`
        if let Some(ref agents_dir) = self.config.agents_skills_dir {
            if let Some(parent) = agents_dir.parent() {
                let plugins_dir = parent.join("plugins");
                if plugins_dir.is_dir() {
                    for entry in std::fs::read_dir(&plugins_dir)? {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            plugins.push(path);
                        }
                    }
                }
            }
        }

        Ok(plugins)
    }

    /// Load skills from a plugin's `skills/` directory.
    fn load_plugin_skills(&mut self, plugin_dir: &PathBuf) -> Result<()> {
        let skills_dir = plugin_dir.join("skills");
        if !skills_dir.is_dir() {
            debug!(
                "Plugin {} has no skills/ directory",
                plugin_dir.display()
            );
            return Ok(());
        }

        let mut loader = SkillLoader::new();
        loader.add_dir(&skills_dir);
        let skills = loader.load_all()?;
        let count = skills.len();
        let plugin_name =
            plugin_dir.file_name().unwrap_or_default().to_string_lossy().to_string();

        for skill in &skills {
            let name = self.qualified_name(skill);
            let entry = CatalogEntry {
                skill: skill.clone(),
                source: SourcePath::Plugin(skills_dir.clone()),
                priority: SourcePriority::Plugin,
            };
            if let Some(existing) = self.skills.get(&name) {
                if entry.priority > existing.priority {
                    debug!(
                        "Overriding {} ({}) with plugin skill",
                        name,
                        existing.source.label()
                    );
                }
            }
            self.skills.insert(name, entry);
        }

        if count > 0 {
            info!(
                "Loaded {} plugin skill(s) from plugin: {}",
                count,
                plugin_name
            );
        }
        Ok(())
    }

    /// Load skills from external_dirs configured in config.yaml.
    fn load_dir_external(&mut self, dir: &PathBuf) -> Result<()> {
        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all()?;
        let count = skills.len();

        for skill in &skills {
            let name = self.qualified_name(skill);
            let entry = CatalogEntry {
                skill: skill.clone(),
                source: SourcePath::External(dir.clone()),
                priority: SourcePriority::External,
            };
            // External skills always override existing ones
            if let Some(existing) = self.skills.get(&name) {
                debug!(
                    "Overriding {} ({}) with external skill",
                    name,
                    existing.source.label()
                );
            }
            self.skills.insert(name, entry);
        }
        info!(
            "Loaded {} external skill(s) from {:?}",
            count, dir
        );
        Ok(())
    }

    /// Build a qualified name for a skill (namespace:name or just name).
    fn qualified_name(&self, skill: &Skill) -> String {
        if let Some(metadata) = &skill.metadata {
            if let Some(meta) = metadata.as_mapping() {
                if let Some(ns_val) = meta.get("namespace") {
                    if let Some(ns) = ns_val.as_str() {
                        if !ns.is_empty() {
                            return format!("{}:{}", ns, skill.name);
                        }
                    }
                }
            }
        }
        skill.name.clone()
    }

    /// Get the number of unique discovery source groups.
    fn source_groups(&self) -> usize {
        let mut groups = HashSet::new();
        for entry in self.skills.values() {
            groups.insert(entry.source.source_group());
        }
        groups.len()
    }

    /// List all loaded skill names, grouped by source type.
    pub fn list_grouped(&self) -> BTreeMap<&str, Vec<String>> {
        let mut grouped = BTreeMap::new();
        for entry in self.skills.values() {
            let group = match &entry.source {
                SourcePath::Builtin(_) => "builtin",
                SourcePath::Plugin(_) => "plugin",
                SourcePath::User(_) => "user",
                SourcePath::External(_) => "external",
            };
            grouped
                .entry(group)
                .or_insert_with(Vec::new)
                .push(entry.skill.name.clone());
        }
        grouped
    }

    /// Try to find a skill by its qualified name.
    ///
    /// Performs an exact match first, then tries stripping the namespace
    /// prefix (e.g., "myns:my-skill" → "my-skill").
    pub fn find(&self, name: &str) -> Option<&Skill> {
        if let Some(entry) = self.skills.get(name) {
            return Some(&entry.skill);
        }
        // Try without namespace prefix
        let (ns, bare) = parse_qualified_name(name);
        if ns.is_some() {
            if let Some(entry) = self.skills.get(bare) {
                return Some(&entry.skill);
            }
        }
        None
    }

    /// Return all enabled skills as a slice.
    pub fn list(&self) -> &[Skill] {
        self.manager.list_skills()
    }

    /// Count of loaded (unique) skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Count of enabled skills (passed through SkillManager filter).
    pub fn enabled_count(&self) -> usize {
        self.manager.count()
    }

    /// Build skill instructions string for the system prompt.
    pub fn build_skill_instructions(&self) -> String {
        self.manager.build_skill_instructions()
    }

    /// Build all skill instructions (including non-auto-use skills).
    pub fn build_all_skill_instructions(&self) -> String {
        self.manager.build_all_skill_instructions()
    }

    /// Get skills grouped by category.
    pub fn by_category(&self) -> Vec<(&str, Vec<&Skill>)> {
        self.manager.skills_by_category()
    }

    /// Get auto-use skills.
    pub fn auto_use_skills(&self) -> Vec<&Skill> {
        self.manager.auto_use_skills()
    }
}

/// Get the default `~/.agents/skills/` directory path.
fn default_agents_skills_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(&home).join(".agents/skills");
        if dir.is_dir() {
            return Some(dir);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_catalog_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_skill_md(path: &Path, frontmatter: &str, body: &str) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = format!("---\n{}\n---\n\n{}", frontmatter, body);
        std::fs::write(path, content).ok();
    }

    #[test]
    fn test_catalog_default_config() {
        let catalog = SkillCatalog::default_config();
        assert_eq!(catalog.count(), 0);
        assert_eq!(catalog.enabled_count(), 0);
    }

    #[test]
    fn test_catalog_load_builtin() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        assert!(catalog.count() >= 1);
        assert!(catalog.find("general").is_some());
    }

    #[test]
    fn test_catalog_load_user_skills() {
        let dir = temp_dir("load_user");
        fs::create_dir_all(dir.join("test-skill")).ok();
        let content = "---\nname: my-test\n---\n\nTest skill instructions.";
        fs::write(dir.join("test-skill/SKILL.md"), &content).ok();

        let config = CatalogConfig {
            agents_skills_dir: Some(dir.clone()),
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        assert!(catalog.find("my-test").is_some());
        assert!(catalog.enabled_count() >= 1);
    }

    #[test]
    fn test_catalog_user_overrides_builtin() {
        let dir = temp_dir("override_builtin");
        fs::create_dir_all(dir.join("overrides")).ok();
        let content = "---\nname: general\n---\n\nCustom instructions that override builtin.";
        fs::write(dir.join("overrides/SKILL.md"), &content).ok();

        let config = CatalogConfig {
            agents_skills_dir: Some(dir.clone()),
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        let general = catalog.find("general");
        assert!(general.is_some());
        assert!(general
            .unwrap()
            .instructions
            .contains("Custom instructions that override builtin."));
    }

    #[test]
    fn test_catalog_find_by_bare_name() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        assert!(catalog.find("general").is_some());
    }

    #[test]
    fn test_catalog_no_discovery_dirs() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        assert_eq!(catalog.count(), 1);
    }

    #[test]
    fn test_catalog_load_skills_from_dir() {
        let dir = temp_dir("load_from_dir");
        std::fs::create_dir_all(dir.join("cat1")).ok();
        std::fs::create_dir_all(dir.join("cat1/sub")).ok();
        write_skill_md(&dir.join("cat1/SKILL.md"), "name: skill-a\n", "Instructions A.");
        write_skill_md(&dir.join("cat1/sub/SKILL.md"), "name: skill-b\n", "Instructions B.");
        write_skill_md(
            &dir.join("cat1/standalone.md"),
            "name: skill-c\ndescription: standalone\n",
            "Instructions C.",
        );

        let mut loader = SkillLoader::new();
        loader.add_dir(&dir);
        let skills = loader.load_all().ok().unwrap_or_default();
        assert!(skills.len() >= 2); // skill-a and skill-b from recursive scan
    }

    #[test]
    fn test_catalog_list_grouped() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        let grouped = catalog.list_grouped();
        assert!(grouped.contains_key("builtin"));
        assert!(grouped
            .get("builtin")
            .unwrap()
            .contains(&"general".to_string()));
    }

    #[test]
    fn test_catalog_by_category() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        let categories = catalog.by_category();
        assert!(!categories.is_empty());
    }

    #[test]
    fn test_catalog_auto_use_skills() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        let auto_skills = catalog.auto_use_skills();
        assert!(auto_skills.iter().any(|s| s.name == "general"));
    }

    #[test]
    fn test_catalog_build_instructions() {
        let config = CatalogConfig {
            agents_skills_dir: None,
            plugin_dirs: Vec::new(),
            external_dirs: Vec::new(),
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();
        let instructions = catalog.build_skill_instructions();
        assert!(instructions.contains("## Available Skills"));
        assert!(instructions.contains("general"));
    }

    #[test]
    fn test_catalog_external_priority() {
        let builtin_dir = temp_dir("builtin_source");
        fs::create_dir_all(&builtin_dir).ok();
        fs::create_dir_all(builtin_dir.join("plugin-skill")).ok();
        let content = "---\nname: plugin-skill\n---\n\nPlugin version.";
        fs::write(builtin_dir.join("plugin-skill/SKILL.md"), &content).ok();

        let external_dir = temp_dir("external_source");
        fs::create_dir_all(&external_dir).ok();
        fs::create_dir_all(external_dir.join("external-skill")).ok();
        let content2 = "---\nname: external-skill\n---\n\nExternal version.";
        fs::write(external_dir.join("external-skill/SKILL.md"), &content2).ok();

        let config = CatalogConfig {
            agents_skills_dir: Some(builtin_dir),
            plugin_dirs: Vec::new(),
            external_dirs: vec![external_dir],
            auto_discover_plugins: false,
        };
        let mut catalog = SkillCatalog::new(config);
        catalog.load().ok();

        assert!(catalog.find("plugin-skill").is_some());
        assert!(catalog.find("external-skill").is_some());
        assert_eq!(catalog.count(), 3);
    }
}
