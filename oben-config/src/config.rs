use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use oben_models::ProviderConfig;

/// All application settings, stored in ~/.oben/config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub model: ProviderConfig,
    pub temperature: Option<f64>,
    pub max_tokens: Option<usize>,
    pub max_iterations: Option<usize>,
    pub tools: ToolsConfig,
    pub skills: SkillsConfig,
    pub gateway: Option<GatewayConfig>,
    pub display: DisplayConfig,
    pub context: ContextConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub enabled: Vec<String>,
    /// Auto-enable tools by category.
    pub auto_detect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    pub enabled: Vec<String>,
    pub auto_use: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub telegram: Option<PlatformConfig>,
    pub discord: Option<PlatformConfig>,
    pub slack: Option<PlatformConfig>,
    pub whatsapp: Option<PlatformConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub enabled: bool,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub spinner_style: String,
    pub theme: String,
    pub code_block_language: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Max messages to keep in context before compression.
    pub max_messages: Option<usize>,
    /// Compression method: "summary", "token_count", "none".
    pub compression: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model: ProviderConfig::new(
                oben_models::ProviderKind::OpenRouter,
                "qwen/qwen3-235b:free",
            ),
            temperature: Some(0.7),
            max_tokens: Some(8192),
            max_iterations: Some(50),
            tools: ToolsConfig {
                enabled: vec![],
                auto_detect: true,
            },
            skills: SkillsConfig {
                enabled: vec![],
                auto_use: vec![],
            },
            gateway: None,
            display: DisplayConfig {
                spinner_style: "dots".to_string(),
                theme: "dark".to_string(),
                code_block_language: true,
            },
            context: ContextConfig {
                max_messages: Some(100),
                compression: "summary".to_string(),
            },
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|d| d.join("oben"))
            .unwrap_or_else(|| PathBuf::from("~/.oben"))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.yaml")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let content = serde_yaml::to_string(self)?;
        std::fs::write(dir.join("config.yaml"), content)?;
        Ok(())
    }
}
