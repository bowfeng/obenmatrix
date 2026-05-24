//! Plugin configuration — enabled/disabled lists, load gating.
//!
//! Maps to Hermes' `plugins.enabled` and `plugins.disabled` config
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
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Explicit allow list. None or empty = all pass (except disabled).
    /// If non-empty, only listed plugins are loaded.
    pub enabled: Option<Vec<String>>,

    /// Always blocked, regardless of enabled list.
    pub disabled: Vec<String>,
}

impl PluginConfig {
    /// Parse plugin config from a TOML/YAML file.
    ///
    /// Looks for a `plugins:` section in the config file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Cannot read config file {}: {}", path.display(), e))?;

        // Parse as YAML
        if let Ok(map) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
            if let Some(plugins) = map.get("plugins") {
                let config: Result<Self, _> = serde_yaml::from_value(plugins.clone());
                return config.map_err(|e| anyhow::anyhow!("Failed to parse plugins config: {}", e));
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
        /// given: YAML config with plugins section
        /// when: PluginConfig::from_file() is called
        /// then: parsed config has correct enabled/disabled lists
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        fs::write(
            &config_path,
            r#"
name: test-agent
plugins:
  enabled: ["plugin-a", "plugin-b"]
  disabled: ["broken-plugin"]
"#,
        )
        .unwrap();

        let config = PluginConfig::from_file(&config_path).unwrap();
        assert_eq!(config.enabled, Some(vec!["plugin-a".into(), "plugin-b".into()]));
        assert_eq!(config.disabled, vec!["broken-plugin".to_string()]);
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
