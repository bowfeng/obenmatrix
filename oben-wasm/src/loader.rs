//! WASM plugin loading — discovers, compiles, initializes, and collects plugins.
//!
//! This module ties together PluginDiscoverer (manifest parsing),
//! WasmRuntime (compilation), and PluginContext (initialization).

use std::path::PathBuf;

use crate::discover::{DiscoveredPlugin, PluginDiscoverer};
use crate::error::{WasmError, Result};
use crate::plugin_context::{PluginContext, PluginCapabilities, RegisteredTool, RegisteredCommand};
use crate::runtime::WasmRuntime;
use oben_config::PluginManifest;

/// Plugin configuration from `.platform.json`.
#[derive(serde::Deserialize, Clone, Debug)]
pub struct PlatformPluginConfig {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_memory_mb: Option<u64>,
}

/// A bundle of all tools, commands, and messages collected from a loaded plugin.
#[derive(Default)]
pub struct PluginBundle {
    pub tools: Vec<RegisteredTool>,
    pub commands: Vec<RegisteredCommand>,
    pub queued_messages: Vec<super::plugin_context::QueuedMessage>,
    pub prepared_component: Option<std::sync::Arc<crate::runtime::PreparedComponent>>,
    pub errors: Vec<String>,
}

impl std::fmt::Debug for PluginBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginBundle")
            .field("tools", &self.tools)
            .field("commands", &self.commands)
            .field("queued_messages", &self.queued_messages)
            .field("errors", &self.errors)
            .field("prepared_component", &self.prepared_component.as_ref().map(|_| "PreparedComponent"))
            .finish()
    }
}

/// Results from loading plugins.
#[derive(Debug, Default)]
pub struct LoadResults {
    pub bundles: Vec<PluginBundle>,
    pub errors: Vec<(PathBuf, WasmError)>,
}

/// Loader for WASM plugins.
pub struct PluginLoader {
    runtime: WasmRuntime,
}

impl PluginLoader {
    pub fn new(runtime: WasmRuntime) -> Self {
        Self { runtime }
    }

    pub fn with_defaults() -> Self {
        let config = crate::runtime::WasmRuntimeConfig::default();
        let runtime = WasmRuntime::new(config).expect("create default WasmRuntime");
        Self { runtime }
    }

    pub fn discover(dir: &std::path::Path) -> std::result::Result<Vec<DiscoveredPlugin>, anyhow::Error> {
        PluginDiscoverer::discover(dir)
    }

    pub fn discover_only(dir: &std::path::Path) -> std::result::Result<Vec<DiscoveredPlugin>, anyhow::Error> {
        Self::discover(dir)
    }

    pub async fn load_plugins(
        &self,
        dir: &std::path::Path,
    ) -> LoadResults {
        let discovered = match Self::discover(dir) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "Failed to discover plugins");
                return LoadResults {
                    bundles: Vec::new(),
                    errors: vec![(dir.to_path_buf(), WasmError::Execute(format!("Discovery failed: {e}")))],
                };
            }
        };

        let mut bundles = Vec::new();
        let mut errors = Vec::new();

        for item in discovered {
            let plugin_dir = &item.dir;
            let manifest = &item.manifest;

            tracing::info!(
                name = %manifest.name,
                version = %manifest.version,
                "Loading WASM plugin",
            );

            let (mut bundle, loading_failed) = match self.load_single_plugin(plugin_dir, manifest).await {
                Ok(b) => (b, false),
                Err(e) => {
                    tracing::warn!(
                        name = %manifest.name,
                        error = %e.to_string(),
                        "Failed to load plugin, skipping",
                    );
                    let path = plugin_dir.join("plugin.wasm");
                    errors.push((path, WasmError::Execute(e.to_string())));
                    (PluginBundle::default(), true)
                }
            };

            if loading_failed {
                bundle.errors = vec!["plugin load failed".to_string()];
            }
            bundles.push(bundle);
        }

        if !bundles.is_empty() {
            let loaded = bundles.iter().filter(|b| b.errors.is_empty()).count();
            let failed = bundles.iter().filter(|b| !b.errors.is_empty()).count();
            tracing::info!(loaded, failed, "Plugin loading complete");
        }

        LoadResults { bundles, errors }
    }

    async fn load_single_plugin(
        &self,
        plugin_dir: &PathBuf,
        manifest: &PluginManifest,
    ) -> Result<PluginBundle> {
        let wasm_path = plugin_dir.join("plugin.wasm");

        if !wasm_path.exists() {
            return Err(WasmError::WasmNotFound(wasm_path));
        }

        let wasm_bytes = tokio::fs::read(&wasm_path).await
            .map_err(|e| WasmError::Execute(format!("Failed to read WASM file: {e}")))?;

        let component = self
            .runtime
            .prepare_component(&manifest.name, &wasm_bytes)
            .await?;

        let caps = PluginCapabilities::new(
            manifest.capabilities.workspace_read,
            manifest.capabilities.http,
            manifest.capabilities.tool_invoke,
        );
        let ctx = PluginContext::new(caps);

        ctx.register_tool(
            &manifest.name,
            &manifest.description,
            "{}",
            vec![],
        ).await;

        let tools = ctx.take_tools().await;
        let commands = ctx.take_commands().await;
        let messages = ctx.take_messages().await;

        Ok(PluginBundle {
            tools,
            commands,
            queued_messages: messages,
            prepared_component: Some(component),
            errors: vec![],
        })
    }

    pub async fn get_components(&self, name: &str) -> Option<std::sync::Arc<crate::runtime::PreparedComponent>> {
        self.runtime.get_component(name).await
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_loader_creation() {
        let _loader = PluginLoader::with_defaults();
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        let discovered = PluginLoader::discover(&PathBuf::from("/fake/nonexistent/path")).unwrap();
        assert!(discovered.is_empty());
    }
}
