# Task 1: WASM Hook Bridge — WIT Interface Definition

## Overview

Created the WIT (WebAssembly Interface Types) interface declaration for the WASM plugin hook bridge system. This file defines the contract between the Rust host and guest WASM plugins for hook event delivery.

## Artifact

- **Path**: `oben-wasm/wit/hook.wit`
- **Package**: `oben:wasm`

## World Definition

The `hook-plugin-world` world defines:

- **Exports** (host-side — plugins implement these):
  - `plugin-metadata` interface with `plugin-id()` and `plugin-priority()` functions

- **Imports** (guest-side — host calls these when events fire):
  - `turn` — pre-turn/post-turn lifecycle
  - `tool` — tool gen/start/complete/error/progress
  - `streaming` — delta/thinking/reasoning/interim output
  - `system` — status messages with levels
  - `session` — rotation/compression tracking

## Hook Categories

| Category | Events | Purpose |
|----------|--------|---------|
| turn | on-pre-turn, on-post-turn | Per-turn lifecycle |
| tool | on-tool-gen, on-tool-start, on-tool-complete, on-tool-error, on-tool-progress | Tool execution lifecycle |
| streaming | on-stream-delta, on-thinking, on-reasoning, on-interim-assistant | LLM output streaming |
| system | on-status | Status and diagnostics |
| session | on-session-rotate, on-compression-start, on-compression-complete | Session lifecycle |

## Exclusions (Not in Scope)

- AgentLoop hooks — startup-only, not runtime-useful
- InterruptLifecycle hooks — Ctrl+C events only

## Verification Results

1. **File exists**: PASS
2. **Package header (`package oben:wasm;`)**: PASS
3. **5 import interfaces** (turn, tool, streaming, system, session): PASS
4. **2 export functions** (plugin-id, plugin-priority): PASS
5. **No extra interfaces/types/functions**: PASS
6. **No AgentLoop/InterruptLifecycle references**: PASS
