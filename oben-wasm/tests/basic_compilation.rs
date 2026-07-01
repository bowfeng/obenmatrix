//! Basic compilation tests for the oben-wasm crate.
//!
//! These trivial tests verify that public types from the crate are
//! accessible from integration test code, confirming the crate's public
//! API surface compiles correctly.

use oben_wasm::WasmRuntimeConfig;

/// Given nothing, when constructing WasmRuntimeConfig::default(),
/// then we get a valid config with expected defaults.
///
/// This is a compile-time + runtime sanity check that the most-used
/// config struct from the crate is constructible and has sensible defaults.
#[test]
fn test_wasm_runtime_config_defaults() {
    let config = WasmRuntimeConfig::default();

    assert_eq!(config.max_memory, 64 * 1024 * 1024, "max_memory should be 64MB");
    assert_eq!(config.call_timeout_ms, 5000, "call_timeout_ms should be 5s");
    assert_eq!(config.cache_enabled, true, "cache should be enabled by default");
}

/// Verify that WasmRuntimeConfig implements Clone.
#[test]
fn test_wasm_runtime_config_clone() {
    let config = WasmRuntimeConfig::default();
    let cloned = config.clone();
    
    assert_eq!(config.max_memory, cloned.max_memory);
    assert_eq!(config.call_timeout_ms, cloned.call_timeout_ms);
    assert_eq!(config.cache_enabled, cloned.cache_enabled);
}
