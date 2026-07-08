# Example WASM Plugin

A minimal WASM plugin demonstrating the guest FFI interface.

## Building

```bash
# Requires wasm32-wasip1 target
rustup target add wasm32-wasip1

# Build (isolated workspace, no parent workspace dependency)
cargo build --target wasm32-wasip1 --release

# Copy the compiled .wasm to the plugin directory
cp target/wasm32-wasip1/release/example_plugin.wasm .
```

## What This Plugin Exports

The guest exports pure FFI symbols callable by the host WASM runtime:

| Symbol | Purpose |
|---|---|
| `component_init` | Called once after instantiation. Phase 1 is a no-op; handler maps are const arrays. |
| `plugin_get_tools` | Returns the count of registered tools. |
| `plugin_get_tool_name` | Returns a pointer to the tool name at the given index. |
| `plugin_execute_tool` | Looks up a handler by name, calls it with serialized arguments, returns the result as a `CString`. |
| `plugin_get_commands` | Returns the count of registered CLI commands. |
| `plugin_get_command_name` | Returns a pointer to the command name at the given index. |
| `plugin_execute_command` | Looks up a command handler by name, calls it with an argument array, returns the result as a `CString`. |
| `plugin_free_string` | Frees a `CString` returned by any execute handler. |

The guest declares:
- 1 tool: `hello-world` — returns a greeting message
- 1 CLI command: `example-status` — prints plugin status info

## Plugin File Structure

```
example-plugin/
├── .platform.json    # Plugin metadata, tool/command lists, capabilities
├── plugin.yaml       # Human-readable plugin metadata
└── plugin.wasm       # Compiled WASM binary
```

## Plugin Manifests

### .platform.json

This file is read by `PluginDiscoverer` and `PluginLoader` to determine which tools and commands to register.

```json
{
  "name": "example-plugin",
  "version": "1.0.0",
  "description": "Example WASM plugin demonstrating plugin API",
  "tools": ["hello-world"],
  "cli_commands": ["example-status"],
  "capabilities": {
    "workspace_read": false,
    "http": false,
    "tool_invoke": false
  },
  "sandbox_limits": {
    "max_memory_mb": 64,
    "timeout_ms": 5000,
    "cpu_fuel": 0
  }
}
```

**Key fields:**
- `tools` — list of tool names the guest handles. The loader registers one `RegisteredTool` per entry.
- `cli_commands` — list of CLI command names the guest handles. The loader registers one `RegisteredCommand` per entry.

### plugin.yaml

```yaml
name: example-plugin
version: "1.0"
description: Example WASM plugin
```

## Architecture

### Phase 1: Pure FFI (current)

The guest is a `cdylib` compiled to `wasm32-wasip1` with:
- **No build.rs** — no wasmtime-bindgen or WIT codegen
- **No host deps** — `Cargo.toml` has empty `[workspace]` so it builds standalone
- **Const handler maps** — `TOOLS` and `COMMANDS` are `&[ (&str, HandlerFn) ]` arrays
- **CString for FFI** — `plugin_execute_tool` and `plugin_execute_command` return `CString::into_raw()` so the host can safely reconstruct and free them without fat pointer issues

The host loader reads `plugins` and `cli_commands` from `.platform.json` and registers each one. At runtime, when the engine dispatches a tool call, the host reads the tool name from the tool registry, calls `plugin_execute_tool` on the WASM component with the handler name and args.

### Phase 2: WIT bindings (future)

The WIT definition lives at `oben-wasm/wit/plugin.wit`. Future work will use `wasmtime-bindgen` to generate typed guest/host bindings from this WIT file, replacing the raw FFI approach with the component model.

## Capabilities

This example plugin declares:
- `workspace_read: false` — cannot read workspace files
- `http: false` — cannot make HTTP requests
- `tool_invoke: false` — cannot invoke other tools

Set corresponding flags to `true` in `.platform.json` to enable.
