# Task 7: WASM Hook Bridge — Evidence

## Verification

### 1. `cargo check -p oben-gateway` passes with 0 errors

**With `wasm-plugins` feature:**
```
$ cargo check -p oben-gateway --features wasm-plugins
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.17s
```

**Without `wasm-plugins` feature:**
```
$ cargo check -p oben-gateway
Finished `dev` profile [unoptimized [debuginfo] target(s) in 2.30s
```

### 2. Existing WASM plugin discovery logic preserved

The existing WASM platform plugin stub discovery (lines 263-314) is untouched.
The stub handle registration (`wasm_{file_stem}`) and abort task spawning remain
intact in the `#[cfg feature = "wasm-plugins")]` block.

### 3. HookBuilder receives WASM hooks via `.with_wasm_hooks()` before `.build()`

```rust
let hook_engine = HookBuilder::from_config(&app_config.hooks)
    .with_wasm_hooks(wasm_hooks)     // ← WASM hooks injected here
    .build();                         // ← built with NudgeHook + all WASM hooks
```

**Code location:** `oben-gateway/src/main.rs:324-326`

### 4. No changes to existing platform factory spawning code

Verified: Only `oben-gateway/src/main.rs` was modified. No changes to any
platform factory code, platform registry, or `start_all()` flow.

### 5. Feature gate `#[cfg(feature = "wasm-plugins")]` preserved

Three feature-gated locations in main.rs:
- Line 63: `load_wasm_hooks` function definition
- Line 319: `#[cfg(feature = ...)]` for `wasm_hooks` let-binding
- Line 321: `#[cfg(not(feature = "..."))]` fallback let-binding

## Files Modified

| File | Changes |
|------|---------|
| `oben-gateway/src/main.rs` | Added `HookBuilder` import, `load_wasm_hooks` async function, HookEngine construction |
| `oben-wasm/Cargo.toml` | Added `oben-agent` dependency for trait sharing |
| `oben-wasm/src/lib.rs` | Re-exported `oben_agent::hooks::kind` types + `WasmHookRegistry` |
| `oben-wasm/src/wasm_hooks.rs` | Replaced local Hook traits with `use crate::kind::*` import |
| `oben-wasm/src/hook_registry.rs` | Added `instantiate_hooks()` method + required imports |

## Architecture

```
main.rs (gateway)
  │
  ├─ load_wasm_hooks()         # async fn, wasm-plugins feature-gated
  │   ├─ WasmRuntime::new()
  │   ├─ WasmHookRegistry::new()
  │   ├─ registry.load_hooks()
  │   └─ registry.instantiate_hooks()
  │       └─ Vec<Box<dyn Hook>> (7 adapters per plugin)
  │
  ├─ HookBuilder::from_config()  # NudgeHook from config
  │   └─ .with_wasm_hooks()     # injects WASM adapter hooks
  │   └─ .build()               # HookEngine with all hooks
```

The adapter traits are defined in `oben_wasm::wasm_hooks` but import the
`Hook` trait from `oben_agent::hooks::kind` via `use crate::kind::*`.
This ensures the adapters implement the same trait that `HookBuilder::with_wasm_hooks` expects.
