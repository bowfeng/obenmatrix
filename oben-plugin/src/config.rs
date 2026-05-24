//! Plugin configuration — enabled/disabled lists, provider selection, load gating.
//!
//! Maps to Hermes' `plugins.enabled`, `plugins.disabled`, and provider config
/// values in config.yaml.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::fs;

/// Plugin configuration from config.yaml.
///
/// Controls which plugins are loaded and how they are gated.
/// ```yaml
/// plugins:
///   enabled: []        # empty = auto-load only; list = explicit allow
///   disabled: []       # always blocked
/// image_gen:
///   provider: "mock"   # which provider to use
/// web_search:
///   provider: "tavily"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Explicit allow list. None or empty = all pass (except disabled).
    /// If non-empty, only listed plugins are loaded.
    pub enabled: Option<Vec<String>>,

    /// Always blocked, regardless of enabled list.
    pub disabled: Vec<String>,

    /// Provider selection by category. Keys like "image_gen", "web_search",
    /// "browser", etc. Values are provider names to use.
    #[serde(default)]
    pub providers: std::collections::HashMap<String, String>,
}

impl PluginConfig {
    /// Parse plugin config from a TOML/YAML file.
    ///
    /// Looks for `plugins:` and `providers:` sections in the config file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Cannot read config file {}: {}", path.display(), e))?;

        // Parse as full YAML with plugins + providers sections
        if let Ok(map) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
            // Build a config with plugins + providers sections
            let mut config = Self::default();
            
            // Parse plugins section
            if let Some(plugins) = map.get("plugins") {
                if let Some(plugins_map) = plugins.as_mapping() {
                    for (k, v) in plugins_map {
                        let key = k.as_str().unwrap_or("");
                        if key == "enabled" {
                            if let Some(enabled) = v.as_sequence() {
                                config.enabled = Some(enabled.iter()
                                    .filter_map(|val| val.as_str().map(String::from))
                                    .collect());
                            }
                        } else if key == "disabled" {
                            if let Some(disabled) = v.as_sequence() {
                                config.disabled = disabled.iter()
                                    .filter_map(|val| val.as_str().map(String::from))
                                    .collect();
                            }
                        }
                    }
                }
            }
            
            // Parse providers section
            if let Some(providers) = map.get("providers") {
                if let Some(providers_map) = providers.as_mapping() {
                    for (k, v) in providers_map {
                        if let (Some(key), Some(val)) = (k.as_str(), v.as_str()) {
                            config.providers.insert(key.to_string(), val.to_string());
                        }
                    }
                }
            }
            
            // Check if we parsed anything
            if config.enabled.is_some() || !config.disabled.is_empty() || !config.providers.is_empty() {
                return Ok(config);
            }
        }

        Ok(Self::default())
    }

    /// Check if a plugin is allowed to load.
    pub fn is_allowed(&self, plugin_key: &str, plugin_name: &str) -> bool {
        super::discovery::is_plugin_enabled(
            &crate::manifest::PluginManifest {
                name: plugin_name.to_string(),
                source: super::manifest::PluginSource::Bundled,
                key: plugin_key.to_string(),
                ..Default::default()
            },
            self.enabled.as_ref(),
            Some(&self.disabled),
        )
    }

    /// Get the configured provider name for a category (e.g. "image_gen").
    /// Returns None if no provider is configured for this category.
    pub fn get_provider(&self, category: &str) -> Option<&str> {
        self.providers.get(category).map(|s| s.as_str())
    }

    /// Check if a specific provider is configured for a category.
    pub fn has_provider(&self, category: &str, provider_name: &str) -> bool {
        self.providers.get(category).map(|s| s.as_str()) == Some(provider_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_config_default() {
        /// given: default PluginConfig
        /// when: PluginConfig::default() is called
        /// then: enabled is None, disabled is empty
        let config = PluginConfig::default();
        assert!(config.enabled.is_none());
        assert!(config.disabled.is_empty());
    }

    #[test]
    fn test_plugin_config_yaml_parse() {
        /// given: YAML config with plugins and providers sections
        /// when: PluginConfig::from_file() is called
        /// then: parsed config has correct enabled/disabled lists and provider selections
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        fs::write(
            &config_path,
            r#"
plugins:
  enabled: ["plugin-a", "plugin-b"]
  disabled: ["broken-plugin"]
providers:
  image_gen: mock
  web_search: tavily
"#,
        )
        .unwrap();

        let config = PluginConfig::from_file(&config_path).unwrap();
        assert_eq!(config.enabled, Some(vec!["plugin-a".into(), "plugin-b".into()]));
        assert_eq!(config.disabled, vec!["broken-plugin".to_string()]);
        assert_eq!(config.get_provider("image_gen"), Some("mock"));
        assert_eq!(config.get_provider("web_search"), Some("tavily"));
        assert_eq!(config.get_provider("browser"), None);
    }

    #[test]
    fn test_plugin_config_no_plugins_section() {
        /// given: YAML config without plugins section
        /// when: PluginConfig::from_file() is called
        /// then: returns default config
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        fs::write(
            &config_path,
            r#"
name: test-agent
other_setting: value
"#,
        )
        .unwrap();

        let config = PluginConfig::from_file(&config_path).unwrap();
        assert!(config.enabled.is_none());
        assert!(config.disabled.is_empty());
    }

    #[test]
    fn test_plugin_config_nonexistent_file() {
        /// given: nonexistent config file
        /// when: PluginConfig::from_file() is called
        /// then: returns error
        let path = Path::new("/nonexistent/config.yaml");
        let result = PluginConfig::from_file(path);
        assert!(result.is_err());
    }
}
