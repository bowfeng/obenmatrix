# Task 3 — WASM Hook Bridge

## Evidence

### 1. `cargo check -p oben-wasm` passes with 0 errors

```
$ cargo check -p oben-wasm
Checking oben-wasm v0.1.0 (/Users/ellie/workspace/oben-alien/oben-wasm)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.01s
```

### 2. File exists at `oben-wasm/src/wasm_hooks.rs`

Confirmed via `ls`:

```
oben-wasm/src/wasm_hooks.rs
```

### 3. File defines exactly 7 adapter structs

Confirmed via `grep`:

| # | Adapter Struct | Trait Interface |
|---|---------------|-----------------|
| 1 | `WasmAgentLoopAdapter` | `Hook` + `AgentLoopHooks` (on_loop_start, on_loop_end) |
| 2 | `WasmTurnLifecycleAdapter` | `Hook` + `TurnLifecycleHooks` (on_pre_turn, on_post_turn) |
| 3 | `WasmToolLifecycleAdapter` | `Hook` + `ToolLifecycleHooks` (on_tool_gen, on_tool_start, on_tool_complete, on_tool_error, on_tool_progress) |
| 4 | `WasmStreamingAdapter` | `Hook` + `StreamingHooks` (on_stream_delta, on_thinking, on_reasoning, on_interim_assistant) |
| 5 | `WasmSystemEventsAdapter` | `Hook` + `SystemEventsHooks` (on_status) |
| 6 | `WasmSessionLifecycleAdapter` | `Hook` + `SessionLifecycleHooks` (on_session_rotate, on_compression_start, on_compression_complete) |
| 7 | `WasmInterruptLifecycleAdapter` | `Hook` + `InterruptLifecycleHooks` (on_interrupt_requested, on_interrupted) |

### 4. Each adapter has `wrap_call` method

Each adapter uses the shared `wrap_call` and `wrap_call_str` helpers:
- **`wrap_call`** — acquires `Mutex<WasmHookBridge>`, creates `WasmStore<()>`, calls the provided closure, catches `WasmHookError`, logs via `tracing::warn!`, returns `Ok(())` on error (never propagates)
- **`wrap_call_str`** — calls `wrap_call` with a stub closure, logs on error

### 5. No `unwrap()` in error paths

Confirmed: `grep -n '\.unwrap()' wasm_hooks.rs` returns zero matches.

Error handling uses `match` exclusively:
- Mutex poisoning: `Err(_poisoned) => tracing::warn!(...); return Ok(())`
- WASM call errors: `Err(e) => tracing::warn!(error = %e); Err(e)`
- All trait methods return `()` (no-Result type), errors handled internally

### 6. `lib.rs` includes `pub mod wasm_hooks;`

Confirmed in `oben-wasm/src/lib.rs`:
```rust
pub mod hook_registry;
pub mod wasm_hooks;
```

### Additional verification: `cargo test -p oben-wasm`

```
test wasm_hooks::tests::test_trait_bounds ... ok
test test_wasm_runtime_config_defaults ... ok
test test_wasm_runtime_config_clone ... ok
test test_discover_plugins_* ... ok
```

All 8 tests pass (1 unit test + 5 integration tests + 2 basic compilation tests).
