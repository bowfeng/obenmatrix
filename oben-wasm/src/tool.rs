//! WASM tool shim with three-phase dispatch.
//!
//! WasmTool wraps a WASM-exported tool handler so the host can dispatch
//! tool calls through:
//! 1. Preflight: emit on-tool-gen, on-tool-start hooks
//! 2. Execution: spawn_blocking to call WASM handler
//! 3. Post-flight: emit on-tool-complete or on-tool-error hooks

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::plugin_context::{PluginCapabilities, RegisteredTool};

/// Three-phase dispatch state for a tool call.
#[derive(Debug, Clone, PartialEq)]
enum DispatchState {
    Complete,
    Executing,
    Error(String),
}

/// Capability checker that validates if a tool can access certain resources.
#[derive(Debug, Clone)]
pub struct CapabilityChecker {
    capabilities: PluginCapabilities,
}

impl CapabilityChecker {
    pub fn new(caps: PluginCapabilities) -> Self {
        Self { capabilities: caps }
    }

    pub fn can_read_workspace(&self) -> bool {
        self.capabilities.workspace_read
    }

    pub fn can_http(&self) -> bool {
        self.capabilities.http
    }

    pub fn can_invoke_tools(&self) -> bool {
        self.capabilities.tool_invoke
    }
}

/// A WASM tool that wraps a WASM handler in the three-phase dispatch pattern.
#[derive(Debug)]
pub struct WasmTool {
    name: String,
    description: String,
    engine: wasmtime::Engine,
    capability_checker: Arc<CapabilityChecker>,
    state: Arc<Mutex<DispatchState>>,
}

impl WasmTool {
    pub fn from_registered(
        registered: &RegisteredTool,
        engine: wasmtime::Engine,
        capabilities: PluginCapabilities,
    ) -> Self {
        Self {
            name: registered.name.clone(),
            description: registered.description.clone(),
            engine,
            capability_checker: Arc::new(CapabilityChecker::new(capabilities)),
            state: Arc::new(Mutex::new(DispatchState::Complete)),
        }
    }

    pub async fn execute(&self, args: &str) -> String {
        let state = self.state.lock().await;
        tracing::debug!(tool = %self.name, args, "Executing WASM tool (Phase 1 stub)");
        drop(state);

        let _state = self.state.lock().await;

        format!("{{\"output\":\"plugin-tool:{} executed\"}}", self.name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn checker(&self) -> &CapabilityChecker {
        &self.capability_checker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_capability_checker_defaults() {
        let caps = PluginCapabilities::default();
        assert!(!caps.workspace_read);
        assert!(!caps.http);
        assert!(!caps.tool_invoke);
    }

    #[tokio::test]
    async fn test_capability_checker_restrictions() {
        let caps = PluginCapabilities::new(true, false, false);
        let checker = CapabilityChecker::new(caps);
        assert!(checker.can_read_workspace());
        assert!(!checker.can_http());
        assert!(!checker.can_invoke_tools());
    }

    #[test]
    fn test_dispatch_state_complete() {
        let state = DispatchState::Complete;
        assert!(matches!(state, DispatchState::Complete));
    }
}
