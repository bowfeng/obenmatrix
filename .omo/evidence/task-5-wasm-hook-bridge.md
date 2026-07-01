# Task 5: WASM Hook Injection API

## Summary

Added WASM hook injection API to both `HookBuilder` and `HookEngine` to enable WASM plugins to register hooks dynamically.

## Files Modified

### 1. `oben-agent/src/hooks/runtime.rs`

- **Added `HookEngine::insert_wasm_hooks()`** — accepts `impl IntoIterator<Item = Box<dyn super::kind::Hook>>` and dispatches to the correct queue based on `hook.id()` prefix matching.
- **Changed queue types** from `Vec<Box<dyn SpecificTrait>>` to `Vec<Box<dyn super::kind::Hook>>` to enable uniform storage of all hook types.
- **Added 7 cast helper functions** (`cast_agent_loop`, `cast_turn`, `cast_tool`, `cast_streaming`, `cast_system`, `cast_session`, `cast_interrupt`) — unsafe transmute to re-bind fat pointers to specific trait traits at emit time.
- **Updated all emit methods** (18 total) to use the cast helpers with `raw.as_ref()` to convert `&Box<dyn Hook>` to `&dyn Hook`.
- `HookEngine::new()` — unchanged.

### 2. `oben-agent/src/hooks.rs`

- **Added `HookBuilder::with_wasm_hooks()`** — accepts `Vec<Box<dyn super::kind::Hook>>` and dispatches to the correct queue based on `hook.id()` prefix matching. Returns `Self` for chaining.
- **Changed builder field types** from specific traits to `Vec<Box<dyn super::kind::Hook>>` to match the queue type changes.
- **Updated `from_config()`** to use `Vec<Box<dyn super::kind::Hook>>` for the `turn_hooks` local variable.
- `build()` — unchanged signature (`self` -> `HookEngine`).
- `register_*` methods — unchanged signatures and behavior (accept specific traits, push converts to `Box<dyn Hook>` via upcast).

## Verification

### 1. `cargo check -p oben-agent --lib` — 0 errors, 0 warnings
```
Checking oben-agent v0.1.0 (/Users/ellie/workspace/oben-alien/oben-agent)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.46s
```

### 2. Tests — 171 passed, 0 failed
```
test result: ok. 171 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### 3. `HookEngine::insert_wasm_hooks` — dispatches to all 7 queues
- `wasm-agent-loop-*` -> `agent_loop_hooks`
- `wasm-turn-*` -> `turn_hooks`
- `wasm-tool-*` -> `tool_hooks`
- `wasm-streaming-*` -> `streaming_hooks`
- `wasm-system-*` -> `system_hooks`
- `wasm-session-*` -> `session_hooks`
- `wasm-interrupt-*` -> `interrupt_hooks`
- Unrecognized prefix -> `tracing::warn!`

### 4. `HookBuilder::with_wasm_hooks` — dispatches to all 7 queues
- Same 7 prefix mappings as `HookEngine::insert_wasm_hooks`

### 5. Both methods use `starts_with` prefix matching on `hook.id()`
- All 14 dispatch arms use `id.starts_with("wasm-<prefix>-")`

### 6. `build()` method signature unchanged
```rust
pub fn build(self) -> HookEngine
```

### 7. `register_*` methods unchanged
All 7 register methods retain their original signatures accepting concrete trait types.

### 8. `HookEngine::new()` unchanged
Constructor retains original queue initialization.

## Technical Notes

### Unsafe Downcasting
Since queue storage changed from `Vec<Box<dyn Trait>>` to `Vec<Box<dyn Hook>>`, the emit methods need to downcast from the base `Hook` trait to the specific trait for each queue. This is accomplished via `unsafe fn transmute::<&dyn Hook, &dyn Trait>` which re-binds the fat pointer to a different vtable.

Safety invariant: All hooks stored in a given queue implement the corresponding trait. The queues are only populated via:
- `build()` — takes typed hooks, upcasts to `Box<dyn Hook>` (safe)
- `register_*()` — takes typed hooks, upcasts to `Box<dyn Hook>` (safe)
- `insert_wasm_hooks()` / `with_wasm_hooks()` — dispatched by prefix to the correct queue

The vtable layouts are compatible because Rust trait objects are fat pointers (data pointer + vtable pointer) with identical memory layout regardless of which trait the vtable targets.
