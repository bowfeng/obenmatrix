# PRD: Plugin System — Parity with Hermes Agent

**Severity scale:** `priority-critical` (core extensibility), `priority-high` (major feature gap), `priority-medium` (useful enhancement), `priority-low` (nice-to-have)

---

## Summary

Hermes Agent has a full **plugin system** that lets users extend the agent with custom tools, hooks, providers, skills, CLI commands, and slash commands — without modifying core code. ObenAgent currently has a **basic tool registry** and a **simpler callback system**, but no plugin discovery, loading, manifest parsing, or extensibility framework.

This is a **priority-critical** gap: without it, ObenAgent cannot support third-party extensions, custom provider backends, or user-defined lifecycle hooks.

**Phase 1 progress (#46):** Phase 1 + Phase 2 + Phase 3 implemented — Phase 1: `PluginManager` singleton, `PluginManifest` YAML parsing, `PluginKind` enum, `HookType` enum (17 types), `invoke_hook()`, `PluginContext` registration API, `PluginSource` enum. Phase 2: `ImageGenProvider`/`WebSearchProvider`/`BrowserProvider`/`ContextEngine` provider traits, full 4-source directory scanning with config-driven gating, `PluginConfig` (enabled/disabled lists), thread-local tool whitelist, `pre_tool_call` blocking, `pre_llm_call` context injection, `transform_llm_output` transformation. Phase 3: `SlashCommandRegistry` with async handling (30s timeout), `CliCommandRegistry`, `MessageInjector` (append/interrupt/queue), introspection with `OBERN_PLUGINS_DEBUG` logging. 66 unit tests passing. Remaining: plugin skills, pip entry-points, provider integration with PluginContext, TUI toolset grouping.

---

## Existing ObenAgent Capabilities

| Feature | Status | Notes |
|---------|--------|-------|
| ToolRegistry | ✅ Basic | `oben-tools/src/registry.rs` — tool registration, dispatch, auto-registration |
| Callback system | ✅ Basic | `oben-agent/src/callbacks.rs` — 12+ callback types, but not hook-based |
| System prompt cache | ✅ Basic | `oben-agent/src/system_prompt_cache.rs` — TTL-based cache |
| Tool dispatch (serial/concurrent) | ✅ Basic | `oben-agent/src/concurrent_dispatch.rs` |
| Error classification | ✅ Basic | `oben-agent/src/error_classifier.rs` — 8 categories |
| Retry with backoff | ✅ Basic | `oben-agent/src/retry.rs` |
| Fallback chain | ✅ Basic | `oben-agent/src/fallback.rs` |
| Message sanitization | ✅ Basic | `oben-agent/src/message_sanitize.rs` |
| Stream scrubbers | ✅ Basic | `oben-agent/src/stream_processor.rs` |
| Interrupt mechanism | ✅ Basic | `oben-agent/src/interrupt.rs` |

---

## Parity Gaps

### 1. PluginManager — Central Discovery & Lifecycle

| Severity | Description |
|----------|-------------|
| **priority-critical** | No `PluginManager` equivalent. Hermes scans 4 sources (bundled, user, project, pip entry-points) and loads plugins via `__init__.py::register(ctx)`. Oben has no plugin discovery, no loading, no lifecycle management. |

| Status | Description |
|--------|-------------|
| ✅ #46 | **PluginManager** — Singleton manager with `discover_and_load()`, `invoke_hook()`, `list_plugins()`, `find_plugin_skill()`, `list_plugin_skills()`, `remove_plugin_skill()` |

### 2. PluginManifest — YAML-Based Plugin Declarations

| Severity | Description |
|----------|-------------|
| **priority-critical** | No `PluginManifest` struct. Each plugin in Hermes has a `plugin.yaml` with `name`, `version`, `description`, `requires_env`, `provides_tools`, `provides_hooks`, `kind`. Oben has no manifest parsing. |

| Status | Description |
|--------|-------------|
| ✅ #46 | **PluginManifest** — Rust struct for plugin.yaml fields: name, version, description, author, requires_env, provides_tools, provides_hooks, kind, source, path, key |

### 3. Plugin Discovery — 4-Source Scanning

| Severity | Description |
|----------|-------------|
| **priority-critical** | No multi-source plugin scanning. Hermes scans: bundled (`plugins/<name>/`), user (`~/.hermes/plugins/<name>/`), project (`./.hermes/plugins/<name>/`), and pip entry-points (`importlib.metadata`). |

| Status | Description |
|--------|-------------|
| ✅ #46 | **Directory scanning** — Recursively scan plugin directories for `plugin.yaml`, support flat and category layouts (e.g., `image_gen/openai`), depth-capped at 2 segments |
| ✅ #46 | **Source discovery** — Bundled (repo-shipped), user (config-dir), project (opt-in via env); pip entry-points deferred to Phase 3 |
| ✅ #46 | **Name collision override** — Later sources override earlier ones (user > bundled, project > user) |

### 4. Plugin Kinds — 5 Categories

| Severity | Description |
|----------|-------------|
| **priority-high** | No plugin kind system. Hermes uses 5 kinds to control loading behavior: standalone (opt-in), backend (bundled auto-load, user opt-in), exclusive (single active provider), platform (bundled auto-load, user opt-in), model-provider (handled by provider discovery). |

| Status | Description |
|--------|-------------|
| ✅ #46 | **PluginKind enum** — Standalone, Backend, Exclusive, Platform, ModelProvider |

### 5. PluginContext — Plugin Registration API

| Severity | Description |
|----------|-------------|
| **priority-critical** | No `PluginContext`. This is the facade given to each plugin's `register()` function. It provides methods for registering tools, hooks, commands, skills, providers, platforms, context engines, and injecting messages. |

| Status | Description |
|--------|-------------|
| ✅ #46 | **PluginContext::register_tool()** — Register tools with name, toolset, schema, handler, override flag (Phase 1: tracks registration; Phase 2: integrates with ToolRegistry) |
| ✅ #46 | **PluginContext::register_hook()** — Register lifecycle hook callbacks |
| ✅ #46 | **PluginContext::register_command()** — Register slash commands (in-session `/cmd` with handler, description, args_hint) |
| ✅ #46 | **PluginContext::register_cli_command()** — Register CLI subcommands (terminal `hermes subcmd` style) |
| ✅ #46 | **PluginContext::register_skill()** — Register plugin skills with qualified names (plugin:name) |
| ❌ | **PluginContext::register_platform()** — Register gateway platform adapters |
| ✅ #50 | **PluginContext::inject_message()** — Inject messages into conversation (interrupt mid-turn or queue when idle) |
| ❌ | **PluginContext::dispatch_tool()** — Dispatch tool calls through registry with parent agent context |
| ❌ | **PluginContext::llm** — Host-owned LLM facade for trusted plugins (gated by config) |
| ❌ | **PluginContext::register_context_engine()** — Replace built-in context compression |
| ✅ #46 | **PluginContext::register_image_gen_provider()** — Add image generation backends (Phase 2 stub; full provider integration Phase 3) |
| ✅ #46 | **PluginContext::register_video_gen_provider()** — Add video generation backends (Phase 2 stub; full provider integration Phase 3) |
| ✅ #46 | **PluginContext::register_web_search_provider()** — Add web search/extract backends (Phase 2 stub; full provider integration Phase 3) |
| ✅ #46 | **PluginContext::register_browser_provider()** — Add cloud browser backends (Phase 2 stub; full provider integration Phase 3) |

### 6. Hook System — 17 Lifecycle Hooks

| Severity | Description |
|----------|-------------|
| **priority-critical** | ObenAgent has a basic callback system but NOT a hook system. Hermes has 17 hook types that fire at specific lifecycle points, with `invoke_hook()` that calls all registered callbacks per hook, wrapping each in try/except. |

| Status | Description |
|--------|-------------|
| ✅ #46 | **Hook types** — `pre_tool_call`, `post_tool_call`, `transform_terminal_output`, `transform_tool_result`, `transform_llm_output`, `pre_llm_call`, `post_llm_call`, `pre_api_request`, `post_api_request`, `on_session_start`, `on_session_end`, `on_session_finalize`, `on_session_reset`, `subagent_stop`, `pre_gateway_dispatch`, `pre_approval_request`, `post_approval_response` |
| ✅ #46 | **invoke_hook()** — Call all callbacks for a hook name, pass kwargs, wrap in try/except (catch_unwind), collect non-None results |
| ✅ #46 | **pre_tool_call blocking** — First `{"action": "block", "message": "..."}` wins; tool whitelisting per-thread |
| ✅ #46 | **pre_llm_call context injection** — Return dict/string to inject context into user message (preserves prompt cache) |
| ✅ #46 | **transform_llm_output** — Replace LLM response text (first non-None wins; for vocabulary/personality transformation) |

### 7. Plugin Tool Whitelisting

| Severity | Description |
|----------|-------------|
| **priority-high** | No thread-local tool whitelist. Hermes uses `set_thread_tool_whitelist()` / `clear_thread_tool_whitelist()` to restrict which tools a sub-agent thread can call. |

| Status | Description |
|--------|-------------|
| ✅ #46 | **Thread-local tool whitelist** — Per-thread allowed tool set, enforced via `get_pre_tool_call_block_message()` |

### 8. Plugin Slash Commands

| Severity | Description |
|----------|-------------|
| **priority-high** | No plugin slash command system. Hermes plugins register `/cmd` handlers that are available in CLI and gateway sessions. Supports async handlers with 30s timeout. |

| Status | Description |
|--------|-------------|
| ✅ #50 | **Plugin slash command registry
| ❌ | **Async command handling** — Await async handlers, with 30s timeout, threaded fallback when no running loop |
| ❌ | **Command resolution** — `resolve_command()` with conflict check against built-in commands |

### 9. Plugin Skills — Qualified Names

| Severity | Description |
|----------|-------------|
| **priority-medium** | No plugin skill system. Hermes registers skills with qualified names (`plugin_name:skill_name`) that are resolvable via `skill_view()` but NOT in the flat system prompt index (opt-in explicit loads only). |

| Status | Description |
|--------|-------------|
| ❌ | **Plugin skill registry** — Qualified name → {path, plugin, bare_name, description} |
| ❌ | **Skill lookup** — `find_plugin_skill(qualified_name)`, `list_plugin_skills(plugin_name)` |

### 10. Plugin CLI Commands

| Severity | Description |
|----------|-------------|
| **priority-medium** | No plugin CLI command registration. Hermes plugins can register `hermes subcmd` CLI subcommands via `register_cli_command(name, help, setup_fn, handler_fn, description)`. |

| Status | Description |
|--------|-------------|
| ✅ #50 | **Plugin CLI command registry

### 11. Provider Plugin System — Backend Abstraction

| Severity | Description |
|----------|-------------|
| **priority-high** | No pluggable provider system. Hermes uses plugins to register alternative backends for: image_gen, video_gen, web_search, browser, memory, context_engine, model_provider. Each provider type has its own registry. |

| Status | Description |
|--------|-------------|
| ❌ | **Image gen provider registry** — `ImageGenProvider` trait with `name`, `display_name`, `is_available()`, `list_models()`, `default_model()`, `get_setup_schema()`, `generate()` |
| ❌ | **Video gen provider registry** — `VideoGenProvider` trait similar to ImageGenProvider |
| ❌ | **Web search provider registry** — `WebSearchProvider` for search/extract backends |
| ❌ | **Browser provider registry** — `BrowserProvider` for cloud browser backends |
| ✅ | **Memory provider registry** — `MemoryProvider` exclusive provider (one active at a time) | `MemoryManager::add_provider()` enforces builtin + 1 external max |
| ❌ | **Context engine registry** — `ContextEngine` exclusive engine (one active at a time, replaces built-in) |
| ❌ | **Model provider registry** — `ProviderProfile` for custom model providers |

### 12. Plugin Configuration & Enable/Disable

| Severity | Description |
|----------|-------------|
| **priority-high** | No plugin configuration. Hermes reads `plugins.enabled` (opt-in allow-list) and `plugins.disabled` (deny-list) from config.yaml. Bundled backends/platforms auto-load; user plugins require opt-in. |

| Status | Description |
|--------|-------------|
| ❌ | **Enabled plugins config** — `plugins.enabled` allow-list (None = nothing enabled) |
| ❌ | **Disabled plugins config** — `plugins.disabled` deny-list (always enforced) |
| ❌ | **Plugin load gating** — Bundled backend/platform auto-load; user plugins gated by enabled list; exclusive handled by category discovery |

### 13. Plugin Introspection & Debugging

| Severity | Description |
|----------|-------------|
| **priority-low** | No plugin introspection. Hermes has `list_plugins()` returning name, key, kind, version, description, source, enabled, tool/hook/command counts, errors. Also has `HERMES_PLUGINS_DEBUG=1` for verbose discovery logging. |

| Status | Description |
|--------|-------------|
| ✅ #50 | **Plugin introspection
| ✅ #50 | **Debug logging

### 14. Plugin Command Toolsets (TUI Integration)

| Severity | Description |
|----------|-------------|
| **priority-low** | No plugin toolset integration. Hermes groups plugin tools by toolset and shows them in the TUI with plugin attribution (`🔌 Toolset`). |

| Status | Description |
|--------|-------------|
| ❌ | **Plugin toolset grouping** — Group plugin tool names by toolset, map back to owning plugin |

---

## Migration Notes

### Rust Implementation Considerations

1. **Plugin discovery** needs to scan directories for `plugin.yaml` and dynamically load modules. In Rust, this means either:
   - Using `libloading` to load `.so`/`.dylib` files (true dynamic plugins)
   - A simpler approach: compile plugin definitions as Rust modules in the workspace
   - A hybrid: scan for YAML manifests but load plugins via pre-compiled binaries or WASM

2. **Plugin manifest** can be parsed with `serde_yaml` — the YAML schema is well-defined.

3. **Hook system** maps well to Rust's callback patterns — `Vec<Box<dyn Fn(&HooksArgs) -> Result<...>>>` per hook type.

4. **Provider system** maps naturally to Rust traits — `ImageGenProvider`, `WebSearchProvider`, etc. as traits with registered implementations.

5. **PluginContext** is a facade pattern — can be a `struct` with methods that delegate to the PluginManager.

6. **Thread-safe plugin loading** needs `Arc<Mutex<>>` for shared state across plugin registrations.

### Recommended Phasing

**Phase 1 (core infrastructure):** PluginManager, PluginManifest, discovery scanning, PluginContext (basic tool + hook registration), hook types + invoke_hook

**Phase 2 (provider system):** Provider traits (image_gen, web_search, memory, context_engine), provider registry, provider selection via config

**Phase 3 (extensibility):** Plugin skills, slash commands, CLI commands, inject_message, tool whitelisting

**Phase 4 (polish):** Plugin introspection, debug logging, toolset grouping, pip entry-point scanning

---

## Relationship to Existing ObenAgent Architecture

The plugin system sits **above** the existing agent engine:

```
┌─────────────────────────────────────────────────┐
│              Plugin System                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │Discovery │ │Lifecycle │ │  PluginContext    │ │
│  │(4 sources)│ │(load/enable)│ │(register hooks, │ │
│  └──────────┘ └──────────┘ │  tools, skills)  │ │
└────────────────────────────┼──────────────────┼┘
                             │                   │
┌────────────────────────────┼───────────────────┼──────────────────┐
│                            ▼                   │                  │
│  ┌─────────────┐ ┌─────────────┐  ┌──────────┐ │  ┌────────────┐  │
│  │ Hook System │ │Provider     │  │ Plugin   │ │  │ Plugin     │  │
│  │ (17 hooks)  │ │Registry     │  │ Skills   │ │  │ Slash      │  │
│  └─────────────┘ └─────────────┘  └──────────┘ │  │ Commands   │  │
│                                                │  └────────────┘  │
│  ┌─────────────┐                              │                  │
│  │Tool         │                              │  ┌────────────┐  │
│  │Whitelist   │                              │  │ CLI        │  │
│  └─────────────┘                              │  │ Commands   │  │
└───────────────────────────────────────────────┼────────────────┼┘
                                                │                │
┌───────────────────────────────────────────────┼────────────────┼──┐
│                                               ▼                │  │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ ┌───────────────┐  │  │
│  │oben-agent│ │oben-tools│ │ oben-sessions│ │oben-gateway   │  │  │
│  │(conversation│ │(tool    │ │(memory,     │ │(platform      │  │  │
│  │  loop)   │ │registry)  │ │  FTS5)      │ │  adapters)    │  │  │
│  └──────────┘ └──────────┘ └──────────────┘ └───────────────┘  │  │
└────────────────────────────────────────────────────────────────┴──┘
```

The plugin system is a **crate-level addition** — likely `oben-plugin` or `oben-extensions` — that imports and extends the existing crates.

---

## Implementation Tracking

| # | Issue | Description | Severity | Parity Row |
|---|-------|-------------|----------|------------|
| 1 | — | **Plugin infrastructure** — PluginManager, PluginManifest, discovery, loading, PluginContext | priority-critical | Rows 1-6 |
| 2 | — | **Hook system** — 17 hook types, invoke_hook, pre_tool_call blocking, context injection | priority-critical | Row 7 |
| 3 | — | **Provider traits** — ImageGenProvider, VideoGenProvider, WebSearchProvider, BrowserProvider, MemoryProvider, ContextEngine | priority-high | Row 11 |
| 4 | — | **Provider registry** — Register/lookup pluggable backends, config-driven selection | priority-high | Row 11 |
| 5 | — | **Plugin config** — enabled/disabled lists, load gating by kind/source | priority-high | Row 12 |
| 6 | — | **Plugin slash commands** — /cmd registration, async handling, TUI integration | priority-high | Row 8 |
| 7 | — | **Tool whitelisting** — Thread-local per-thread tool restriction | priority-high | Row 7 |
| 8 | — | **Plugin skills** — Qualified names, lookup, system prompt integration | priority-medium | Row 9 |
| 9 | — | **Plugin CLI commands** — hermes subcmd registration | priority-medium | Row 10 |
| 10 | — | **Plugin introspection** — list_plugins, debug logging | priority-low | Row 13 |
| 11 | — | **Plugin toolsets** — TUI grouping, attribution | priority-low | Row 14 |
