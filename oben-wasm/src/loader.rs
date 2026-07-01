use std::path::PathBuf;
use std::sync::Arc;

use tokio::fs;

use crate::error::{WasmError, Result};
use crate::runtime::{PreparedComponent, WasmRuntime};

/// Metadata about a discovered plugin.
pub struct DiscoveredPlugin {
    pub path: PathBuf,
    pub name: String,
}

/// Plugin configuration from .platform.json.
#[derive(serde::Deserialize, Clone, Debug)]
pub struct PlatformPluginConfig {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_memory_mb: Option<u64>,
}

/// Results from loading plugins.
pub struct LoadResults {
    pub loaded: Vec<(String, PlatformPluginConfig, Arc<PreparedComponent>)>,
    pub errors: Vec<(PathBuf, WasmError)>,
}

/// WASM plugin loader — scans directory, loads and validates plugins.
pub struct PluginLoader {
    plugins_dir: PathBuf,
    runtime: WasmRuntime,
}

impl PluginLoader {
    pub fn new(plugins_dir: PathBuf, runtime: WasmRuntime) -> Self {
        Self {
            plugins_dir,
            runtime,
        }
    }

    /// Discover all plugin candidates (.wasm files in directory).
    pub async fn discover_plugins(&self) -> Result<Vec<DiscoveredPlugin>> {
        let mut plugins = Vec::new();

        if !self.plugins_dir.exists() {
            tracing::debug!(
                "Plugin directory does not exist: {}",
                self.plugins_dir.display()
            );
            return Ok(plugins);
        }

        let mut entries = fs::read_dir(&self.plugins_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }

            let file_stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .replace('-', "_");

            let json_path = path.with_extension("platform.json");
            let name = if json_path.exists() {
                let text = fs::read_to_string(&json_path).await?;
                let config: PlatformPluginConfig =
                    serde_json::from_str(&text).map_err(WasmError::InvalidPlatformJson)?;
                config.name
            } else {
                file_stem
            };

            plugins.push(DiscoveredPlugin { path, name });
        }

        Ok(plugins)
    }

    /// Discover plugins WITHOUT loading them.
    pub async fn discover_only(&self) -> Result<Vec<DiscoveredPlugin>> {
        self.discover_plugins().await
    }

    /// Load all discovered plugins.
    pub async fn load_plugins(&self) -> LoadResults {
        let mut results = LoadResults {
            loaded: Vec::new(),
            errors: Vec::new(),
        };

        let discovered = match self.discover_plugins().await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "Failed to discover plugins");
                return results;
            }
        };

        for plugin in discovered {
            let plugin_path = plugin.path.clone();
            if let Err(e) = self.load_single_plugin(&mut results, plugin).await {
                results.errors.push((plugin_path, e));
            }
        }

        if !results.loaded.is_empty() {
            tracing::info!(
                loaded = results.loaded.len(),
                "Loaded WASM platform plugins"
            );
        }

        results
    }

    async fn load_single_plugin(
        &self,
        results: &mut LoadResults,
        plugin: DiscoveredPlugin,
    ) -> Result<()> {
        let plugin_path = &plugin.path;

        // Read WASM bytes
        let wasm_bytes =
            fs::read(plugin_path)
                .await
                .map_err(|_| WasmError::WasmNotFound(plugin_path.clone()))?;

        // Determine plugin name from .platform.json or fallback
        let json_path = plugin_path.with_extension("platform.json");
        let config = if json_path.exists() {
            let text = fs::read_to_string(&json_path)
                .await
                .map_err(|_| WasmError::WasmNotFound(plugin_path.clone()))?;
            serde_json::from_str(&text).map_err(WasmError::InvalidPlatformJson)?
        } else {
            PlatformPluginConfig {
                name: plugin.name.clone(),
                version: "0.1.0".to_string(),
                timeout_seconds: None,
                max_memory_mb: None,
            }
        };

        // Prepare the component
        let component = self
            .runtime
            .prepare_component(&plugin.name, &wasm_bytes)
            .await?;

        let config_name = config.name.clone();
        let config_version = config.version.clone();
        results
            .loaded
            .push((config_name.clone(), config, component));
        tracing::info!(
            name = %config_name,
            version = %config_version,
            source = %plugin_path.display(),
            "Loaded WASM platform plugin"
        );

        Ok(())
    }
}
