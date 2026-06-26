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
//! - Provider traits (ImageGenProvider, WebSearchProvider, BrowserProvider, ContextWindowManager)
//! - Provider registries (ImageGen, VideoGen, WebSearch, Browser, ContextWindowManager, ModelProvider)
//!   with register/get_default/get_by_name/list/remove and exclusivity enforcement
//! - Full 4-source directory scanning with config-driven gating
//! - Thread-local tool whitelist
//! - pre_tool_call blocking, pre_llm_call context injection, transform_llm_output
//! - Slash commands with async handling and 30s timeout
//! - CLI command registry
//! - Message injection (append, interrupt, queue)
//! - Plugin introspection with OBERN_PLUGINS_DEBUG logging
//! - Plugin toolset grouping — map toolset → [tool names with plugin attribution]
//! - PluginContext::llm() — host-owned LLM facade for trusted plugins

pub mod cli_command;
pub mod config;
pub mod discovery;
pub mod hook;
pub mod manager;
pub mod manifest;
pub mod message_injector;
pub mod mock_provider;
pub mod plugin_kind;
pub mod provider;
pub mod slash_command;
pub mod whitelist;

pub use cli_command::{CliCommand, CliCommandHandler, CliCommandRegistry, CliCommandSetup};
pub use config::PluginConfig;
pub use discovery::{discover_plugins, DiscoveryConfig};
pub use hook::{
    check_pre_tool_call_block, get_pre_llm_call_context, get_transform_llm_output, invoke_hook,
    BlockAction, HookCallback, HookType,
};
pub use manager::{LoadedPlugin, PluginContext, PluginInfo, PluginManager};
pub use manifest::{parse_manifest, PluginManifest, PluginSource};
pub use message_injector::{InjectedMessage, MessageAction, MessageInjector};
pub use mock_provider::{
    MockBrowserProvider, MockContextWindowManager, MockImageGenProvider, MockModelProvider,
    MockVideoGenProvider, MockWebSearchProvider,
};
pub use plugin_kind::PluginKind;
pub use provider::{
    BrowserMarker, BrowserProvider, BrowserRegistry, ChatCompletionOutput, ChatToolCall,
    CompletionUsage, ContextWindowManager, ContextWindowManagerMarker, ContextWindowManagerRegistry, ImageGenMarker,
    ImageGenProvider, ImageGenRegistry, ModelProvider, ModelProviderMarker, ModelProviderRegistry,
    ProviderKind, ProviderProfile, ToolCallFunction, VideoGenMarker, VideoGenRegistry,
    WebSearchMarker, WebSearchProvider, WebSearchRegistry,
};
pub use slash_command::{SlashCommand, SlashCommandHandler, SlashCommandRegistry};
pub use whitelist::{check_tool_allowed, clear_thread_tool_whitelist, set_thread_tool_whitelist};
