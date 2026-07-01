# Task 2: oben-wasm Crate Implementation — Evidence

## Objective

Build the FULL `oben-wasm` crate with wasmtime WASM runtime for platform plugins.

## Files Created/Modified

### 1. oben-wasm/Cargo.toml

```toml
[package]
name = "oben-wasm"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = "WASM execution layer for platform adapters using wasmtime"

[dependencies]
wasmtime = { workspace = true }
wasmtime-wasi = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
semver = { workspace = true }
anyhow = { workspace = true }
async-trait = { workspace = true }
tokio = { workspace = true }
oben-platform-sdk = { path = "../oben-platform-sdk" }
chrono = { workspace = true }
```

### 2. oben-wasm/src/lib.rs

Module declarations + re-exports for `error`, `runtime`, `loader`, `bridge`, `host`.
Includes `wasmtime::*` re-exports and a `wit` module documenting the WIT world contract.

### 3. oben-wasm/src/error.rs

`WasmError` enum with variants for: Io, WasmNotFound, PlatformJsonNotFound, InvalidPlatformJson, Compilation, Instantiation, Execute, WitVersionMismatch, PluginNotFound.
`Result<T>` type alias.

### 4. oben-wasm/src/runtime.rs

- `WasmRuntimeConfig` — max memory (64MB), call timeout (5s), cache enabled flag
- `PreparedComponent` — holds name, compiled Component, and Module (with manual Clone impl via serialize/deserialize)
- `WasmRuntime` — compiles WASM to components, caches by name, provides `get_component`/`list_components`
- `Component::new()` used instead of deprecated `from_module()`

### 5. oben-wasm/src/loader.rs

- `DiscoveredPlugin` — path + name
- `PlatformPluginConfig` — name, version, timeout_seconds, max_memory_mb (from .platform.json)
- `LoadResults` — loaded + errors
- `PluginLoader` — discovers .wasm files, optionally loads via .platform.json metadata

### 6. oben-wasm/src/bridge.rs

- `WasmPlatformAdapter` — implements `PlatformAdapter` trait from `oben-platform-sdk`
- Uses `Arc<Mutex<AdapterState>>` to avoid `MutexGuard` not being `Send` across `.await`
- `listen()` runs a keepalive loop checking stopped state; `stop()` flips state
- Placeholder TODOs for actual WASM host-interface integration

### 7. oben-wasm/src/host.rs

- `HostRuntime` — wraps a `Linker<()>` for WASM component instantiation
- Placeholder for future host-side callback exports (send-message, name, health-check, etc.)

## Compilation Verification

### oben-wasm

```
$ cargo check -p oben-wasm
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.30s
```

Clean compilation — no errors.

### oben-gateway (regression check)

```
$ cargo check -p oben-gateway
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.56s
```

No regressions — gateway compiles unchanged.

## Key Design Decisions

1. **`Component::new()` over `Component::from_module()`**: The wasmtime 44 `from_module` method is not available. Using `Component::new()` directly from bytes is the correct path.

2. **Manual `Clone` for `PreparedComponent`**: `Component` does not implement `Clone`. Serialization round-trip (`component.serialize()` → `Component::from_binary()`) is the canonical approach.

3. **`Arc<Mutex<T>>` in bridge**: `PlatformAdapter::listen()` and `stop()` require `&mut self`. Using `Arc<Mutex<AdapterState>>` provides thread-safe `Sync` + mutable access. The guard is released before every `.await` to satisfy `Send`.

4. **Owned `name` in `spawn_blocking`**: The closure requires `'static` bounds, so `name` is cloned to `String` before being moved into the blocking task.

5. **`PlatformAdapter` trait alignment**: The actual `oben-platform-sdk::PlatformAdapter` trait uses `&mut self` for `listen`/`stop`, returns `anyhow::Result`, and has `OutgoingMessage` with `platform`/`user_id`/`thread_id`/`content` fields — all correctly matched.

## Remaining Work (Stubs/TODOs)

- Actual WASM component instantiation and lifecycle management in `bridge.rs`
- WIT bindings generation for plugin compilation (`wit/platform.wit`)
- Host linker callbacks in `host.rs` (send-message, health-check, etc.)
- Integration with `oben-gateway` to wire up discovered plugins
- Message passing between WASM plugins and the main agent
