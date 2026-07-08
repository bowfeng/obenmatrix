# Example WASM Plugin

A minimal WASM plugin demonstrating the plugin API.

## Building

```bash
# Requires wasm32-wasip1 target
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release

# Copy the compiled .wasm file
cp target/wasm32-wasip1/release/example_plugin.wasm .
```

## What This Plugin Does

Registers:
- 1 tool: `hello-world` — says hello to the named person
- 1 CLI command: `example-status` — prints plugin status

## Plugin File Structure

```
example-plugin/
├── .platform.json    # Plugin metadata and capabilities
├── plugin.yaml       # Plugin tools and commands list
└── plugin.wasm       # Compiled WASM binary (built separately)
```

## Plugin Manifests

### .platform.json
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

### plugin.yaml
```yaml
name: example-plugin
version: "1.0"
description: Example WASM plugin
```

## How It Works

1. `PluginDiscoverer::discover()` scans the plugin directory and finds `.platform.json`
2. `PluginLoader::load_plugins()` reads the WASM bytes, compiles the component, creates a `PluginContext`
3. The plugin (if compiled) calls `ctx.register_tool()` and `ctx.register_command()` during init
4. `PluginBundle` collects the registrations
5. Gateway registers tools with `ToolRegistry` and commands with CLI
6. At runtime, `WasmTool::execute()` dispatches calls through the three-phase pattern

## Capabilities

This example plugin declares:
- `workspace_read: false` — cannot read workspace files
- `http: false` — cannot make HTTP requests
- `tool_invoke: false` — cannot invoke other tools

Enable features by setting corresponding flags to `true` in `.platform.json`.
