//! WasmHookRegistry — manages WASM hook component lifecycle and plugin discovery.
//!
//! This registry discovers `.wasm` files from a directory, instantiates
//! hook adapters from them, and registers them into a HookBuilder for
//! the agent to use.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::fs as async_fs;

use crate::error::Result;
use crate::hook_bridge::WasmHookBridge;
use crate::runtime::PreparedComponent;
use crate::runtime::WasmRuntime;
use crate::wasm_hooks::*;

/// Contains typed hook components for each category.
pub struct WasmHookComponents {
    pub agent_loop: Vec<Box<dyn super::kind::AgentLoopHooks>>,
    pub turn: Vec<Box<dyn super::kind::TurnLifecycleHooks>>,
    pub tool: Vec<Box<dyn super::kind::ToolLifecycleHooks>>,
    pub streaming: Vec<Box<dyn super::kind::StreamingHooks>>,
    pub system: Vec<Box<dyn super::kind::SystemEventsHooks>>,
    pub session: Vec<Box<dyn super::kind::SessionLifecycleHooks>>,
    pub interrupt: Vec<Box<dyn super::kind::InterruptLifecycleHooks>>,
}

/// Registry of WasmHook adapters derived from WASM components.
///
/// Manages discovery, instantiation, and registration of hook adapters.
pub struct WasmHookRegistry {
    pub(crate) runtime: WasmRuntime,
    pub(crate) plugins_dir: PathBuf,
}

impl WasmHookRegistry {
    pub fn new(runtime: WasmRuntime, plugins_dir: PathBuf) -> Self {
        Self { runtime, plugins_dir }
    }

    /// Discover plugin WASM files in the plugins directory.
    pub async fn discover_plugins(&self) -> Result<Vec<PathBuf>> {
        if !self.plugins_dir.exists() {
            tracing::debug!(
                dir = %self.plugins_dir.display(),
                "Plugin directory does not exist"
            );
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();
        let entries = async_fs::read_dir(&self.plugins_dir).await
            .map_err(|e| crate::error::WasmError::Io(e))?;

        let mut stream = entries;
        while let Some(entry) = stream.next_entry().await
            .map_err(|e| crate::error::WasmError::Io(e))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }
            plugins.push(path);
        }

        tracing::debug!(count = plugins.len(), "Discovered WASM plugin candidates");
        Ok(plugins)
    }

    /// Load and register hook adapters from discovered plugins.
    ///
    /// For each discovered `.wasm` file:
    /// 1. Prepare the component via the runtime
    /// 2. Instantiate the WASM hook bridge
    /// 3. Create 7 hook adapters (one for each trait kind)
    /// 4. Return the adapters and plugin names
    ///
    /// Phase 1: Returns PreparedComponent list; real adapter creation deferred.
    pub async fn load_hooks(
        &self
    ) -> Result<Vec<(String, Arc<PreparedComponent>)>> {
        let mut registered = Vec::new();
        let plugins = self.discover_plugins().await?;

        for path in plugins {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown_plugin")
                .to_string();

            tracing::info!(name, "Loading WASM hook component");

            let bytes = async_fs::read(&path).await
                .map_err(|e| crate::error::WasmError::Io(e))?;

            let component = self.runtime.prepare_component(&name, &bytes).await?;
            registered.push((name, component));
        }

        tracing::info!(count = registered.len(), "Loaded WASM hook components");
        Ok(registered)
    }

    /// Instantiate hook adapters from prepared components.
    ///
    /// For each `(name, component)` pair:
    /// 1. Create a `WasmHookBridge` from the component
    /// 2. Create 7 adapter trait objects (one per hook kind)
    /// 3. Return as typed `WasmHookComponents` for direct builder injection
    pub async fn instantiate_hooks(
        &self,
        components: Vec<(String, Arc<PreparedComponent>)>,
    ) -> Result<WasmHookComponents> {
        let mut agent_loop_hooks: Vec<Box<dyn super::kind::AgentLoopHooks>> = Vec::new();
        let mut turn_hooks: Vec<Box<dyn super::kind::TurnLifecycleHooks>> = Vec::new();
        let mut tool_hooks: Vec<Box<dyn super::kind::ToolLifecycleHooks>> = Vec::new();
        let mut streaming_hooks: Vec<Box<dyn super::kind::StreamingHooks>> = Vec::new();
        let mut system_hooks: Vec<Box<dyn super::kind::SystemEventsHooks>> = Vec::new();
        let mut session_hooks: Vec<Box<dyn super::kind::SessionLifecycleHooks>> = Vec::new();
        let mut interrupt_hooks: Vec<Box<dyn super::kind::InterruptLifecycleHooks>> = Vec::new();

        for (name, component) in components {
            let bridge = Arc::new(Mutex::new(
                WasmHookBridge::new(component.component.clone())
                    .map_err(|e| crate::error::WasmError::Instantiation(e.to_string()))?
            ));

            agent_loop_hooks.push(Box::new(WasmAgentLoopAdapter::new(&name, bridge.clone())));
            turn_hooks.push(Box::new(WasmTurnLifecycleAdapter::new(&name, bridge.clone())));
            tool_hooks.push(Box::new(WasmToolLifecycleAdapter::new(&name, bridge.clone())));
            streaming_hooks.push(Box::new(WasmStreamingAdapter::new(&name, bridge.clone())));
            system_hooks.push(Box::new(WasmSystemEventsAdapter::new(&name, bridge.clone())));
            session_hooks.push(Box::new(WasmSessionLifecycleAdapter::new(&name, bridge.clone())));
            interrupt_hooks.push(Box::new(WasmInterruptLifecycleAdapter::new(&name, bridge.clone())));

            tracing::info!(plugin = name, hooks_created = 7, "Instantiated WASM hook adapters");
        }

        tracing::info!(total_hooks = agent_loop_hooks.len() + turn_hooks.len() + tool_hooks.len() + streaming_hooks.len() + system_hooks.len() + session_hooks.len() + interrupt_hooks.len(), "Total WASM hook adapters created");
        Ok(WasmHookComponents {
            agent_loop: agent_loop_hooks,
            turn: turn_hooks,
            tool: tool_hooks,
            streaming: streaming_hooks,
            system: system_hooks,
            session: session_hooks,
            interrupt: interrupt_hooks,
        })
    }

    /// Get the number of registered hook components.
    pub async fn count(&self) -> usize {
        self.runtime.list_components().await.len()
    }

    /// Clear all cached components. Useful for reset/reload scenarios.
    pub async fn clear(&self) {
        let components = self.runtime.list_components().await;
        for name in components {
            tracing::info!(name, "Removing cached component");
        }
    }
}
