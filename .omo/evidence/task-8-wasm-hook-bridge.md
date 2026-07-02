# Task 8: WASM Hook Bridge — Integration Test Evidence

**Date:** 2026-07-01
**Status: Pass**

---

## Changes Made

### Re-export Added
- `oben-wasm/src/error.rs` — Added `pub use crate::hook_bridge::WasmHookError;` so `WasmHookError` is accessible from `oben_wasm::error::WasmHookError` (required by T8c test import).

### Adapter ID Format Bug Fix
- `oben-wasm/src/wasm_hooks.rs` — Fixed all 7 adapter constructors to produce IDs matching the routing prefix patterns expected by `HookBuilder::with_wasm_hooks`:

| Adapter | Before | After |
|---------|--------|-------|
| `WasmAgentLoopAdapter` | `wasm-{name}-agent-loop` | `wasm-agent-loop-{name}` |
| `WasmTurnLifecycleAdapter` | `wasm-{name}-turn` | `wasm-turn-{name}` |
| `WasmToolLifecycleAdapter` | `wasm-{name}-tools` | `wasm-tool-{name}` |
| `WasmStreamingAdapter` | `wasm-{name}-streaming` | `wasm-streaming-{name}` |
| `WasmSystemEventsAdapter` | `wasm-{name}-system` | `wasm-system-{name}` |
| `WasmSessionLifecycleAdapter` | `wasm-{name}-session` | `wasm-session-{name}` |
| `WasmInterruptLifecycleAdapter` | `wasm-{name}-interrupt` | `wasm-interrupt-{name}` |

This was a genuine bug: the old ID format `wasm-{name}-{category}` never matched the routing prefixes (`wasm-agent-loop-`, `wasm-turn-`, etc.), so `with_wasm_hooks` would never route adapter hooks into their correct queues. All hooks would fall through to the `tracing::warn!("unrecognized WASM hook ID")` path.

### New Integration Test File
- `oben-wasm/tests/hook_bridge_test.rs` — 6 test functions covering all 5 scenarios:

## Test Results

```
running 6 tests
test test_wasm_hook_bridge_struct_exists ... ok          [T8a]
test test_hook_builder_wasm_hooks ... ok                 [T8e]
test test_hook_builder_categorization_routing ... ok     [T8e addendum]
test test_wasm_hook_error_variants ... ok                [T8c]
test test_hook_id_prefix_matching ... ok                 [T8b]
test test_wasm_hook_registry_discovery ... ok            [T8d]

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Verification

- `cargo test -p oben-wasm --lib`: 1 passed, 0 failed
- `cargo test -p oben-wasm --test hook_bridge_test`: 6 passed, 0 failed
- `cargo test -p oben-agent --lib`: 171 passed, 0 failed
- No existing tests broken by adapter ID format changes

## Test Coverage Detail

| Scenario | Test Function | What It Verifies |
|----------|--------------|------------------|
| T8a | `test_wasm_hook_bridge_struct_exists` | `WasmHookBridge::new`, `::engine`, `::store` signatures compile; `WasmAgentLoopAdapter::new` signature compiles |
| T8b | `test_hook_id_prefix_matching` | All 10 hook ID strings match their expected category prefixes; WIT internal names don't leak |
| T8c | `test_wasm_hook_error_variants` | All 6 `WasmHookError` variants construct correctly; `Display` output contains original messages |
| T8d | `test_wasm_hook_registry_discovery` | `tempfile::tempdir()` + wasm file creation + `WasmRuntime::new` + discovery flow compiles and runs |
| T8e | `test_hook_builder_wasm_hooks` | `HookBuilder::with_wasm_hooks()` accepts `Vec<Box<dyn Hook>>`; hooks are categorized into queues |
| T8e+ | `test_hook_builder_categorization_routing` | All 7 category prefixes route hooks correctly into separate engine queues |
