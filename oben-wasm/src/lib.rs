//! ObenAgent WASM execution layer.
//!
//! Uses wasmtime + wasmtime-wasi to run WASI-compatible WASM modules as
//! platform adapters (Telegram, Discord, Slack, QQ, etc.).

pub mod error;
pub mod runtime;
pub mod loader;
pub mod bridge;
pub mod host;
pub mod hook_bridge;
pub mod hook_registry;
pub mod wasm_hooks;

// Re-export key public types (wasmtime::Result shadows error::Result, so export WasmError explicitly)
pub use error::WasmError;
pub use runtime::*;
pub use loader::*;
pub use bridge::*;

// wasmtime re-exports for convenience
pub use wasmtime::*;

// Re-export hook types from oben-agent so adapters can implement the same traits
pub use oben_agent::hooks::kind;

// Re-export registry types for gateway main.rs
pub use hook_registry::WasmHookRegistry;

/// WIT world definition for platform plugins.
///
/// Plugins should be compiled with wit-bindgen targeting the `platform-plugin-world`.
/// The world exports the `host-api` (host-provided functions) and imports `guest` types.
///
/// ```wit
/// package oben:wasm;
///
/// world platform-plugin-world {
///     export host-api;
///     import guest: interface { types };
/// }
/// ```
pub mod wit {
    // The world-level WIT file is:
    //   oben-wasm/wit/platform.wit
    //
    // Plugins compiled against this world will use wasmtime's component-model
    // bindings generator (wit-bindgen) to produce code that implements
    // the `host-api` interface.
    //
    // See oben-wasm/wit/platform.wit for the full interface contract.
}
