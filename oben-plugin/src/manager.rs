/// PluginContext — the facade given to each plugin's `register()` function.
///
/// Maps to Hermes' `PluginContext` class. Provides methods for registering
/// tools, hooks, commands, skills, providers, and platforms.
///
/// PluginContext holds a reference to the PluginManager and the plugin's
/// manifest, allowing it to register capabilities that the manager tracks.

use std::sync::{Arc, Mutex, Weak};
use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::plugin_kind::PluginKind;
use crate::manifest::{PluginManifest, PluginSource};
use crate::hook::{HookCallback, HookType};

/// Context given to plugins so they can register tools and hooks.
///
/// This is the facade that each plugin's `register(ctx)` function receives.
/// Plugins use it to register their capabilities with the PluginManager.
///
/// Mirrors Hermes' PluginContext which provides:
/// - register_tool()
/// - register_hook()
/// - register_command() (slash commands)
/// - register_cli_command() (CLI subcommands)
/// - register_skill()
/// - register_platform()
/// - inject_message()
/// - dispatch_tool()
/// - llm (host-owned LLM facade)
pub struct PluginContext {
    /// The plugin's manifest.
    manifest: Arc<PluginManifest>,

    /// Weak reference to the PluginManager (avoids circular references).
    manager: Weak<Mutex<ManagerInner>>,
}

impl PluginContext {
    /// Create a new PluginContext for a plugin.
    pub fn new(manifest: Arc<PluginManifest>, manager: Weak<Mutex<ManagerInner>>) -> Self {
        Self {
            manifest,
            manager,
        }
    }

    /// Return the plugin's manifest.
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// Register a tool provided by this plugin.
    ///
    /// The tool is registered in the global registry and tracked as
    /// plugin-provided.
    ///
    /// Args:
    ///   - name: Tool name
    ///   - toolset: Tool group/toolset name
    ///   - description: Tool description
    ///   - schema: JSON Schema for tool parameters
    ///   - handler: Async handler function
    ///   - override: Allow overriding existing tools
    pub fn register_tool(
        &self,
        name: &str,
        toolset: &str,
        description: &str,
        schema: Value,
        _handler: Box<dyn Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> + Send>,
        _override: bool,
    ) -> Result<()> {
        // In Phase 1, we track tool registration but don't integrate with
        // the actual tool registry yet. Phase 2+ will connect to
        // oben-tools::registry.
        let manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

        {
            let mut mgr = manager.lock().unwrap();
            mgr.registered_tools.insert(name.to_string());
            if let Some(loaded) = mgr.plugins.get_mut(self.manifest.lookup_key()) {
                loaded.tools_registered.push(name.to_string());
            }
        }

        debug!(
            "Plugin {} registered tool: {} (toolset: {})",
            self.manifest.name, name, toolset
        );

        Ok(())
    }

    /// Register a lifecycle hook callback.
    ///
    /// The callback is stored in the PluginManager and will be invoked
    /// when `invoke_hook()` is called for the given hook type.
    ///
    /// Unknown hook names produce a warning but are still stored so
    /// forward-compatible plugins don't break.
    pub fn register_hook(
        &self,
        hook_type: HookType,
        callback: HookCallback,
    ) -> Result<()> {
        let manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

        {
            let mut mgr = manager.lock().unwrap();

            // Check if hook type is valid
            if !HookType::all().contains(&hook_type) {
                warn!(
                    "Plugin '{}' registered unknown hook '{}' (valid: {})",
                    self.manifest.name,
                    hook_type,
                    HookType::all().iter()
                        .map(|h| h.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            // Register hook
            let hooks_map = mgr.hooks.entry(hook_type.clone()).or_default();
            hooks_map.push(callback);
        }

        Ok(())
    }

    /// Register a slash command (in-session /cmd).
    ///
    /// Slash commands are available in CLI and gateway sessions.
    /// The handler signature is `fn(raw_args: &str) -> Result<String>`.
    pub fn register_command(
        &self,
        name: &str,
        _description: &str,
        _args_hint: &str,
        _handler: Box<dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> + Send>,
    ) -> Result<()> {
        let clean = name.to_lowercase().trim().trim_start_matches('/').replace(' ', "-");

        if clean.is_empty() {
            warn!(
                "Plugin '{}' tried to register a command with an empty name.",
                self.manifest.name,
            );
            return Ok(());
        }

        let manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

        {
            let mut mgr = manager.lock().unwrap();
            if let Some(loaded) = mgr.plugins.get_mut(self.manifest.lookup_key()) {
                loaded.commands_registered.push(clean.clone());
            }
        }

        debug!(
            "Plugin {} registered command: /{}",
            self.manifest.name, clean
        );

        Ok(())
    }

    /// Register a CLI subcommand (terminal `hermes <subcommand>`).
    pub fn register_cli_command(
        &self,
        name: &str,
        _description: &str,
        _setup_fn: Box<dyn Fn() + Send>,
        _handler_fn: Box<dyn Fn() + Send>,
    ) -> Result<()> {
        let manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

        let mut mgr = manager.lock().unwrap();
        // Phase 2: Store CLI command metadata
        let _ = name;

        debug!(
            "Plugin {} registered CLI command: {}",
            self.manifest.name, name
        );

        Ok(())
    }

    /// Register a plugin skill.
    ///
    /// Skills are registered with qualified names (`plugin_name:skill_name`).
    pub fn register_skill(
        &self,
        name: &str,
        path: std::path::PathBuf,
        description: &str,
    ) -> Result<()> {
        if name.contains(':') {
            return Err(anyhow!(
                "Skill name '{}' must not contain ':' (namespace derived from plugin name)",
                name
            ));
        }

        if !name.is_empty() && !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(anyhow!(
                "Invalid skill name '{}'. Must match [a-zA-Z0-9_-]+.",
                name
            ));
        }

        if !path.exists() {
            return Err(anyhow!("SKILL.md not found at {}", path.display()));
        }

        let qualified = format!("{}:{}", self.manifest.name, name);
        let bare_name = name.to_string();
        let qualified_clone = qualified.clone();

        let manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

        {
            let mut mgr = manager.lock().unwrap();
            mgr.plugin_skills.insert(qualified, PluginSkill {
                path,
                plugin: self.manifest.name.clone(),
                bare_name,
                description: description.to_string(),
            });
        }

        debug!(
            "Plugin {} registered skill: {}",
            self.manifest.name, qualified_clone
        );

        Ok(())
    }

    /// Get the host-owned LLM facade for trusted plugins.
    ///
    /// This provides plugins access to the user's active model and auth
    /// without requiring their own provider keys.
    ///
    /// NOTE: Phase 1 stub — returns None. Phase 2+ will implement
    /// `oben-plugin::plugin_llm::PluginLlm`.
    pub fn llm(&self) -> Option<()> {
        // Placeholder — will be implemented in Phase 2
        None
    }
}

/// Inner state for PluginManager (hidden behind Mutex).
pub(crate) struct ManagerInner {
    /// All discovered/loaded plugins.
    plugins: std::collections::HashMap<String, LoadedPlugin>,

    /// Hook callbacks: hook_type → [callbacks].
    hooks: std::collections::HashMap<HookType, Vec<HookCallback>>,

    /// Tool names registered by plugins.
    registered_tools: std::collections::HashSet<String>,

    /// Registered plugin skills: qualified_name → metadata.
    plugin_skills: std::collections::HashMap<String, PluginSkill>,

    /// CLI commands registered by plugins.
    plugin_cli_commands: std::collections::HashMap<String, PluginCliCommand>,

    /// Whether discovery has been run.
    discovered: bool,
}

/// Metadata about a loaded plugin.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    /// Plugin manifest.
    pub manifest: Arc<PluginManifest>,

    /// Whether the plugin is enabled (loaded and active).
    pub enabled: bool,

    /// Error during loading (if any).
    pub error: Option<String>,

    /// Tools registered by this plugin.
    pub tools_registered: Vec<String>,

    /// Hooks registered by this plugin.
    pub hooks_registered: Vec<String>,

    /// Commands registered by this plugin.
    pub commands_registered: Vec<String>,
}

/// Metadata about a registered plugin skill.
#[derive(Debug, Clone)]
struct PluginSkill {
    path: std::path::PathBuf,
    plugin: String,
    bare_name: String,
    description: String,
}

/// Metadata about a registered CLI command.
#[derive(Debug, Clone)]
struct PluginCliCommand {
    name: String,
    description: String,
    plugin: String,
}

/// PluginManager — singleton manager for plugin discovery, loading, and invocation.
///
/// Maps to Hermes' `PluginManager` class which:
/// - Discovers plugins from 4 sources (bundled, user, project, entrypoint)
/// - Loads plugins by calling their `register(ctx)` function
/// - Invokes hooks at lifecycle points
/// - Provides introspection (list_plugins)
///
/// Phase 1 scope:
/// - Basic PluginManager with discover_and_load(), invoke_hook(), list_plugins()
/// - Hook registration and invocation
/// - Basic plugin discovery (bundled + user directories)
///
/// Phase 2 scope:
/// - Full 4-source discovery (including pip entry-points)
/// - Provider system (image_gen, web_search, etc.)
/// - Plugin skills, slash commands, CLI commands
pub struct PluginManager {
    inner: Mutex<ManagerInner>,
}

impl PluginManager {
    /// Create a new PluginManager.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ManagerInner {
                plugins: std::collections::HashMap::new(),
                hooks: std::collections::HashMap::new(),
                registered_tools: std::collections::HashSet::new(),
                plugin_skills: std::collections::HashMap::new(),
                plugin_cli_commands: std::collections::HashMap::new(),
                discovered: false,
            }),
        }
    }

    /// Return the global PluginManager singleton.
    pub fn get() -> &'static Self {
        use once_cell::sync::Lazy;
        static MANAGER: Lazy<PluginManager> = Lazy::new(PluginManager::new);
        &MANAGER
    }

    /// Discover and load plugins.
    ///
    /// Phase 1: Scans bundled + user directories for `plugin.yaml` files.
    /// Phase 2+: Adds project dir, pip entry-points, name collision override.
    pub fn discover_and_load(&mut self, _force: bool) -> Result<()> {
        let mut mgr = self.inner.lock().unwrap();

        if mgr.discovered && !_force {
            return Ok(());
        }

        if _force {
            mgr.plugins.clear();
            mgr.hooks.clear();
            mgr.registered_tools.clear();
            mgr.discovered = false;
        }

        // Phase 1: Scan bundled + user directories
        let manifests = Self::scan_directories();

        for manifest in manifests {
            let key = manifest.lookup_key().to_string();

            // Skip exclusive plugins (they have their own discovery)
            if manifest.kind.is_exclusive() {
                let loaded = LoadedPlugin {
                    manifest: Arc::new(manifest.clone()),
                    enabled: false,
                    error: Some("exclusive plugin — handled by category discovery".to_string()),
                    tools_registered: vec![],
                    hooks_registered: vec![],
                    commands_registered: vec![],
                };
                mgr.plugins.insert(key.clone(), loaded);
                continue;
            }

            // Auto-load bundled backends/platforms
            let should_load = match manifest.source {
                PluginSource::Bundled => manifest.kind.auto_load_when_bundled(),
                _ => false, // User/project plugins need opt-in
            };

            if should_load {
                Self::load_plugin(&mut mgr, manifest);
            } else {
                // Record but don't load (needs opt-in)
                let loaded = LoadedPlugin {
                    manifest: Arc::new(manifest.clone()),
                    enabled: false,
                    error: Some("not enabled in config".to_string()),
                    tools_registered: vec![],
                    hooks_registered: vec![],
                    commands_registered: vec![],
                };
                mgr.plugins.insert(key, loaded);
            }
        }

        mgr.discovered = true;
        info!(
            "Plugin discovery complete: {} found",
            mgr.plugins.len()
        );

        Ok(())
    }

    /// Scan bundled + user directories for plugin manifests.
    fn scan_directories() -> Vec<PluginManifest> {
        let manifests = Vec::new();

        // Phase 1: Scan bundled + user directories only
        // Full Phase 2: Also scan project dir, pip entry-points

        // TODO: Implement actual directory scanning
        // For now, return empty — Phase 2 will implement discovery

        manifests
    }

    /// Load a plugin module and call its `register(ctx)` function.
    fn load_plugin(mgr: &mut ManagerInner, manifest: PluginManifest) {
        let key = manifest.lookup_key().to_string();

        info!(
            "Loading plugin '{}' (source={}, kind={})",
            manifest.name, manifest.source, manifest.kind.as_str()
        );

        // Phase 1: Track the plugin but don't actually load the module
        // Phase 2: Dynamically load the Rust module and call register(ctx)
        let loaded = LoadedPlugin {
            manifest: Arc::new(manifest),
            enabled: true,
            error: None,
            tools_registered: vec![],
            hooks_registered: vec![],
            commands_registered: vec![],
        };

        mgr.plugins.insert(key, loaded);
    }

    /// Invoke all callbacks for a hook type.
    ///
    /// Each callback is wrapped in try/except so a misbehaving plugin
    /// cannot break the core agent loop.
    ///
    /// Returns a list of non-None return values from callbacks.
    pub fn invoke_hook(&self, hook_type: &HookType, args: &Value) -> Vec<Value> {
        let mgr = self.inner.lock().unwrap();
        let callbacks = mgr.hooks.get(hook_type);

        if let Some(callbacks) = callbacks {
            crate::hook::invoke_hook(callbacks, args)
        } else {
            vec![]
        }
    }

    /// List all discovered plugins with their metadata.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        let mgr = self.inner.lock().unwrap();

        mgr.plugins
            .values()
            .map(|loaded| PluginInfo {
                name: loaded.manifest.name.clone(),
                key: loaded.manifest.lookup_key().to_string(),
                kind: loaded.manifest.kind.clone(),
                version: loaded.manifest.version.clone(),
                description: loaded.manifest.description.clone(),
                source: loaded.manifest.source.clone(),
                enabled: loaded.enabled,
                tools: loaded.tools_registered.len(),
                hooks: loaded.hooks_registered.len(),
                commands: loaded.commands_registered.len(),
                error: loaded.error.clone(),
            })
            .collect()
    }

    /// Find a plugin skill by qualified name.
    pub fn find_plugin_skill(&self, qualified_name: &str) -> Option<std::path::PathBuf> {
        let mgr = self.inner.lock().unwrap();
        mgr.plugin_skills
            .get(qualified_name)
            .map(|skill| skill.path.clone())
    }

    /// List all skills registered by a plugin.
    pub fn list_plugin_skills(&self, plugin_name: &str) -> Vec<String> {
        let mgr = self.inner.lock().unwrap();
        let prefix = format!("{}:", plugin_name);
        mgr.plugin_skills
            .iter()
            .filter(|(qn, _)| qn.starts_with(&prefix))
            .map(|(_, skill)| skill.bare_name.clone())
            .collect::<Vec<_>>()
    }

    /// Register a hook callback for a plugin.
    pub(crate) fn register_hook(&mut self, hook_type: &HookType, callback: HookCallback, _manifest: &PluginManifest) {
        let mut mgr = self.inner.lock().unwrap();
        mgr.hooks
            .entry(hook_type.clone())
            .or_default()
            .push(callback);
    }

    /// Track a registered tool for a plugin.
    pub(crate) fn track_registered_tools_for_plugin(&mut self, manifest: &PluginManifest, tool_name: &str) {
        let mut mgr = self.inner.lock().unwrap();
        if let Some(loaded) = mgr.plugins.get_mut(manifest.lookup_key()) {
            loaded.tools_registered.push(tool_name.to_string());
        }
    }

    /// Track a registered command for a plugin.
    pub(crate) fn track_command_for_plugin(&mut self, manifest: &PluginManifest, name: &str, _description: &str, _args_hint: &str) {
        let mut mgr = self.inner.lock().unwrap();
        if let Some(loaded) = mgr.plugins.get_mut(manifest.lookup_key()) {
            loaded.commands_registered.push(name.to_string());
        }
    }

    /// Track a registered CLI command for a plugin.
    pub(crate) fn track_cli_command_for_plugin(&mut self, manifest: &PluginManifest, name: &str, _description: &str) {
        // Phase 2: Store CLI command metadata
        let _ = (manifest, name, _description);
    }

    /// Register a plugin skill.
    pub(crate) fn register_plugin_skill(&mut self, qualified_name: &str, path: std::path::PathBuf, description: &str, plugin: &str) {
        let mut mgr = self.inner.lock().unwrap();
        mgr.plugin_skills.insert(qualified_name.to_string(), PluginSkill {
            path,
            plugin: plugin.to_string(),
            bare_name: qualified_name.split(':').last().unwrap_or("").to_string(),
            description: description.to_string(),
        });
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Plugin metadata for introspection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub key: String,
    pub kind: PluginKind,
    pub version: String,
    pub description: String,
    pub source: PluginSource,
    pub enabled: bool,
    pub tools: usize,
    pub hooks: usize,
    pub commands: usize,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_manager_new() {
        /// given: no PluginManager exists
        /// when: PluginManager::new() is called
        /// then: returns empty manager with no plugins
        let mgr = PluginManager::new();
        let plugins = mgr.list_plugins();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_plugin_context_creation() {
        /// given: a manifest and weak manager reference
        /// when: PluginContext::new() is called
        /// then: returns valid context
        let manifest = Arc::new(PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            author: "Test".to_string(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            source: PluginSource::Bundled,
            path: Some("/test".to_string()),
            kind: PluginKind::Standalone,
            key: "test".to_string(),
        });

        let _mgr = PluginManager::new();
        let weak = std::sync::Arc::downgrade(&std::sync::Arc::new(Mutex::new(
            ManagerInner {
                plugins: std::collections::HashMap::new(),
                hooks: std::collections::HashMap::new(),
                registered_tools: std::collections::HashSet::new(),
                plugin_skills: std::collections::HashMap::new(),
                plugin_cli_commands: std::collections::HashMap::new(),
                discovered: false,
            },
        )));
        let ctx = PluginContext::new(manifest, weak);

        assert_eq!(ctx.manifest().name, "test");
    }

    #[test]
    fn test_invoke_hook_no_callbacks() {
        /// given: empty PluginManager
        /// when: invoke_hook() is called
        /// then: returns empty list
        let mgr = PluginManager::new();
        let args = serde_json::json!({});
        let results = mgr.invoke_hook(&HookType::PreToolCall, &args);
        assert!(results.is_empty());
    }
}
