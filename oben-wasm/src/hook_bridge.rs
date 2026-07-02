//! WASM hook bridge — calls exported WASM functions and wraps results.
//!
//! This module is the bridge between wasmtime Component/Store and the
//! Hook trait implementations. It compiles a WASM component that
//! implements the hook-plugin-world, then provides safe `try_call_*` methods
//! for each hook function.

use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store as WasmStore};

/// Errors specific to WASM hook execution.
#[derive(Debug, thiserror::Error)]
pub enum WasmHookError {
    #[error("WASM compilation error: {0}")]
    Compilation(String),

    #[error("WASM instantiation error: {0}")]
    Instantiation(String),

    #[error("WASM call error on export '{0}': {1}")]
    Call(String, String),

    #[error("Missing WASM export: {0}")]
    MissingExport(String),

    #[error("WASM unreachable trap: {0}")]
    Unreachable(String),

    #[error("Invalid UTF-8 in WASM string: {0}")]
    InvalidUtf8(std::string::FromUtf8Error),
}

impl From<wasmtime::Error> for WasmHookError {
    fn from(e: wasmtime::Error) -> Self {
        WasmHookError::Instantiation(e.to_string())
    }
}

/// Alias for WASM hook results.
pub type WasmResult<T> = std::result::Result<T, WasmHookError>;

/// Bridge to a WASM component that implements the hook-plugin-world.
///
/// Holds the compiled component, a store for execution, and exposes
/// `try_call_*` methods that invoke the guest's exported hook functions.
///
/// Each `try_call_*` method catches wasmtime errors and converts them
/// into `WasmHookError::Call(fn_name, message)` so the adapter layer
/// can log and discard without panicking.
pub struct WasmHookBridge {
    /// The compiled WASM component.
    component: Component,

    /// Engine reference for store creation.
    engine: Engine,
}

impl WasmHookBridge {
    /// Create a new hook bridge from a compiled WASM component.
    ///
    /// Initializes a new `Linker<()>` for the hook-world and
    /// instantiates the component with it.
    pub fn new(component: Component) -> WasmResult<Self> {
        let engine = component.engine().clone();

        // Create a linker for the hook-world.
        // In Phase 1 we use a minimal linker that doesn't require
        // any host functions — the plugin only exports hooks.
        let mut linker = Linker::new(&engine);

        // Instantiate with empty imports (no host functions needed in Phase 1).
        // Define unknown imports as traps so the component can run without host functions.
        linker.define_unknown_imports_as_traps(&component)?;

        let mut store = WasmStore::new(&engine, ());
        let _instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| WasmHookError::Instantiation(e.to_string()))?;

        Ok(Self {
            component,
            engine,
        })
    }

    /// Get the engine reference.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Create a new store for this bridge's component.
    pub fn store(&self) -> WasmStore<()> {
        WasmStore::new(&self.engine, ())
    }

    /// Try to call a specific WASM export function with no arguments returning ().
    ///
    /// This is a generic entry point for calling exported hook functions.
    /// The caller should use the typed `try_call_*` methods instead.
    #[inline(never)]
    pub fn try_call_generic(
        &self,
        _store: &mut WasmStore<()>,
        export_name: &str,
        _args: &[wasmtime::Val],
    ) -> WasmResult<()> {
        let _module = wasmtime::Module::new(&self.engine, &[])
            .or_else(|_| {
                // Try to get module from component
                let bytes = self.component.serialize()
                    .map_err(|e| WasmHookError::Call(export_name.to_string(), format!("serialize failed: {e}")))?;
                wasmtime::Module::new(&self.engine, &bytes)
            });

        // For Phase 1 (stub/scaffold), we log that the call would happen
        tracing::debug!(
            export = export_name,
            "WASM hook export would be called (Phase 1 scaffold)"
        );
        Ok(())
    }

    /// Extract a string from WASM memory given a pointer and length.
    ///
    /// This reads bytes from the WASM module's linear memory and
    /// converts them to a Rust `String`. Returns an error if the
    /// bytes are not valid UTF-8.
    #[inline(never)]
    pub fn extract_string(&self, _store: &mut WasmStore<()>, ptr: u32, len: u32) -> WasmResult<String> {
        // Phase 1 scaffold: return empty string
        // In production, this would read from guest memory
        tracing::debug!(ptr, len, "Extracting WASM string (Phase 1 scaffold)");
        Ok(String::new())
    }
}

/// Marker trait for hooks that support cloning in the bridge.
pub trait CloneableHookBridge: Send + Sync + 'static {
    fn clone_box(&self) -> Box<dyn CloneableHookBridge>;
}

impl<T> CloneableHookBridge for T
where
    T: Clone + Send + Sync + 'static,
{
    fn clone_box(&self) -> Box<dyn CloneableHookBridge> {
        Box::new(self.clone())
    }
}
