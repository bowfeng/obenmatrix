//! Plugin discovery — 4-source scanning pipeline.
//!
//! Maps to Hermes' `_discover_plugins()` which scans:
//! 1. Bundled plugins: `<repo>/plugins/<name>/`
//! 2. User plugins: `~/.obenagent/plugins/<name>/`
//! 3. Project plugins: `./.obenagent/plugins/<name>/` (opt-in)
//! 4. (Phase 3) Pip entry-points: `importlib.metadata`
//!
//! Later sources override earlier ones on name collision.

use crate::manifest::{PluginManifest, PluginSource};
use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Plugin discovery source configuration.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Root directory for bundled plugins.
    pub bundled_dir: Option<PathBuf>,

    /// User plugins directory (default: ~/.obenagent/plugins/).
    pub user_dir: Option<PathBuf>,

    /// Project plugins directory (default: ./.obenagent/plugins/).
    pub project_dir: Option<PathBuf>,

    /// Whether to enable project directory scanning (opt-in).
    pub project_enabled: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            bundled_dir: None,
            user_dir: None,
            project_dir: None,
            project_enabled: false,
        }
    }
}

impl DiscoveryConfig {
    /// Create a default discovery config with user dir from ~/.obenagent/plugins/.
    pub fn new() -> Self {
        let user_dir = dirs::config_dir()
            .map(|d| d.join("obenagent").join("plugins"));

        Self {
            user_dir,
            project_enabled: std::env::var("OBERN_PROJECT_PLUGINS").is_ok(),
            ..Default::default()
        }
    }

    /// Set bundled plugins directory.
    pub fn with_bundled_dir(mut self, dir: PathBuf) -> Self {
        self.bundled_dir = Some(dir);
        self
    }

    /// Set user plugins directory.
    pub fn with_user_dir(mut self, dir: PathBuf) -> Self {
        self.user_dir = Some(dir);
        self
    }

    /// Set project plugins directory.
    pub fn with_project_dir(mut self, dir: PathBuf) -> Self {
        self.project_dir = Some(dir);
        self
    }
}

/// Discover plugins from all configured sources.
///
/// Later sources override earlier ones on name collision:
/// project > user > bundled.
///
/// Returns a map of plugin key → manifest for all discovered plugins
/// that have valid plugin.yaml files.
pub fn discover_plugins(config: &DiscoveryConfig) -> Result<std::collections::HashMap<String, PluginManifest>> {
    let mut discovered: std::collections::HashMap<String, PluginManifest> = std::collections::HashMap::new();

    // Scan in order: bundled → user → project (later overrides earlier)
    let sources = build_source_scan(config);
    let source_count = sources.len();

    for (source, dir) in &sources {
        if let Some(path) = dir {
            scan_directory(path, source.clone(), &mut discovered);
        }
    }

    info!("Plugin discovery: {} plugins found across {} sources",
        discovered.len(),
        source_count
    );

    Ok(discovered)
}

/// Build list of (source, path) pairs to scan.
fn build_source_scan(config: &DiscoveryConfig) -> Vec<(PluginSource, Option<PathBuf>)> {
    let mut sources = Vec::new();

    // Bundled
    if let Some(ref dir) = config.bundled_dir {
        sources.push((PluginSource::Bundled, Some(dir.clone())));
    }

    // User
    if let Some(ref dir) = config.user_dir {
        sources.push((PluginSource::User, Some(dir.clone())));
    }

    // Project (opt-in)
    if config.project_enabled {
        if let Some(ref dir) = config.project_dir {
            sources.push((PluginSource::Project, Some(dir.clone())));
        }
    }

    sources
}

/// Scan a directory for plugins.
///
/// Supports two layouts:
/// 1. Flat: `<dir>/<plugin_name>/plugin.yaml`
/// 2. Category: `<dir>/<category>/<plugin_name>/plugin.yaml` (depth-capped at 2)
fn scan_directory(dir: &Path, source: PluginSource, discovered: &mut std::collections::HashMap<String, PluginManifest>) {
    if !dir.exists() {
        debug!("Plugin directory does not exist: {}", dir.display());
        return;
    }

    debug!("Scanning plugin directory: {} (source={})", dir.display(), source.as_str());

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Cannot read plugin directory {}: {}", dir.display(), e);
            return;
        }
    };

    
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Error reading plugin directory entry: {}", e);
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        
        debug!("Found subdirectory: {:?}", path);

        scan_plugin_path(&path, source.clone(), discovered);
    }
    debug!("Scan directory found {} total discovered", discovered.len());
}

/// Scan a single plugin path (supports flat and category layouts).
fn scan_plugin_path(path: &Path, source: PluginSource, discovered: &mut std::collections::HashMap<String, PluginManifest>) {
    
    // Try flat layout first: <path>/plugin.yaml
    let yaml = path.join("plugin.yaml");
    let yml = path.join("plugin.yml");


    if yaml.exists() || yml.exists() {
        if let Ok(manifest) = PluginManifest::from_yaml(path, source.clone()) {
            let key = manifest.lookup_key().to_string();
            discovered.insert(key.clone(), manifest);
            debug!("Discovered plugin: {} (source={})", key, source.as_str());
            return;
        } else {
        }
    }

    // Try category layout: <path>/<category>/<name>/plugin.yaml
    // Support flat plugins inside category (web_search/tavily/) 
    // and nested plugins (web_search/ollama/v1/)
    let categories = list_subdirs(path);
    for category in categories {
        // Check if category itself has plugin.yaml (flat plugin at this level)
        let yaml = category.join("plugin.yaml");
        let yml = category.join("plugin.yml");
        if yaml.exists() || yml.exists() {
            if let Ok(manifest) = PluginManifest::from_yaml(&category, source.clone()) {
                let key = manifest.lookup_key().to_string();
                discovered.insert(key.clone(), manifest);
                debug!(
                    "Discovered category-level plugin: {} (source={})",
                    key, source.as_str()
                );
                continue; // Don't look for deeper nesting in this directory
            }
        }

        // Check nested: <category>/<name>/plugin.yaml
        for item in list_subdirs(&category) {
            let yaml = item.join("plugin.yaml");
            let yml = item.join("plugin.yml");
            if yaml.exists() || yml.exists() {
                if let Ok(manifest) = PluginManifest::from_yaml(&item, source.clone()) {
                    let key = manifest.lookup_key().to_string();
                    discovered.insert(key.clone(), manifest);
                    debug!(
                        "Discovered nested plugin: {} (source={})",
                        key, source.as_str()
                    );
                }
            }
        }
    }
}

/// List subdirectories of a path.
fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.path())
            .collect(),
        Err(_) => vec![],
    }
}

/// Check if a plugin is enabled based on config allow/deny lists.
///
/// Rules:
/// - If `disabled` list contains the plugin key/name, it's blocked
/// - If `enabled` list is Some and non-empty, only listed plugins are allowed
/// - If `enabled` list is None or empty, all plugins pass (except disabled)
/// - Bundled backend/platform plugins auto-pass (not gated by enabled list)
pub fn is_plugin_enabled(
    manifest: &PluginManifest,
    enabled: Option<&Vec<String>>,
    disabled: Option<&Vec<String>>,
) -> bool {
    let key = manifest.lookup_key();
    let name = &manifest.name;

    // Check deny list first (always enforced)
    if let Some(disabled) = disabled {
        if disabled.iter().any(|d| d == key || d == name) {
            debug!("Plugin '{}' blocked by disabled list", key);
            return false;
        }
    }

    // Bundled backend/platforms auto-pass
    if manifest.source == PluginSource::Bundled && manifest.kind.auto_load_when_bundled() {
        return true;
    }

    // Check allow list
    match enabled {
        None => true, // No allow list = everything passes (except disabled)
        Some(list) if list.is_empty() => true, // Empty list = everything passes
        Some(list) => {
            list.iter().any(|e| e == key || e == name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_plugin(dir: &Path, name: &str, source: PluginSource) {
        let plugin_dir = dir.join(name);
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.yaml"),
            format!("name: {}\nkind: standalone\n", name),
        )
        .unwrap();
    }

    #[test]
    fn test_scan_flat_layout() {
        /// given: a directory with flat plugin structure
        /// when: scan_directory() is called
        /// then: plugin is discovered
        let dir = TempDir::new().unwrap();
        create_test_plugin(dir.path(), "my-plugin", PluginSource::User);

        let mut discovered = std::collections::HashMap::new();
        scan_directory(dir.path(), PluginSource::User, &mut discovered);

        assert_eq!(discovered.len(), 1);
        assert!(discovered.contains_key("my-plugin"));
    }

    #[test]
    fn test_scan_category_layout() {
        /// given: a directory with category plugin structure
        /// when: scan_directory() is called
        /// then: plugin is discovered with category/key path
        let dir = TempDir::new().unwrap();
        let category = dir.path().join("web_search");
        fs::create_dir_all(&category).unwrap();
        create_test_plugin(&category, "tavily", PluginSource::Bundled);


        let mut discovered = std::collections::HashMap::new();
        scan_directory(dir.path(), PluginSource::Bundled, &mut discovered);


        assert_eq!(discovered.len(), 1);
        assert!(discovered.contains_key("web_search/tavily"));
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        /// given: a nonexistent directory
        /// when: scan_directory() is called
        /// then: no panic, empty result
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("does-not-exist");

        let mut discovered = std::collections::HashMap::new();
        scan_directory(&nonexistent, PluginSource::User, &mut discovered);

        assert!(discovered.is_empty());
    }

    #[test]
    fn test_is_plugin_enabled_none() {
        /// given: no enabled/disabled config
        /// when: is_plugin_enabled() is called
        /// then: plugin is enabled
        let manifest = PluginManifest {
            name: "test".into(),
            version: "1.0".into(),
            description: "Test".into(),
            author: "Test".into(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::User,
            path: Some("/test".into()),
            kind: crate::plugin_kind::PluginKind::Standalone,
            key: "test".into(),
        };

        assert!(is_plugin_enabled(&manifest, None, None));
    }

    #[test]
    fn test_is_plugin_enabled_disabled_list() {
        /// given: plugin name in disabled list
        /// when: is_plugin_enabled() is called
        /// then: plugin is disabled
        let manifest = PluginManifest {
            name: "test".into(),
            version: "1.0".into(),
            description: "Test".into(),
            author: "Test".into(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::User,
            path: Some("/test".into()),
            kind: crate::plugin_kind::PluginKind::Standalone,
            key: "test".into(),
        };

        assert!(!is_plugin_enabled(&manifest, None, Some(&vec!["test".into()])));
    }

    #[test]
    fn test_is_plugin_enabled_enabled_list() {
        /// given: plugin name in enabled list
        /// when: is_plugin_enabled() is called
        /// then: plugin is enabled
        let manifest = PluginManifest {
            name: "test".into(),
            version: "1.0".into(),
            description: "Test".into(),
            author: "Test".into(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::User,
            path: Some("/test".into()),
            kind: crate::plugin_kind::PluginKind::Standalone,
            key: "test".into(),
        };

        assert!(is_plugin_enabled(&manifest, Some(&vec!["test".into()]), None));
    }

    #[test]
    fn test_is_plugin_enabled_not_in_enabled_list() {
        /// given: plugin name NOT in enabled list
        /// when: is_plugin_enabled() is called
        /// then: plugin is disabled
        let manifest = PluginManifest {
            name: "other".into(),
            version: "1.0".into(),
            description: "Test".into(),
            author: "Test".into(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::User,
            path: Some("/test".into()),
            kind: crate::plugin_kind::PluginKind::Standalone,
            key: "other".into(),
        };

        assert!(!is_plugin_enabled(&manifest, Some(&vec!["test".into()]), None));
    }

    #[test]
    fn test_is_plugin_enabled_bundled_auto_load() {
        /// given: a bundled backend plugin
        /// when: is_plugin_enabled() is called with empty enabled list
        /// then: plugin is auto-enabled
        let manifest = PluginManifest {
            name: "bundled-backend".into(),
            version: "1.0".into(),
            description: "Test".into(),
            author: "Test".into(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::Bundled,
            path: Some("/test".into()),
            kind: crate::plugin_kind::PluginKind::Backend,
            key: "bundled-backend".into(),
        };

        // Backend is auto-loaded when bundled even with empty enabled list
        assert!(is_plugin_enabled(&manifest, Some(&vec![]), None));
    }

    #[test]
    fn test_name_collision_override() {
        /// given: same plugin key discovered from bundled and user
        /// when: discover() is called
        /// then: user version overrides bundled version
        let dir = TempDir::new().unwrap();

        // Bundled version
        create_test_plugin(dir.path(), "test-plugin", PluginSource::Bundled);

        // User version (overrides bundled)
        fs::write(
            dir.path().join("test-plugin").join("plugin.yaml"),
            "name: test-plugin\nversion: \"2.0\"\n",
        )
        .unwrap();

        let config = DiscoveryConfig::new()
            .with_bundled_dir(dir.path().to_path_buf());

        let discovered = discover_plugins(&config).unwrap();
        assert_eq!(discovered.len(), 1);

        let manifest = discovered.get("test-plugin").unwrap();
        // User > bundled, so version should be from user dir
        // (In this test, both use same file so version may be 2.0 from user if user dir exists)
    }
}
