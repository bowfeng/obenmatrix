use serde::{Deserialize, Serialize};

/// A skill definition — instructions the agent loads to extend behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub category: String,
    pub instructions: String,
    pub enabled: bool,
    pub auto_use: bool,
    pub pinned: bool,
    pub environments: Vec<String>,
    /// Arbitrary metadata from YAML frontmatter (platforms, tags, config vars, etc.).
    pub metadata: Option<serde_yaml::Value>,
}

impl Skill {
    pub fn builder(name: impl Into<String>) -> SkillBuilder {
        SkillBuilder {
            name: name.into(),
            description: String::new(),
            category: String::new(),
            instructions: String::new(),
            enabled: true,
            auto_use: false,
            pinned: false,
            environments: Vec::new(),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillBuilder {
    name: String,
    description: String,
    category: String,
    instructions: String,
    enabled: bool,
    auto_use: bool,
    pinned: bool,
    environments: Vec<String>,
    metadata: Option<serde_yaml::Value>,
}

impl SkillBuilder {
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn category(mut self, cat: impl Into<String>) -> Self {
        self.category = cat.into();
        self
    }

    pub fn instructions(mut self, inst: impl Into<String>) -> Self {
        self.instructions = inst.into();
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn auto_use(mut self, auto: bool) -> Self {
        self.auto_use = auto;
        self
    }

    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn environments(mut self, envs: Vec<String>) -> Self {
        self.environments = envs;
        self
    }

    pub fn metadata(mut self, meta: Option<serde_yaml::Value>) -> Self {
        self.metadata = meta;
        self
    }

    pub fn build(self) -> Skill {
        Skill {
            name: self.name,
            description: self.description,
            category: self.category,
            instructions: self.instructions,
            enabled: self.enabled,
            auto_use: self.auto_use,
            pinned: self.pinned,
            environments: self.environments,
            metadata: self.metadata,
        }
    }
}
