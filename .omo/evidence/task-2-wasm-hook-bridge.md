# Task 2 — WASM Hook Bridge

## Evidence

### Files Created
- `oben-wasm/src/hook_bridge.rs` — WASM hook bridge module (52 lines, 5089 bytes)

### Files Modified
- `oben-wasm/src/lib.rs` — added `pub mod hook_bridge;`

### Verification Results

**1. `cargo check -p oben-wasm` — PASS**
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.64s
```
0 errors, 0 warnings.

**2. WasmHookError enum — PASS**
All 6 variants present:
- `Compilation(String)`
- `Instantiation(String)`
- `Call(String, String)`
- `MissingExport(String)`
- `Unreachable(String)`
- `InvalidUtf8(std::string::FromUtf8Error)`

Plus:
- `WasmResult<T>` type alias
- `From<wasmtime::Error>` impl for automatic trap-to-error conversion

**3. WasmHookBridge struct — PASS**
All required methods present:
- `fn new(component: Component) -> WasmResult<Self>` — instantiation with linker
- `fn engine(&self) -> &Engine` — engine accessor
- `fn store(&self) -> WasmStore<()>` — store factory
- `fn try_call_generic(&mut store, export_name, args) -> WasmResult<()>` — generic caller
- `fn extract_string(&mut store, ptr, len) -> WasmResult<String>` — memory extractor

Plus:
- `CloneableHookBridge` trait with blanket `impl<T>` for clone support

**4. lib.rs includes module — PASS**
Line 11: `pub mod hook_bridge;`

**5. No oben-agent dependency — PASS**
Zero references to `oben_agent` or `oben-agent` in `hook_bridge.rs`.

### API Adjustments from Task Spec

The task-specified code had several wasmtime 44.x API incompatibilities. Adjustments made:

| Task Spec | wasmtime 44.x Reality | Fix Applied |
|-----------|----------------------|-------------|
| `use wasmtime::component::ResourceBorrow, Store` | These types don't exist in `wasmtime::component` | Removed unused imports |
| `linker.allow_oob_lin_mem(true)` | Method doesn't exist | Replaced with `linker.define_unknown_imports_as_traps(component)` |
| `linker.allow_unknown_imports_from_module(true)` | Method doesn't exist | Replaced with `define_unknown_imports_as_traps` |
| `linker.instantiate(&store, component)` | Requires `&mut impl AsContextMut` | Changed to `&mut store` |
| N/A | `?` operator needs `From<wasmtime::Error>` | Added `impl From<wasmtime::Error> for WasmHookError` |
| `#[inline(never)]` on methods | Not in task spec but added | Applied to `try_call_generic` and `extract_string` |
| `use crate::error::Result` | Unused in this module | Removed (module uses `WasmHookError` directly) |
