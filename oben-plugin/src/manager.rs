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
        _description: &str,
        _schema: Value,
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

            // Toolset grouping — tool name only
            let entries = mgr.toolset_groups.entry(toolset.to_string()).or_default();
            if !entries.contains(&name.to_string()) {
                entries.push(name.to_string());
            }
            // Tool -> plugin attribution
            mgr.tool_to_plugin.insert(name.to_string(), self.manifest.lookup_key().to_string());
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
        let _manager = self.manager.upgrade().ok_or_else(|| {
            anyhow!("PluginManager no longer exists")
        })?;

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

    /// Check if this plugin is trusted to access the LLM facade.
    ///
    /// Returns true if the plugin is in the `trusted_plugins` config list,
    /// or if it is a builtin provider.
    ///
    /// Builtin providers (oben-image-gen, video-gen, web-search, browser, model-provider)
    /// are always trusted.
    pub fn is_trusted(&self) -> bool {
        if let Some(manager) = self.manager.upgrade() {
            let inner = manager.lock().unwrap();
            let config = &inner.config;
            let key = self.manifest.lookup_key();

            let builtin_name = key.to_lowercase();
            let is_builtin = [
                "oben-image-gen", "oben-video-gen", "oben-web-search",
                "oben-browser", "oben-model-provider",
            ].contains(&builtin_name.as_str());

            let trusted = is_builtin || config.is_trusted(key);

            if !trusted && PluginManager::debug_logging_enabled() {
                warn!(
                    "Plugin '{}' (key={}) is not in trusted_plugins list",
                    self.manifest.name, key
                );
            }

            trusted
        } else {
            false
        }
    }

    // ── Provider Registration ───────────────────────────────────────────

    /// Register an image generation provider.
    pub fn register_image_gen_provider(
        &self,
        provider: Box<dyn crate::provider::ImageGenProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.image_gen_registry.register(provider);
        debug!(
            "Plugin {} registered image gen provider: {}",
            self.manifest.name, name
        );
    }

    /// Register a video generation provider.
    pub fn register_video_gen_provider(
        &self,
        provider: Box<dyn crate::provider::VideoGenProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.video_gen_registry.register(provider);
        debug!(
            "Plugin {} registered video gen provider: {}",
            self.manifest.name, name
        );
    }

    /// Register a web search provider.
    pub fn register_web_search_provider(
        &self,
        provider: Box<dyn crate::provider::WebSearchProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.web_search_registry.register(provider);
        debug!(
            "Plugin {} registered web search provider: {}",
            self.manifest.name, name
        );
    }

    /// Register a browser provider.
    pub fn register_browser_provider(
        &self,
        provider: Box<dyn crate::provider::BrowserProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.browser_registry.register(provider);
        debug!(
            "Plugin {} registered browser provider: {}",
            self.manifest.name, name
        );
    }

    /// Register a context engine (exclusive — replaces previous).
    pub fn register_context_engine(
        &self,
        engine: Box<dyn crate::provider::ContextEngine + Send + Sync>,
    ) {
        let name = engine.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.context_engine_registry.register(engine);
        debug!(
            "Plugin {} registered context engine: {}",
            self.manifest.name, name
        );
    }

    /// Register a model provider.
    pub fn register_model_provider(
        &self,
        provider: Box<dyn crate::provider::ModelProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let mut mgr = manager.lock().unwrap();
        mgr.model_provider_registry.register(provider);
        debug!(
            "Plugin {} registered model provider: {}",
            self.manifest.name, name
        );
    }

    // ── Provider Retrieval (config-driven selection) ────────────────────
    /// Note: These return owned clones. For live references, use the
    /// PluginManager directly via `PluginManager::get()` or `PluginContext`'s
    /// weak reference chain.
    ///
    /// Config-driven selection: when `name` is None, checks `config.providers`
    /// map for a configured provider name (e.g., "image_gen" -> "openai"),
    /// falls back to first registered provider.

    /// Get info about the default image gen provider by configured name.
    pub fn get_image_gen_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let inner = manager.lock().unwrap();
        
        let target = name.or_else(|| inner.config.get_provider("image_gen"));
        
        let profile = match target {
            Some(n) => inner.image_gen_registry.get_by_name(n).map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
            None => inner.image_gen_registry.get_default().map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
        };
        profile
    }

    /// Get info about the default web search provider by configured name.
    pub fn get_web_search_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let inner = manager.lock().unwrap();
        
        let target = name.or_else(|| inner.config.get_provider("web_search"));
        
        let profile = match target {
            Some(n) => inner.web_search_registry.get_by_name(n).map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
            None => inner.web_search_registry.get_default().map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
        };
        profile
    }

    /// Get info about the default browser provider by configured name.
    pub fn get_browser_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let inner = manager.lock().unwrap();
        
        let target = name.or_else(|| inner.config.get_provider("browser"));
        
        let profile = match target {
            Some(n) => inner.browser_registry.get_by_name(n).map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
            None => inner.browser_registry.get_default().map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
        };
        profile
    }

    /// Get info about the active context engine (if any).
    pub fn get_context_engine(&self) -> Option<crate::provider::ProviderProfile> {
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let inner = manager.lock().unwrap();
        let profile = inner.context_engine_registry.get_default().map(|p| {
            p.list_models().into_iter().next()
        }).flatten();
        profile
    }

    /// Get info about the default model provider by configured name.
    pub fn get_model_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let manager = self.manager.upgrade().expect("PluginManager no longer exists");
        let inner = manager.lock().unwrap();
        
        let target = name.or_else(|| inner.config.get_provider("model_provider"));
        
        let profile = match target {
            Some(n) => inner.model_provider_registry.get_by_name(n).map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
            None => inner.model_provider_registry.get_default().map(|p| {
                p.list_models().into_iter().next()
            }).flatten(),
        };
        profile
    }
}

/// Inner state for PluginManager (hidden behind Mutex).
#[allow(dead_code)]
pub struct ManagerInner {
    /// All discovered/loaded plugins.
    plugins: std::collections::HashMap<String, LoadedPlugin>,

    /// Hook callbacks: hook_type → [callbacks].
    hooks: std::collections::HashMap<HookType, Vec<HookCallback>>,

    /// Tool names registered by plugins.
    registered_tools: std::collections::HashSet<String>,

    /// Registered plugin skills: qualified_name → metadata.
    plugin_skills: std::collections::HashMap<String, PluginSkill>,

    /// Slash commands registered by plugins.
    slash_commands: crate::slash_command::SlashCommandRegistry,

    /// CLI commands registered by plugins.
    cli_commands: crate::cli_command::CliCommandRegistry,

    /// Message injector for plugin-injected messages.
    message_injector: crate::message_injector::MessageInjector,

    /// Plugin configuration (enabled/disabled lists).
    config: crate::config::PluginConfig,

    /// Discovery config (directory paths, project opt-in).
    discovery_config: crate::discovery::DiscoveryConfig,

    /// Image gen provider registry (non-exclusive).
    image_gen_registry: crate::provider::ImageGenRegistry,
    /// Video gen provider registry (non-exclusive).
    video_gen_registry: crate::provider::VideoGenRegistry,
    /// Web search provider registry (non-exclusive).
    web_search_registry: crate::provider::WebSearchRegistry,
    /// Browser provider registry (non-exclusive).
    browser_registry: crate::provider::BrowserRegistry,
    /// Context engine registry (exclusive — one at a time).
    context_engine_registry: crate::provider::ContextEngineRegistry,
    /// Model provider registry (non-exclusive).
    model_provider_registry: crate::provider::ModelProviderRegistry,

    /// Plugin toolset grouping: toolset -> tool names only.
    /// Separate from toolset_groups: each tool maps to its owning plugin via 
    /// `tool_to_plugin` (tool_name -> plugin).
    toolset_groups: std::collections::HashMap<String, Vec<String>>,
    /// Maps each registered tool name to its owning plugin name.
    tool_to_plugin: std::collections::HashMap<String, String>,

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

    /// Tool names registered by this plugin.
    pub tools_registered: Vec<String>,

    /// Hook types registered by this plugin.
    pub hooks_registered: Vec<String>,

    /// Slash commands registered by this plugin.
    pub commands_registered: Vec<String>,
}

/// Metadata about a registered plugin skill.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PluginSkill {
    path: std::path::PathBuf,
    plugin: String,
    bare_name: String,
    description: String,
}

/// Metadata about a registered CLI command.
#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    /// Create a new PluginManager and register builtin providers.
    pub fn new() -> Self {
        let mgr = Self {
            inner: Mutex::new(ManagerInner {
                plugins: std::collections::HashMap::new(),
                hooks: std::collections::HashMap::new(),
                registered_tools: std::collections::HashSet::new(),
                plugin_skills: std::collections::HashMap::new(),
                slash_commands: crate::slash_command::SlashCommandRegistry::new(),
                cli_commands: crate::cli_command::CliCommandRegistry::new(),
                message_injector: crate::message_injector::MessageInjector::new(),
                config: crate::config::PluginConfig::default(),
                discovery_config: crate::discovery::DiscoveryConfig::new(),
                image_gen_registry: crate::provider::ImageGenRegistry::new(),
                video_gen_registry: crate::provider::VideoGenRegistry::new(),
                web_search_registry: crate::provider::WebSearchRegistry::new(),
                browser_registry: crate::provider::BrowserRegistry::new(),
                context_engine_registry: crate::provider::ContextEngineRegistry::new(),
                model_provider_registry: crate::provider::ModelProviderRegistry::new(),
                toolset_groups: std::collections::HashMap::new(),
                tool_to_plugin: std::collections::HashMap::new(),
                discovered: false,
            }),
        };
        mgr.register_builtin_providers();
        mgr
    }

    /// Return the global PluginManager singleton.
    pub fn get() -> &'static Self {
        use once_cell::sync::Lazy;
        static MANAGER: Lazy<PluginManager> = Lazy::new(PluginManager::new);
        &MANAGER
    }

    /// Discover and load plugins.
    ///
    /// Phase 2: Full 4-source scanning with config-driven gating.
    pub fn discover_and_load(&mut self, _force: bool) -> Result<()> {
        let mut mgr = self.inner.lock().unwrap();

        if mgr.discovered && !_force {
            return Ok(());
        }

        if _force {
            mgr.plugins.clear();
            mgr.hooks.clear();
            mgr.registered_tools.clear();
            mgr.plugins.iter_mut().for_each(|(_, p)| {
                p.enabled = false;
                p.error = Some("re-discovered".to_string());
            });
            mgr.discovered = false;
        }

        // Phase 2: Full 4-source discovery
        let discovered = crate::discovery::discover_plugins(&mgr.discovery_config)?;

        for (key, manifest) in discovered {
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

            // Config gating: enabled/disabled lists
            let enabled = crate::discovery::is_plugin_enabled(
                &manifest,
                mgr.config.enabled.as_ref(),
                Some(&mgr.config.disabled),
            );

            if !enabled {
                let loaded = LoadedPlugin {
                    manifest: Arc::new(manifest.clone()),
                    enabled: false,
                    error: Some("not enabled in config".to_string()),
                    tools_registered: vec![],
                    hooks_registered: vec![],
                    commands_registered: vec![],
                };
                mgr.plugins.insert(key, loaded);
                continue;
            }

            // Auto-load bundled backends/platforms
            let should_load = match manifest.source {
                PluginSource::Bundled => manifest.kind.auto_load_when_bundled(),
                PluginSource::User => enabled, // User plugins gated by enabled list
                PluginSource::Project => enabled, // Project plugins opt-in
                _ => false,
            };

            if should_load {
                Self::load_plugin(&mut mgr, manifest);
            } else {
                // Track but don't load
                let loaded = LoadedPlugin {
                    manifest: Arc::new(manifest),
                    enabled: false,
                    error: Some("not auto-loading".to_string()),
                    tools_registered: vec![],
                    hooks_registered: vec![],
                    commands_registered: vec![],
                };
                mgr.plugins.insert(key, loaded);
            }
        }

        mgr.discovered = true;
        info!(
            "Plugin discovery complete: {} found, {} enabled",
            mgr.plugins.len(),
            mgr.plugins.values().filter(|p| p.enabled).count()
        );

        Ok(())
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
                author: loaded.manifest.author.clone(),
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
    #[allow(dead_code)]
    pub(crate) fn register_hook(&mut self, hook_type: &HookType, callback: HookCallback, _manifest: &PluginManifest) {
        let mut mgr = self.inner.lock().unwrap();
        mgr.hooks
            .entry(hook_type.clone())
            .or_default()
            .push(callback);
    }

    /// Track a registered tool for a plugin.
    #[allow(dead_code)]
    pub(crate) fn track_registered_tools_for_plugin(&mut self, manifest: &PluginManifest, tool_name: &str) {
        let mut mgr = self.inner.lock().unwrap();
        if let Some(loaded) = mgr.plugins.get_mut(manifest.lookup_key()) {
            loaded.tools_registered.push(tool_name.to_string());
        }
    }

    /// Track a registered command for a plugin.
    #[allow(dead_code)]
    pub(crate) fn track_command_for_plugin(&mut self, manifest: &PluginManifest, name: &str, _description: &str, _args_hint: &str) {
        let mut mgr = self.inner.lock().unwrap();
        if let Some(loaded) = mgr.plugins.get_mut(manifest.lookup_key()) {
            loaded.commands_registered.push(name.to_string());
        }
    }

    /// Track a registered CLI command for a plugin.
    #[allow(dead_code)]
    pub(crate) fn track_cli_command_for_plugin(&mut self, manifest: &PluginManifest, name: &str, _description: &str) {
        // Phase 2: Store CLI command metadata
        let _ = (manifest, name, _description);
    }

    /// Register a plugin skill.
    #[allow(dead_code)]
    pub(crate) fn register_plugin_skill(&mut self, qualified_name: &str, path: std::path::PathBuf, description: &str, plugin: &str) {
        let mut mgr = self.inner.lock().unwrap();
        mgr.plugin_skills.insert(qualified_name.to_string(), PluginSkill {
            path,
            plugin: plugin.to_string(),
            bare_name: qualified_name.split(':').last().unwrap_or("").to_string(),
            description: description.to_string(),
        });
    }

    // -----------------------------------------------------------------------
    // Phase 3: Slash commands
    // -----------------------------------------------------------------------

    /// Register a slash command from a plugin.
    pub fn register_slash_command(&self, cmd: crate::slash_command::SlashCommand) -> Result<()> {
        self.inner.lock().unwrap().slash_commands.register(cmd)?;
        Ok(())
    }

    /// Resolve a slash command invocation.
    pub fn resolve_slash_command(&self, raw: &str) -> Option<crate::slash_command::SlashCommand> {
        self.inner.lock().unwrap()
            .slash_commands
            .resolve(raw)
            .map(|(cmd, _)| cmd)
    }

    /// Execute a slash command with 30s timeout.
    pub async fn execute_slash_command(&self, raw: &str) -> Result<String> {
        self.inner.lock().unwrap().slash_commands.execute(raw).await
    }

    /// List all registered slash commands.
    pub fn list_slash_commands(&self) -> Vec<crate::slash_command::SlashCommand> {
        self.inner.lock().unwrap().slash_commands.list_owned()
    }

    // -----------------------------------------------------------------------
    // Phase 3: CLI commands
    // -----------------------------------------------------------------------

    /// Register a CLI command from a plugin.
    pub fn register_cli_command_internal(
        &self,
        name: &str,
        description: &str,
        plugin: &str,
    ) {
        self.inner.lock().unwrap().cli_commands.register(name, description, plugin);
    }

    /// Register a CLI command with setup/handler functions.
    pub fn register_cli_command_with_fns(
        &self,
        name: &str,
        description: &str,
        plugin: &str,
        setup_fn: Option<std::sync::Arc<dyn crate::cli_command::CliCommandSetup + Send + Sync>>,
        handler_fn: Option<std::sync::Arc<dyn crate::cli_command::CliCommandHandler + Send + Sync>>,
    ) {
        self.inner.lock().unwrap().cli_commands
            .register_with_fns(name, description, plugin, setup_fn, handler_fn);
    }

    /// List all registered CLI commands.
    pub fn list_cli_commands(&self) -> Vec<crate::cli_command::CliCommand> {
        self.inner.lock().unwrap().cli_commands.list_owned()
    }

    /// Get a registered CLI command by name.
    pub fn get_cli_command(&self, name: &str) -> Option<crate::cli_command::CliCommand> {
        self.inner.lock().unwrap().cli_commands.get_owned(name)
    }

    // -----------------------------------------------------------------------
    // Phase 3: Message injection
    // -----------------------------------------------------------------------

    /// Inject a message into the conversation.
    pub fn inject_message(
        &self,
        role: impl Into<String>,
        content: impl Into<String>,
        action: crate::message_injector::MessageAction,
        plugin: impl Into<String>,
    ) -> String {
        self.inner.lock().unwrap().message_injector
            .inject(role, content, action, plugin)
    }

    /// Get all unconsumed messages.
    pub fn get_unconsumed_messages(&self, action: Option<crate::message_injector::MessageAction>) -> Vec<crate::message_injector::InjectedMessage> {
        self.inner.lock().unwrap().message_injector.get_unconsumed(action)
    }

    /// Get interrupt messages.
    pub fn get_interrupt_messages(&self) -> Vec<crate::message_injector::InjectedMessage> {
        self.inner.lock().unwrap().message_injector.get_interrupt_messages()
    }

    /// Consume all messages of a given action.
    pub fn consume_messages(&self, action: crate::message_injector::MessageAction) -> Vec<String> {
        self.inner.lock().unwrap().message_injector.consume(action)
    }

    // -----------------------------------------------------------------------
    // Helper methods for tests
    // -----------------------------------------------------------------------

    /// Register an image gen provider directly into the registry (test helper).
    #[cfg_attr(test, allow(dead_code))]
    pub fn register_image_gen_provider_for_test(
        &self, provider: Box<dyn crate::provider::ImageGenProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let mut mgr = self.inner.lock().unwrap();
        mgr.image_gen_registry.register(provider);
        debug!("Test helper registered image gen provider: {}", name);
    }

    /// Register a web search provider directly into the registry (test helper).
    #[cfg_attr(test, allow(dead_code))]
    pub fn register_web_search_provider_for_test(
        &self, provider: Box<dyn crate::provider::WebSearchProvider + Send + Sync>,
    ) {
        let name = provider.name().to_string();
        let mut mgr = self.inner.lock().unwrap();
        mgr.web_search_registry.register(provider);
        debug!("Test helper registered web search provider: {}", name);
    }

    // -----------------------------------------------------------------------
    // Phase 3: Introspection
    // -----------------------------------------------------------------------

    /// Check if debug logging is enabled via OBERN_PLUGINS_DEBUG env var.
    pub fn debug_logging_enabled() -> bool {
        std::env::var("OBERN_PLUGINS_DEBUG").is_ok()
    }

    /// Register builtin mock providers into each registry on startup.
    /// These act as fallback providers so the system always has at least
    /// one provider per type available without requiring external plugins.
    fn register_builtin_providers(&self) {
        let mut inner = self.inner.lock().unwrap();

        inner.image_gen_registry.register(Box::new(
            crate::mock_provider::MockImageGenProvider::new("oben-image-gen", true),
        ));
        inner.video_gen_registry.register(Box::new(
            crate::mock_provider::MockVideoGenProvider::new("oben-video-gen", true),
        ));
        inner.web_search_registry.register(Box::new(
            crate::mock_provider::MockWebSearchProvider::new("oben-web-search", true),
        ));
        inner.browser_registry.register(Box::new(
            crate::mock_provider::MockBrowserProvider::new("oben-browser", true),
        ));
        inner.model_provider_registry.register(Box::new(
            crate::mock_provider::MockModelProvider::new("oben-model-provider", true),
        ));

        debug!("Registered builtin providers: image-gen, video-gen, web-search, browser, model-provider");
    }

    /// Inject the application configuration into this plugin manager.
    /// This sets up discovery config, provider selection, trusted plugins,
    /// and the enabled/disabled lists.
    pub fn set_config(&mut self, config: crate::config::PluginConfig) {
        let mut inner = self.inner.lock().unwrap();
        inner.config = config;
        debug!("Plugin manager config updated");
    }

    /// Config-driven provider selection — use config.providers map when name is None.
    pub fn get_image_gen_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let inner = self.inner.lock().unwrap();
        let target = name.or_else(|| inner.config.get_provider("image_gen"));
        let profile = match target {
            Some(n) => inner.image_gen_registry.get_by_name(n).and_then(|p| {
                p.list_models().into_iter().next()
            }),
            None => inner.image_gen_registry.get_default().and_then(|p| {
                p.list_models().into_iter().next()
            }),
        };
        profile
    }

    pub fn get_web_search_provider(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let inner = self.inner.lock().unwrap();
        let target = name.or_else(|| inner.config.get_provider("web_search"));
        let profile = match target {
            Some(n) => inner.web_search_registry.get_by_name(n).and_then(|p| {
                p.list_models().into_iter().next()
            }),
            None => inner.web_search_registry.get_default().and_then(|p| {
                p.list_models().into_iter().next()
            }),
        };
        profile
    }

    pub fn get_browser_profile(&self, name: Option<&str>) -> Option<crate::provider::ProviderProfile> {
        let inner = self.inner.lock().unwrap();
        let target = name.or_else(|| inner.config.get_provider("browser"));
        let profile = match target {
            Some(n) => inner.browser_registry.get_by_name(n).and_then(|p| {
                p.list_models().into_iter().next()
            }),
            None => inner.browser_registry.get_default().and_then(|p| {
                p.list_models().into_iter().next()
            }),
        };
        profile
    }
    pub fn list_image_gen_providers(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.image_gen_registry.list()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    /// List all registered web search provider names.
    pub fn list_web_search_providers(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.web_search_registry.list()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    /// List all registered browser provider names.
    pub fn list_browser_providers(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.browser_registry.list()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    /// List all registered context engine names.
    pub fn list_context_engines(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.context_engine_registry.list()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    /// List all registered model provider names.
    pub fn list_model_providers(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.model_provider_registry.list()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    // -----------------------------------------------------------------------
    // Phase 3: Toolset grouping (returned formats fixed to Hermes parity)
    // -----------------------------------------------------------------------

    /// Get all toolset groupings: toolset -> [tool names].
    ///
    /// Returns a HashMap mapping toolset names to the tool names registered
    /// within them. This mirrors Hermes' format: `HashMap<String, Vec<String>>`.
    pub fn get_tools_in_all_toolsets(&self) -> std::collections::HashMap<String, Vec<String>> {
        let inner = self.inner.lock().unwrap();
        inner.toolset_groups.clone()
    }

    /// Get tool names in a specific toolset.
    ///
    /// Returns the list of tool names registered under the given toolset,
    /// or an empty vec if the toolset doesn't exist.
    pub fn get_tools_in_toolset(&self, toolset_name: &str) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.toolset_groups
            .get(toolset_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the plugin that owns a specific tool.
    ///
    /// Returns the plugin key (lookup key) that registered this tool.
    pub fn get_tool_owner(&self, tool_name: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner.tool_to_plugin
            .get(tool_name)
            .cloned()
    }

    /// Get plugin attribution map: tool_name -> plugin_key.
    ///
    /// Returns a separate map that can be used alongside `get_tools_in_toolsets()`
    /// to show which plugin each tool belongs to.
    pub fn get_tool_to_plugin_map(&self) -> std::collections::HashMap<String, String> {
        let inner = self.inner.lock().unwrap();
        inner.tool_to_plugin.clone()
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
    /// Plugin display name.
    pub name: String,
    /// Plugin key for config lookups.
    pub key: String,
    /// Plugin kind.
    pub kind: PluginKind,
    /// Plugin version.
    pub version: String,
    /// Plugin description.
    pub description: String,
    /// Plugin author.
    pub author: String,
    /// Source where plugin was discovered.
    pub source: PluginSource,
    /// Whether the plugin is enabled.
    pub enabled: bool,
    /// Number of tools registered by this plugin.
    pub tools: usize,
    /// Number of hooks registered by this plugin.
    pub hooks: usize,
    /// Number of commands registered by this plugin.
    pub commands: usize,
    /// Error during loading (if any).
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
            provides_providers: vec![],
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
                slash_commands: crate::slash_command::SlashCommandRegistry::new(),
                cli_commands: crate::cli_command::CliCommandRegistry::new(),
                message_injector: crate::message_injector::MessageInjector::new(),
                config: crate::config::PluginConfig::default(),
                discovery_config: crate::discovery::DiscoveryConfig::new(),
                image_gen_registry: crate::provider::ImageGenRegistry::new(),
                video_gen_registry: crate::provider::VideoGenRegistry::new(),
                web_search_registry: crate::provider::WebSearchRegistry::new(),
                browser_registry: crate::provider::BrowserRegistry::new(),
                context_engine_registry: crate::provider::ContextEngineRegistry::new(),
                model_provider_registry: crate::provider::ModelProviderRegistry::new(),
                toolset_groups: std::collections::HashMap::new(),
                tool_to_plugin: std::collections::HashMap::new(),
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

    #[test]
    fn test_builtin_providers_registered_on_new() {
        /// given: no explicit provider registration
        /// when: PluginManager::new() is called
        /// then: builtin mock providers are registered in all registries
        let mgr = PluginManager::new();
        
        let image_gen = mgr.list_image_gen_providers();
        assert!(image_gen.contains(&"oben-image-gen".to_string()));
        
        let web_search = mgr.list_web_search_providers();
        assert!(web_search.contains(&"oben-web-search".to_string()));
        
        let browser = mgr.list_browser_providers();
        assert!(browser.contains(&"oben-browser".to_string()));
        
        let model_provider = mgr.list_model_providers();
        assert!(model_provider.contains(&"oben-model-provider".to_string()));
    }

    #[test]
    fn test_config_driven_provider_selection_image_gen() {
        /// given: a plugin manager with configurable provider selection
        /// when: config is set to use a specific image gen provider
        /// then: get_image_gen_provider(None) returns the configured provider
        let mut mgr = PluginManager::new();
        
        // First add extra providers to make selection meaningful
        mgr.register_image_gen_provider_for_test(Box::new(
            crate::mock_provider::MockImageGenProvider::new("custom", true),
        ));
        
        mgr.set_config(crate::config::PluginConfig {
            enabled: None,
            disabled: vec![],
            trusted: vec![],
            providers: std::collections::HashMap::from([
                ("image_gen".to_string(), "custom".to_string()),
            ]),
        });
        
        let profile = mgr.get_image_gen_provider(None);
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "custom");
    }

    #[test]
    fn test_config_driven_provider_selection_web_search() {
        /// given: a plugin manager with configurable web search provider
        /// when: config specifies web_search provider name
        /// then: get_web_search_provider(None) returns configured provider
        let mut mgr = PluginManager::new();
        
        // Register extra provider with distinct name
        mgr.register_web_search_provider_for_test(Box::new(
            crate::mock_provider::MockWebSearchProvider::new("custom-search", true),
        ));
        
        mgr.set_config(crate::config::PluginConfig {
            enabled: None,
            disabled: vec![],
            trusted: vec![],
            providers: std::collections::HashMap::from([
                ("web_search".to_string(), "custom-search".to_string()),
            ]),
        });
        
        let profile = mgr.get_web_search_provider(None);
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "custom-search");
    }

    #[test]
    fn test_explicit_name_overrides_config() {
        /// given: config specifies a provider name, but explicit name passed
        /// when: get_image_gen_provider(Some(other)) is called
        /// then: the explicit name is returned, not config value
        let mut mgr = PluginManager::new();
        
        // Register two providers
        mgr.register_image_gen_provider_for_test(Box::new(
            crate::mock_provider::MockImageGenProvider::new("config-pref", true),
        ));
        mgr.register_image_gen_provider_for_test(Box::new(
            crate::mock_provider::MockImageGenProvider::new("explicit", true),
        ));
        
        mgr.set_config(crate::config::PluginConfig {
            enabled: None,
            disabled: vec![],
            trusted: vec![],
            providers: std::collections::HashMap::from([
                ("image_gen".to_string(), "config-pref".to_string()),
            ]),
        });
        
        // Explicit name should win over config
        let profile = mgr.get_image_gen_provider(Some("explicit"));
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "explicit");
    }

    #[test]
    fn test_llm_trust_gating_trusted_plugin() {
        /// given: PluginContext for a trusted plugin
        /// when: llm() is called
        /// then: returns Some(())
        let mut mgr = PluginManager::new();
        mgr.set_config(crate::config::PluginConfig {
            enabled: None,
            disabled: vec![],
            trusted: vec!["trusted-plugin".to_string()],
            providers: std::collections::HashMap::new(),
        });
        
        let manifest = Arc::new(PluginManifest {
            name: "trusted-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            author: "Test".to_string(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            provides_providers: vec![],
            source: PluginSource::Bundled,
            path: Some("/test".to_string()),
            kind: PluginKind::Standalone,
            key: "trusted-plugin".to_string(),
        });
        
        let manager_arc = std::sync::Arc::new(Mutex::new(
            ManagerInner {
                plugins: std::collections::HashMap::new(),
                hooks: std::collections::HashMap::new(),
                registered_tools: std::collections::HashSet::new(),
                plugin_skills: std::collections::HashMap::new(),
                slash_commands: crate::slash_command::SlashCommandRegistry::new(),
                cli_commands: crate::cli_command::CliCommandRegistry::new(),
                message_injector: crate::message_injector::MessageInjector::new(),
                config: crate::config::PluginConfig {
                    trusted: vec!["trusted-plugin".to_string()],
                    ..Default::default()
                },
                discovery_config: crate::discovery::DiscoveryConfig::new(),
                image_gen_registry: crate::provider::ImageGenRegistry::new(),
                video_gen_registry: crate::provider::VideoGenRegistry::new(),
                web_search_registry: crate::provider::WebSearchRegistry::new(),
                browser_registry: crate::provider::BrowserRegistry::new(),
                context_engine_registry: crate::provider::ContextEngineRegistry::new(),
                model_provider_registry: crate::provider::ModelProviderRegistry::new(),
                toolset_groups: std::collections::HashMap::new(),
                tool_to_plugin: std::collections::HashMap::new(),
                discovered: false,
            },
        ));
        let weak = std::sync::Arc::downgrade(&manager_arc);
        let ctx = PluginContext::new(manifest, weak);
        
        assert!(ctx.is_trusted());
    }

    #[test]
    fn test_is_trust_gating_untrusted_plugin() {
        /// given: PluginContext for an untrusted plugin
        /// when: is_trusted() is called
        /// then: returns false
        let manifest = Arc::new(PluginManifest {
            name: "untrusted-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            author: "Test".to_string(),
            requires_env: vec![],
            provides_tools: vec![],
            provides_hooks: vec![],
            provides_providers: vec![],
            source: PluginSource::Bundled,
            path: Some("/test".to_string()),
            kind: PluginKind::Standalone,
            key: "untrusted-plugin".to_string(),
        });
        
        let manager_arc = std::sync::Arc::new(Mutex::new(
            ManagerInner {
                plugins: std::collections::HashMap::new(),
                hooks: std::collections::HashMap::new(),
                registered_tools: std::collections::HashSet::new(),
                plugin_skills: std::collections::HashMap::new(),
                slash_commands: crate::slash_command::SlashCommandRegistry::new(),
                cli_commands: crate::cli_command::CliCommandRegistry::new(),
                message_injector: crate::message_injector::MessageInjector::new(),
                config: crate::config::PluginConfig {
                    trusted: vec![],
                    ..Default::default()
                },
                discovery_config: crate::discovery::DiscoveryConfig::new(),
                image_gen_registry: crate::provider::ImageGenRegistry::new(),
                video_gen_registry: crate::provider::VideoGenRegistry::new(),
                web_search_registry: crate::provider::WebSearchRegistry::new(),
                browser_registry: crate::provider::BrowserRegistry::new(),
                context_engine_registry: crate::provider::ContextEngineRegistry::new(),
                model_provider_registry: crate::provider::ModelProviderRegistry::new(),
                toolset_groups: std::collections::HashMap::new(),
                tool_to_plugin: std::collections::HashMap::new(),
                discovered: false,
            },
        ));
        let weak = std::sync::Arc::downgrade(&manager_arc);
        let ctx = PluginContext::new(manifest, weak);
        
        assert!(!ctx.is_trusted());
    }

    #[test]
    fn test_toolset_grouping_format_fix() {
        /// given: a PluginManager with tools registered under different toolsets
        /// when: get_tools_in_*() methods are called
        /// then: returns correct format with tool names only + separate attribution
        let mgr = PluginManager::new();
        
        // Simulate tool registration by manually populating the internal state
        {
            let mut inner = mgr.inner.lock().unwrap();
            inner.toolset_groups.insert("core".to_string(), vec!["read".to_string(), "write".to_string()]);
            inner.toolset_groups.insert("files".to_string(), vec!["touch".to_string(), "delete".to_string()]);
            inner.tool_to_plugin.insert("read".to_string(), "plugin-a".to_string());
            inner.tool_to_plugin.insert("write".to_string(), "plugin-a".to_string());
            inner.tool_to_plugin.insert("touch".to_string(), "plugin-b".to_string());
            inner.tool_to_plugin.insert("delete".to_string(), "plugin-b".to_string());
        }
        
        // get all toolsets
        let toolsets = mgr.get_tools_in_all_toolsets();
        assert_eq!(toolsets.get("core").unwrap(), &vec!["read".to_string(), "write".to_string()]);
        assert_eq!(toolsets.get("files").unwrap(), &vec!["touch".to_string(), "delete".to_string()]);
        assert_eq!(toolsets.len(), 2);
        
        // get tools in specific toolset
        assert_eq!(mgr.get_tools_in_toolset("core"), vec!["read".to_string(), "write".to_string()]);
        assert!(mgr.get_tools_in_toolset("nonexistent").is_empty());
        
        // get tool owner
        assert_eq!(mgr.get_tool_owner("read"), Some("plugin-a".to_string()));
        assert_eq!(mgr.get_tool_owner("touch"), Some("plugin-b".to_string()));
        assert!(mgr.get_tool_owner("unknown").is_none());
        
        // get attribution map
        let attribution = mgr.get_tool_to_plugin_map();
        assert_eq!(attribution.get("read").unwrap(), "plugin-a");
        assert_eq!(attribution.get("touch").unwrap(), "plugin-b");
        assert_eq!(attribution.len(), 4);
    }
}
