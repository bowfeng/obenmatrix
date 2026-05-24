//! Plugin infrastructure for oben-agent.
//!
//! Maps to `hermes_cli/plugins.py` in Hermes Agent.
//!
//! Features:
//! - `PluginKind` enum (5 categories: standalone, backend, exclusive, platform, model-provider)
//! - `PluginManifest` — YAML manifest parsing
//! - `PluginSource` — discovery source enum (bundled, user, project, entrypoint)
//! - `HookType` enum (17 hook types)
//! - `invoke_hook()` — safe hook invocation with try/except per callback
//! - `PluginManager` — singleton with discover/load/invoke/list
//! - `PluginContext` — registration API for tools, hooks, commands, skills, providers
//! - Provider traits (ImageGenProvider, WebSearchProvider, BrowserProvider, ContextEngine)
//! - Full 4-source directory scanning with config-driven gating
//! - Thread-local tool whitelist
//! - pre_tool_call blocking, pre_llm_call context injection, transform_llm_output
//! - Slash commands with async handling and 30s timeout
//! - CLI command registry
//! - Message injection (append, interrupt, queue)
//! - Plugin introspection with OBERN_PLUGINS_DEBUG logging
//!
//! Phase 4 (future): pip entry-points, Provider integration with PluginContext,
//! TUI toolset grouping

pub mod plugin_kind;
pub mod manifest;
pub mod hook;
pub mod manager;
pub mod discovery;
pub mod config;
pub mod provider;
pub mod whitelist;
pub mod slash_command;
pub mod cli_command;
pub mod message_injector;

pub use plugin_kind::PluginKind;
pub use manifest::{PluginManifest, PluginSource, parse_manifest};
pub use hook::{HookType, HookCallback, invoke_hook, check_pre_tool_call_block, get_pre_llm_call_context, get_transform_llm_output, BlockAction};
pub use manager::{PluginManager, PluginContext, LoadedPlugin, PluginInfo};
pub use discovery::{DiscoveryConfig, discover_plugins};
pub use config::PluginConfig;
pub use provider::{ProviderKind, ProviderProfile, ImageGenProvider, WebSearchProvider, BrowserProvider, ContextEngine};
pub use whitelist::{set_thread_tool_whitelist, clear_thread_tool_whitelist, check_tool_allowed};
pub use slash_command::{SlashCommand, SlashCommandRegistry, SlashCommandHandler};
pub use cli_command::{CliCommand, CliCommandRegistry, CliCommandHandler, CliCommandSetup};
pub use message_injector::{MessageInjector, MessageAction, InjectedMessage};
