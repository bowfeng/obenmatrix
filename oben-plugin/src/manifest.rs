/// PluginManifest — parsed from plugin.yaml manifest files.
///
/// Maps to Hermes' `PluginManifest` dataclass which represents a plugin's
/// `plugin.yaml` declaration.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::debug;

use crate::plugin_kind::{parse_plugin_kind, PluginKind};

/// Source where a plugin was discovered.
///
/// Later sources override earlier ones on name collision:
/// project > user > bundled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSource {
    /// Bundled with the agent repository (`<repo>/plugins/<name>/`).
    Bundled,
    /// User plugins (`~/.obenagent/plugins/<name>/`).
    User,
    /// Project plugins (`./.obenagent/plugins/<name>/`, opt-in).
    Project,
    /// Pip-installed plugins via entry points.
    Entrypoint,
}

impl Default for PluginSource {
    fn default() -> Self {
        PluginSource::Bundled
    }
}

impl std::fmt::Display for PluginSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginSource::Bundled => write!(f, "bundled"),
            PluginSource::User => write!(f, "user"),
            PluginSource::Project => write!(f, "project"),
            PluginSource::Entrypoint => write!(f, "entrypoint"),
        }
    }
}

impl PluginSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Bundled => "bundled",
            Self::User => "user",
            Self::Project => "project",
            Self::Entrypoint => "entrypoint",
        }
    }
}

/// Parsed representation of a `plugin.yaml` manifest.
///
/// Mirrors Hermes' `PluginManifest` dataclass:
/// - `name`: plugin name (from YAML or directory name)
/// - `version`, `description`, `author`: metadata
/// - `requires_env`: list of required environment variables
/// - `provides_tools`, `provides_hooks`: advertised capabilities
/// - `kind`: one of the 5 PluginKind variants
/// - `source`: where it was discovered
/// - `path`: filesystem path to plugin directory
/// - `key`: path-derived key for config lookups (e.g. "image_gen/openai")
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin name.
    pub name: String,

    /// Plugin version (e.g. "1.0.0").
    #[serde(default)]
    pub version: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: String,

    /// Plugin author(s).
    #[serde(default)]
    pub author: String,

    /// Environment variables that must be set for this plugin to work.
    #[serde(default)]
    pub requires_env: Vec<String>,

    /// Tool names this plugin provides.
    #[serde(default)]
    pub provides_tools: Vec<String>,

    /// Hook names this plugin provides callbacks for.
    #[serde(default)]
    pub provides_hooks: Vec<String>,

    /// Where this plugin was discovered.
    #[serde(default)]
    pub source: PluginSource,

    /// Filesystem path to the plugin directory.
    #[serde(default)]
    pub path: Option<String>,

    /// Plugin kind — controls loading behavior.
    #[serde(default = "default_kind")]
    pub kind: PluginKind,

    /// Registry key — path-derived key for config lookups.
    /// For flat plugin at `plugins/disk-cleanup/` key is `disk-cleanup`.
    /// For nested plugin at `plugins/image_gen/openai/` key is `image_gen/openai`.
    #[serde(default)]
    pub key: String,
}

fn default_kind() -> PluginKind {
    PluginKind::Standalone
}

impl PluginManifest {
    /// Parse a `plugin.yaml` from a plugin directory.
    pub fn from_yaml(plugin_dir: &Path, source: PluginSource) -> Result<Self> {
        let manifest_file = plugin_dir.join("plugin.yaml");
        let fallback = plugin_dir.join("plugin.yml");

        let path = if manifest_file.exists() {
            &manifest_file
        } else if fallback.exists() {
            &fallback
        } else {
            return Err(anyhow!("No plugin.yaml or plugin.yml found in {}", plugin_dir.display()));
        };

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let data: HashMap<String, serde_yaml::Value> =
            serde_yaml::from_str(&content).with_context(|| {
                format!("Failed to parse YAML in {}", path.display())
            })?;

        let name = data
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| plugin_dir.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();

        let raw_kind = data
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("standalone");

        let kind = parse_plugin_kind(raw_kind);

        // Auto-detect kind if not explicitly set
        let kind = if raw_kind.trim().to_lowercase() == "standalone"
            && !data.contains_key("kind")
        {
            Self::detect_kind(plugin_dir)
        } else {
            kind
        };

        let key = data
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or(&name);

        // Try to build path-derived key from directory structure
        let key = Self::derive_key_from_path(plugin_dir, key);

        Ok(Self {
            name: name.to_string(),
            version: data
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: data
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            author: data
                .get("author")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            requires_env: Self::parse_env_list(
                data.get("requires_env")
                    .and_then(|v| v.as_sequence())
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            ),
            provides_tools: Self::parse_string_list(
                data.get("provides_tools")
                    .and_then(|v| v.as_sequence()),
            ),
            provides_hooks: Self::parse_string_list(
                data.get("provides_hooks")
                    .and_then(|v| v.as_sequence()),
            ),
            source,
            path: Some(plugin_dir.display().to_string()),
            kind,
            key: key.to_string(),
        })
    }

    /// Auto-detect plugin kind from `__init__.py` content.
    fn detect_kind(plugin_dir: &Path) -> PluginKind {
        let init_file = plugin_dir.join("__init__.py");
        if !init_file.exists() {
            return PluginKind::Standalone;
        }

        if let Ok(content) = fs::read_to_string(&init_file) {
            let content = &content[..content.len().min(8192)];
            if content.contains("register_memory_provider")
                || content.contains("MemoryProvider")
            {
                debug!(
                    "Plugin {}: detected memory provider, treating as kind='exclusive'",
                    plugin_dir.display()
                );
                return PluginKind::Exclusive;
            }
            if content.contains("register_provider") && content.contains("ProviderProfile") {
                debug!(
                    "Plugin {}: detected model provider, treating as kind='model-provider'",
                    plugin_dir.display()
                );
                return PluginKind::ModelProvider;
            }
        }

        PluginKind::Standalone
    }

    /// Derive path-based key from directory structure.
    fn derive_key_from_path(plugin_dir: &Path, default_key: &str) -> String {
        // If the path contains multiple directory segments (category plugin),
        // use "category/name" format
        if let Some(parent) = plugin_dir.parent() {
            if let Some(category_dir) = parent.file_name() {
                let category = category_dir.to_string_lossy();
                // Skip hidden directories (e.g., temp dirs like .tmpXXX)
                if category.starts_with('.') {
                    return default_key.to_string();
                }
                // Check if the parent itself has a plugin.yaml (then it's a nested category)
                if !parent.join("plugin.yaml").exists() && !parent.join("plugin.yml").exists() {
                    let name = plugin_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    return format!("{}/{}", category, name);
                }
            }
        }
        default_key.to_string()
    }

    /// Parse a YAML sequence into a Vec<String>.
    fn parse_string_list(seq: Option<&Vec<serde_yaml::Value>>) -> Vec<String> {
        seq.map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
    }

    /// Parse env list (handles both list of strings and list of objects).
    fn parse_env_list(list: Vec<String>) -> Vec<String> {
        list
    }

    /// Check if required environment variables are available.
    pub fn has_required_env(&self) -> bool {
        self.requires_env
            .iter()
            .all(|env| std::env::var(env).is_ok())
    }

    /// Get the effective key for config lookups (falls back to name if key is empty).
    pub fn lookup_key(&self) -> &str {
        if self.key.is_empty() {
            &self.name
        } else {
            &self.key
        }
    }
}

/// Parse a plugin manifest from a YAML file path.
pub fn parse_manifest(path: &Path, source: PluginSource) -> Result<PluginManifest> {
    PluginManifest::from_yaml(path, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_manifest(dir: &Path, yaml_content: &str) {
        fs::write(dir.join("plugin.yaml"), yaml_content).unwrap();
    }

    #[test]
    fn test_parse_minimal_manifest() {
        /// given: a plugin directory with minimal plugin.yaml
        /// when: from_yaml() is called
        /// then: returns valid PluginManifest with defaults
        let dir = TempDir::new().unwrap();
        create_test_manifest(
            dir.path(),
            "name: test-plugin\n",
        );

        let manifest = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.kind, PluginKind::Standalone);
        assert_eq!(manifest.source, PluginSource::Bundled);
    }

    #[test]
    fn test_parse_full_manifest() {
        /// given: a plugin directory with full plugin.yaml
        /// when: from_yaml() is called
        /// then: all fields are correctly parsed
        let dir = TempDir::new().unwrap();
        create_test_manifest(
            dir.path(),
            r#"
name: test-plugin
version: "1.0.0"
description: A test plugin
author: Test Author
requires_env:
  - TEST_API_KEY
provides_tools:
  - test_tool
provides_hooks:
  - pre_tool_call
kind: standalone
"#,
        );

        let manifest = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description, "A test plugin");
        assert_eq!(manifest.author, "Test Author");
        assert_eq!(manifest.requires_env, vec!["TEST_API_KEY"]);
        assert_eq!(manifest.provides_tools, vec!["test_tool"]);
        assert_eq!(manifest.provides_hooks, vec!["pre_tool_call"]);
        assert_eq!(manifest.kind, PluginKind::Standalone);
        assert_eq!(manifest.source, PluginSource::Bundled);
    }

    #[test]
    fn test_parse_backend_kind() {
        /// given: a plugin.yaml with kind=backend
        /// when: from_yaml() is called
        /// then: kind is Backend
        let dir = TempDir::new().unwrap();
        create_test_manifest(dir.path(), "name: test\nkind: backend\n");

        let manifest = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled).unwrap();
        assert_eq!(manifest.kind, PluginKind::Backend);
    }

    #[test]
    fn test_parse_exclusive_kind() {
        /// given: a plugin.yaml with kind=exclusive
        /// when: from_yaml() is called
        /// then: kind is Exclusive
        let dir = TempDir::new().unwrap();
        create_test_manifest(dir.path(), "name: test\nkind: exclusive\n");

        let manifest = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled).unwrap();
        assert_eq!(manifest.kind, PluginKind::Exclusive);
    }

    #[test]
    fn test_unknown_kind_defaults_to_standalone() {
        /// given: a plugin.yaml with unknown kind
        /// when: from_yaml() is called
        /// then: kind defaults to Standalone with warning
        let dir = TempDir::new().unwrap();
        create_test_manifest(dir.path(), "name: test\nkind: unknown\n");

        let manifest = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled).unwrap();
        assert_eq!(manifest.kind, PluginKind::Standalone);
    }

    #[test]
    fn test_missing_manifest_file() {
        /// given: a plugin directory without plugin.yaml
        /// when: from_yaml() is called
        /// then: returns Err
        let dir = TempDir::new().unwrap();
        let result = PluginManifest::from_yaml(dir.path(), PluginSource::Bundled);
        assert!(result.is_err());
    }
}
