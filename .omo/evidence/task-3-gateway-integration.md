# Task 3: Gateway WASM Plugin Loader Integration — Evidence

## Verification Results

### 1. `cargo check -p oben-config`
```
Checking oben-config v0.1.0 (/Users/ellie/workspace/oben-alien/oben-config)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.82s
```
✅ 0 errors, 0 warnings.

### 2. `cargo check -p oben-gateway` (default features, no wasm-plugins)
```
Checking oben-config v0.1.0 (/Users/ellie/workspace/oben-alien/oben-config)
Checking oben-tools v0.1.0 (/Users/ellie/workspace/oben-alien/oben-tools)
Checking oben-agent v0.1.0 (/Users/ellie/workspace/oben-alien/oben-agent)
Checking oben-gateway v0.1.0 (/Users/ellie/workspace/oben-alien/oben-gateway)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 7.09s
```
✅ 0 errors, 0 warnings.

### 3. `cargo check -p oben-gateway --features wasm-plugins`
```
Checking oben-platform-sdk v0.1.0 (/Users/ellie/workspace/oben-alien/oben-platform-sdk)
Checking wasmtime v44.0.3
Checking wasmtime-wasi v44.0.3
Checking oben-wasm v0.1.0 (/Users/ellie/workspace/oben-alien/oben-wasm)
Checking oben-gateway v0.1.0 (/Users/ellie/workspace/oben-alien/oben-gateway)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.54s
```
✅ 0 errors, 0 warnings. Feature-gated code path compiles cleanly.

## Changes Summary

### oben-config/src/config.rs
- Added `plugin_dir: Option<PathBuf>` field to `GatewayConfig` struct (line ~499)
- `#[serde(default)]` ensures backward-compatible YAML deserialization
- Documented via doc comment

### oben-gateway/Cargo.toml
- Added `wasm-plugins = ["dep:oben-wasm"]` to `[features]`
- Added `oben-wasm = { path = "../oben-wasm", optional = true }` to `[dependencies]`

### oben-gateway/src/main.rs
- Added `#[allow(unused_mut)]` on `platform_handles` binding
- Added `#[cfg(feature = "wasm-plugins")]` block after `registry.start_all()?` (line ~207)
- Plugin loading logic:
  1. Resolves plugin directory from config or defaults to `~/.obenalien/plugins/wasm`
  2. Scans directory for `.wasm` files
  3. Logs each discovered plugin via `tracing::info!`
  4. Registers a stub `PlatformHandle` (tokio sleep-loop task) per plugin
  5. Inserted into `platform_handles` HashMap before `Gateway::new()`
