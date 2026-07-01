# Task 1: Workspace Cargo.toml Update

## Changes Made

### 1. Removed `oben-plugin` from workspace members
- Replaced `"oben-plugin"` with `"oben-wasm"` in `[workspace] members`

### 2. Added new workspace dependencies
```toml
wasmtime = { version = "44.0.3", features = ["component-model"] }
wasmtime-wasi = "44.0.3"
semver = "1.0"
```

### 3. Created `oben-wasm` crate
- `oben-wasm/Cargo.toml` — minimal crate with wasmtime + wasmtime-wasi deps
- `oben-wasm/src/lib.rs` — placeholder stub

## Verification

```
$ cargo check -p oben-config
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.88s

$ cargo check -p oben-gateway
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 32.11s

$ cargo check -p oben-wasm
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 12s
```

All 0 errors.
