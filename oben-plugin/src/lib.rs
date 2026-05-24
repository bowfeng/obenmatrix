//! Plugin infrastructure for oben-agent.
//!
//! Maps to `hermes_cli/plugins.py` in Hermes Agent.
//!
//! Phase 1 features:
//! - `PluginKind` enum (5 categories: standalone, backend, exclusive, platform, model-provider)
//! - `PluginManifest` — YAML manifest parsing
//! - `HookType` enum (17 hook types)
//! - `invoke_hook()` — safe hook invocation with try/except per callback
//! - `PluginManager` — singleton with discover_and_load(), invoke_hook(), list_plugins()
//! - `PluginContext` — registration API for tools, hooks, commands, skills
//!
//! Phase 2 (future): Full 4-source discovery, provider system, pip entry-points

pub mod plugin_kind;
pub mod manifest;
pub mod hook;
pub mod manager;

pub use plugin_kind::PluginKind;
pub use manifest::{PluginManifest, PluginSource, parse_manifest};
pub use hook::{HookType, HookCallback, invoke_hook};
pub use manager::{PluginManager, PluginContext, LoadedPlugin, PluginInfo};
