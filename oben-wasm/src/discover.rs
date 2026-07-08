//! PluginDiscoverer — consolidates plugin discovery into a single module.
//!
//! Replaces duplicated discovery logic from `loader.rs` and `hook_registry.rs`.
//! Both modules should now call `PluginDiscoverer::discover()` instead of
//! having their own scanning logic.

use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use tracing::info;

use oben_config::PluginManifest;

/// A discovered plugin with its directory path and parsed manifest.
pub struct DiscoveredPlugin {
    pub dir: PathBuf,
    pub manifest: PluginManifest,
}

/// Discovers all plugins in a given directory.
///
/// Scans the directory for subdirectories containing either:
/// - A `plugin.yaml` file
/// - A `plugin.json` file
///
/// Returns a list of `DiscoveredPlugin` with parsed metadata from
/// `.platform.json` and `plugin.yaml`.
pub struct PluginDiscoverer;

impl PluginDiscoverer {
    /// Scan a directory for plugins and return manifests.
    ///
    /// For each subdirectory of `plugin_dir`, looks for manifest files
    /// and parses them into `PluginManifest`.
    ///
    /// # Arguments
    /// * `plugin_dir` — directory to scan for plugin subdirectories
    ///
    /// # Returns
    /// * `Vec<DiscoveredPlugin>` — parsed manifests with their directory paths
    ///
    /// # Errors
    /// * Returns error if the plugin directory doesn't exist or cannot be read
    pub fn discover(plugin_dir: &Path) -> Result<Vec<DiscoveredPlugin>> {
        if !plugin_dir.exists() {
            info!(path = ?plugin_dir, "Plugin directory not found, skipping discovery");
            return Ok(vec![]);
        }

        let mut discovered = Vec::new();

        for entry in fs::read_dir(plugin_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only consider directories
            if !path.is_dir() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Try to find manifest files (.platform.json or plugin.yaml)
            let platform_json = path.join(".platform.json");
            let plugin_yaml = path.join("plugin.yaml");

            // If neither manifest exists, skip this directory
            if !platform_json.exists() && !plugin_yaml.exists() {
                info!(name = %name, path = ?path, "Skipping directory without manifest");
                continue;
            }

            // Parse manifest
            match parse_manifest(&path) {
                Ok(manifest) => {
                    info!(
                        name = %manifest.name,
                        version = %manifest.version,
                        path = ?path,
                        "Discovered plugin",
                    );
                    discovered.push(DiscoveredPlugin { dir: path, manifest });
                }
                Err(e) => {
                    tracing::warn!(
                        name = %name,
                        path = ?path,
                        error = %e,
                        "Failed to parse plugin manifest, skipping",
                    );
                }
            }
        }

        info!(count = discovered.len(), "Plugin discovery complete");
        Ok(discovered)
    }

    /// Discover plugins using the user's default plugin directory.
    ///
    /// Looks for plugins in `~/.abenmatrix/plugins/` first,
    /// falls back to `~/.config/obenmatrix/plugins/`.
    pub fn discover_defaults() -> Result<Vec<DiscoveredPlugin>> {
        if let Ok(home) = std::env::var("HOME") {
            let base = PathBuf::from(&home);

            let primary = base.join(".abenmatrix").join("plugins");
            if primary.exists() {
                return Self::discover(&primary);
            }

            let fallback = base.join(".config").join("obenmatrix").join("plugins");
            if fallback.exists() {
                return Self::discover(&fallback);
            }
        }

        Ok(vec![])
    }
}

/// Parse a PluginManifest from a plugin directory.
///
/// Looks for `.platform.json` first, falls back to `plugin.yaml`.
///
/// # Arguments
/// * `plugin_path` — path to the plugin's root directory
///
/// # Errors
/// * Returns error if neither manifest file exists or parsing fails
fn parse_manifest(plugin_path: &Path) -> Result<PluginManifest> {
    let json_path = plugin_path.join(".platform.json");
    let yaml_path = plugin_path.join("plugin.yaml");

    if json_path.exists() {
        let text = fs::read_to_string(&json_path)?;
        let manifest: PluginManifest = serde_json::from_str(&text)?;
        return Ok(manifest);
    }

    if yaml_path.exists() {
        let text = fs::read_to_string(&yaml_path)?;
        let manifest: PluginManifest = serde_yaml::from_str(&text)?;
        return Ok(manifest);
    }

    anyhow::bail!(
        "No manifest file found for plugin at {:?}",
        plugin_path
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_empty_dir() {
        let dir = TempDir::new().unwrap();
        let discovered = PluginDiscoverer::discover(dir.path()).unwrap();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        let dir = PathBuf::from("/fake/nonexistent/path");
        let discovered = PluginDiscoverer::discover(&dir).unwrap();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_with_valid_manifest() {
        let dir = TempDir::new().unwrap();

        let plugin_dir = dir.path().join("test-plugin");
        fs::create_dir(&plugin_dir).unwrap();

        let manifest_json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "A test plugin"
        }"#;
        fs::write(plugin_dir.join(".platform.json"), manifest_json).unwrap();

        let discovered = PluginDiscoverer::discover(dir.path()).unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].manifest.name, "test-plugin");
        assert_eq!(discovered[0].dir, plugin_dir);
    }

    #[test]
    fn test_discover_skips_invalid_manifest() {
        let dir = TempDir::new().unwrap();

        let plugin_dir = dir.path().join("bad-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join(".platform.json"),
            "not valid json {{{",
        )
        .unwrap();

        let discovered = PluginDiscoverer::discover(dir.path()).unwrap();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_skips_non_plugin_dirs() {
        let dir = TempDir::new().unwrap();

        let other_dir = dir.path().join("other");
        fs::create_dir(&other_dir).unwrap();
        fs::write(other_dir.join("README.md"), "Hello").unwrap();

        let discovered = PluginDiscoverer::discover(dir.path()).unwrap();
        assert!(discovered.is_empty());
    }
}
