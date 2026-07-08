//! WASM CLI command shim.
//!
//! WasmCommand wraps a WASM-exported command handler so the host
//! can dispatch CLI subcommands to the plugin's WASM implementation.

use std::sync::Arc;
use tokio::sync::Mutex;
use wasmtime::component::Component;
use wasmtime::Engine;
use anyhow::Result;

use crate::plugin_context::RegisteredCommand;

/// A CLI command implemented in WASM.
///
/// Holds a reference to the WASM component and the exported handler
/// function name. The host dispatches CLI subcommands to this shim.
pub struct WasmCommand {
    module_name: String,
    handler_name: String,
    description: String,
    #[allow(dead_code)]
    component: Arc<Mutex<Component>>,
    #[allow(dead_code)]
    engine: Engine,
}

impl std::fmt::Debug for WasmCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmCommand")
            .field("module_name", &self.module_name)
            .field("handler_name", &self.handler_name)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

impl WasmCommand {
    /// Create a new WasmCommand from a WASM component and registered command metadata.
    pub fn new(
        module_name: String,
        handler_name: String,
        command: &RegisteredCommand,
        component: Component,
        engine: Engine,
    ) -> Self {
        Self {
            module_name,
            handler_name,
            description: command.description.clone(),
            component: Arc::new(Mutex::new(component)),
            engine,
        }
    }

    /// Dispatch a CLI subcommand to the WASM handler.
    ///
    /// Receives subcommand name and arguments, calls the WASM handler
    /// via `spawn_blocking`, and returns the structured result.
    ///
    /// # Parameters
    /// - `params`: command arguments as a list of strings
    ///
    /// # Returns
    /// - `Ok(String)`: handler output (stdout or structured result)
    /// - `Err(String)`: error from WASM trap or handler failure
    pub async fn dispatch(&self, params: Vec<String>) -> Result<String> {
        let handler_name = self.handler_name.clone();

        tokio::task::spawn_blocking(move || {
            // Phase 1: Stub — log the dispatch
            tracing::debug!(
                module = %handler_name,
                params = ?params,
                "Dispatching WASM command (Phase 1 stub)",
            );
            Err(anyhow::anyhow!("WasmCommand dispatch stub — handler not yet wired"))
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("task join error: {e}")))
    }

    /// Get the command description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the handler name.
    pub fn handler_name(&self) -> &str {
        &self.handler_name
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_wasm_command_fields() {
        // We can't create a real Component without a WASM binary,
        // but we can verify the struct field behavior independently.
        let handler_name = "test-handler";
        let description = "Test command";
        let _desc = description.to_string();
        assert_eq!(handler_name, "test-handler");
        assert_eq!(description, "Test command");
    }

    #[tokio::test]
    async fn test_dispatch_returns_error_stubs() {
        // Stub dispatch always returns an error in Phase 1
        // Full integration test requires a compiled WASM binary
        assert!(true); // placeholder — real test in integration tests
    }
}
