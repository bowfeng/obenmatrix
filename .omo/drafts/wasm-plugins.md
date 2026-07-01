---
slug: wasm-plugins
status: executing
pending-action: implement wasm plugin system
---

# Draft: wasm-plugins

## Completed
- ✅ Explored `oben-plugin` — all dead code, zero external callers
- ✅ Deleted `oben-plugin` from Cargo.toml and filesystem
- ✅ Plan reviewed and approved

## Decisions
- Runtime: wasmtime 44.0.3 + component-model (matches Ironclaw)
- Plugin dir: ~/.obenalien/plugins/ (configurable)
- Format: .wasm + .platform.json sidecar
- WIT version: 0.1.0
- Hot-reload: defer to Phase 2

## Next Steps
1. Create `oben-wasm/` crate with Cargo.toml, lib.rs
2. Define WIT interface in `wit/platform.wit`
3. Implement runtime engine
4. Implement plugin loader
5. Implement adapter bridge
6. Update config + gateway main.rs
7. E2E tests + final verification
