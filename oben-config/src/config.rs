use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use oben_models::ProviderConfig;

/// All application settings, stored in ~/.config/obenalien/config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
    pub providers: Vec<ProviderConfig>,
    pub custom_providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub enabled: Vec<String>,
    /// Auto-enable tools by category.
    #[serde(default)]
    pub auto_detect: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            auto_detect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    pub enabled: Vec<String>,
    pub auto_use: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            auto_use: vec![],
        }
    }
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

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            spinner_style: "dots".to_string(),
            theme: "dark".to_string(),
            code_block_language: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    /// Max messages to keep in context before compression.
    #[serde(default)]
    pub max_messages: Option<usize>,
    /// Compression method: "summary", "token_count", "none".
    #[serde(default = "default_compression")]
    pub compression: String,
    /// Total context window size in tokens (default: 128000).
    #[serde(default = "default_context_length")]
    pub context_length: usize,
    /// Token threshold as percentage of context_length for compaction (default: 0.75 = 75%).
    #[serde(default = "default_threshold_percent")]
    pub threshold_percent: f64,
}

fn default_compression() -> String {
    "summary".to_string()
}

fn default_context_length() -> usize {
    128_000
}

fn default_threshold_percent() -> f64 {
    0.75
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_messages: Some(100),
            compression: "summary".to_string(),
            context_length: default_context_length(),
            threshold_percent: default_threshold_percent(),
        }
    }
}

impl ContextConfig {}

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
                context_length: 128_000,
                threshold_percent: 0.75,
            },
            providers: Vec::new(),
            custom_providers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults;
    use oben_models::ProviderKind;
    use std::fs;

    #[test]
    fn test_default_model_is_openrouter_qwen() {
        let config = AppConfig::default();
        assert_eq!(config.model.kind, ProviderKind::OpenRouter);
        assert_eq!(config.model.model, "qwen/qwen3-235b:free");
        assert_eq!(config.model.default_model, "qwen/qwen3-235b:free");
        assert!(config.model.api_key.is_none());
    }

    #[test]
    fn test_default_settings() {
        let config = AppConfig::default();
        assert_eq!(config.temperature, Some(0.7));
        assert_eq!(config.max_tokens, Some(8192));
        assert_eq!(config.max_iterations, Some(50));
        assert!(config.tools.auto_detect);
        assert_eq!(config.display.theme, "dark");
        assert_eq!(config.context.compression, "summary");
        assert_eq!(config.context.max_messages, Some(100));
    }

    #[test]
    fn test_default_system_prompt_not_empty() {
        let prompt = defaults::default_system_prompt();
        assert!(!prompt.trim().is_empty());
        assert!(prompt.contains("AI agent"));
        assert!(prompt.contains("tools"));
    }

    #[test]
    fn test_config_yaml_roundtrip() {
        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        // Compare core fields
        assert_eq!(restored.model.kind, config.model.kind);
        assert_eq!(restored.model.model, config.model.model);
        assert_eq!(restored.temperature, config.temperature);
        assert_eq!(restored.max_tokens, config.max_tokens);
        assert_eq!(restored.max_iterations, config.max_iterations);
        assert_eq!(restored.tools.auto_detect, config.tools.auto_detect);
        assert_eq!(restored.display.theme, config.display.theme);
        assert_eq!(restored.context.compression, config.context.compression);
    }

    #[test]
    fn test_config_yaml_roundtrip_with_gateway() {
        let mut config = AppConfig::default();
        config.gateway = Some(GatewayConfig {
            telegram: Some(PlatformConfig {
                enabled: true,
                token: Some("tg-secret-token".to_string()),
            }),
            discord: None,
            slack: None,
            whatsapp: None,
        });
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(restored.gateway.is_some());
        let gw = restored.gateway.unwrap();
        let tg = gw.telegram.unwrap();
        assert!(tg.enabled);
        assert_eq!(tg.token, Some("tg-secret-token".to_string()));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");

        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        fs::write(&config_path, &yaml).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&content).unwrap();
        assert_eq!(restored.model.kind, config.model.kind);
        assert_eq!(restored.temperature, config.temperature);
    }

    #[test]
    fn test_minimal_config_deserialize() {
        // User's actual config only has model settings.
        // All other fields should use defaults.
        let yaml = r#"model:
  kind: Custom
  base_url: "http://10.0.0.177:8000/v1"
  model: "qwen35-local"
  default_model: "qwen35-local"
  api_key: "sk-local"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.kind, oben_models::ProviderKind::Custom);
        assert_eq!(config.model.model, "qwen35-local");
        assert_eq!(
            config.model.base_url.as_deref(),
            Some("http://10.0.0.177:8000/v1")
        );
        assert_eq!(config.tools.enabled, Vec::<String>::new());
        assert!(config.tools.auto_detect);
        assert_eq!(config.display.theme, "dark");
        assert_eq!(config.context.compression, "summary");
    }

    #[test]
    fn test_providers_field_serializes_empty() {
        let config = AppConfig::default();
        assert!(config.providers.is_empty());
        assert!(config.custom_providers.is_empty());
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(restored.providers.is_empty());
        assert!(restored.custom_providers.is_empty());
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|d| d.join("oben"))
            .unwrap_or_else(|| PathBuf::from("~/.config/obenalien"))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.yaml")
    }

    /// Read from `~/.obenalien/config.yaml` (legacy/standard path).
    pub fn config_dir_legacy() -> PathBuf {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("~"));
        home.join(".config/obenalien")
    }

    /// Read from `~/.config/obenalien/config.yaml` (legacy/standard path).
    pub fn config_path_legacy() -> PathBuf {
        Self::config_dir_legacy().join("config.yaml")
    }

    /// Load config from `~/.config/obenalien/config.yaml`.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path_legacy();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut config: Self = serde_yaml::from_str(&content)?;

        if let Some(t) = config.temperature {
            config.model.temperature = Some(t);
        }
        if let Some(m) = config.max_tokens {
            config.model.max_tokens = Some(m);
        }

        if config
            .model
            .api_key
            .as_ref()
            .map_or(true, |v| v.trim().is_empty())
        {
            if let Some(key) =
                oben_models::provider_registry::resolve_api_key_from_env(config.model.kind.as_str())
            {
                config.model.api_key = Some(key);
            }
        }

        if config
            .model
            .base_url
            .as_ref()
            .map_or(true, |v| v.trim().is_empty())
        {
            if let Some(env_var) = config.model.kind.base_url_env_var() {
                if let Ok(val) = std::env::var(env_var) {
                    let val = val.trim().to_string();
                    if !val.is_empty() {
                        config.model.base_url = Some(val);
                    }
                }
            }
        }

        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir_legacy();
        std::fs::create_dir_all(&dir)?;
        let content = serde_yaml::to_string(self)?;
        std::fs::write(dir.join("config.yaml"), content)?;
        Ok(())
    }
}
