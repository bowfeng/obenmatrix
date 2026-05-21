# ADR-0001: Plugin System Architecture

**Date:** 2026-05-20  
**Status:** Accepted  
**Related:** Plugin system design discussed 2026-05-20  

## Context

Hermes Agent has a comprehensive plugin system that lets users extend the agent with new tools, commands, context engines, and behaviors — including swapping out core subsystems. ObenAgent is porting the full Hermes functionality and needs equivalent extensibility.

The plugin system must support:
- Adding new tools beyond the built-in 15
- Replacing the default `ContextCompressor` with custom context engines
- Registering CLI subcommands and slash commands
- Registering skills
- Lifecycle hooks for events like `post_tool_call`, `on_session_end`
- Provider backends (image gen, video gen, web search, browser)

## Decision

ObenAgent will use a **trait-based plugin system** with no dynamic loading. Plugins are compiled Rust crates that register through a central `PluginManager`.

### Core Design

1. **`Plugin` trait** — each plugin implements this interface
   ```rust
   trait Plugin {
       fn name(&self) -> &str;
       fn register(&self, ctx: &mut PluginContext) -> Result<()>;
   }
   ```

2. **`PluginContext`** — registration API provided to plugins
   - `register_tool(Box<dyn Tool>)` — add tools to `ToolRegistry`
   - `register_hook(hook_name, callback)` — lifecycle callbacks
   - `register_command(name, handler)` — slash commands
   - `register_cli_command(name, help, setup_fn, handler_fn)` — CLI subcommands
   - `register_context_engine(Box<dyn ContextEngine>)` — replace default
   - `register_skill(name, path)` — agent skills
   - `register_provider(provider)` — for various backend providers

3. **`PluginManager`** — discovers, loads, and dispatches to plugins
   - Configuration-driven: `enabled`/`disabled` lists in `config.yaml`
   - Scans directories for plugin crates
   - Supports multiple plugin sources (bundled, user, project)

4. **Hook system** — `PluginManager::invoke_hook(hook_name, args)` returns `Result<Vec<HookResult>>`
   - Callbacks wrapped in try/except — a misbehaving plugin can't crash the agent
   - First error fails fast, or collect all results
   - `pre_tool_call` hooks can return `{"action": "block", "message": "..."}` to block tools

5. **Context engine trait** — abstract base class for context compression strategies
   ```rust
   trait ContextEngine: Send + Sync {
       fn name(&self) -> &str;
       fn update_from_response(&mut self, usage: TokenUsage);
       fn should_compress(&self, prompt_tokens: usize) -> bool;
       fn compress(&mut self, messages: &mut Vec<Message>, focus_topic: Option<&str>) -> Result<Vec<Message>>;
       fn on_session_start(&mut self, session_id: &str);
       fn on_session_end(&mut self, session_id: &str);
       fn on_session_reset(&mut self);
   }
   ```

### Plugin Sources

| Source | Path | Auto-load | Example |
|--------|------|-----------|---------|
| **Bundled** | `<repo>/plugins/<name>/` | Yes | `disk-cleanup`, `kanban` |
| **User** | `~/.oben/plugins/<name>/` | No (opt-in) | Third-party plugins |
| **Project** | `<cwd>/.oben/plugins/<name>/` | No (opt-in) | Project-specific plugins |

### Configuration

```yaml
# ~/.oben/config.yaml
plugins:
  enabled:
    - disk-cleanup
    - kanban
  disabled:
    - old-plugin
```

### Plugin Lifecycle

1. **Discovery** — `PluginManager::discover()` scans directories for `plugin.yaml` manifests
2. **Loading** — `PluginManager::load(plugin)` imports the crate and calls `register(ctx)`
3. **Registration** — Plugin calls `ctx.register_*()` methods to hook into the system
4. **Dispatch** — `PluginManager::invoke_hook(hook_name, args)` calls all registered callbacks
5. **Cleanup** — `PluginManager::unload(plugin)` removes registration

### Configuration-Driven Loading

- Bundled plugins auto-load (unless disabled)
- User/project plugins are opt-in via `plugins.enabled` in config
- Later sources override earlier ones on key collision (user > bundled, project > user)

### Rust-Idomatic Choices

1. **`Box<dyn Trait>`** instead of dynamic linking — compile-time safety with runtime flexibility
2. **`PluginContext`** pattern — plugins receive context object, register via methods (not direct access)
3. **`invoke_hook`** returns `Result<Vec<HookResult>>` — first error fails fast, or collect all results
4. **`discover_plugins`** — scan directories for plugin crates (like Python's `importlib`)

### Extension Points

| Hermes | Rust equivalent | Purpose |
|--------|----------------|---------|
| ContextEngine | `ContextEngine` trait | Replace/extend context compression |
| register_tool | `ToolRegistry` | Add new tools |
| register_hook | Hook system | Lifecycle callbacks |
| register_command | CLI commands | Custom slash commands |
| register_skill | Skills | Agent skills |
| register_provider | Provider registry | Image gen, video gen, web search, browser backends |

## Consequences

### Positive
- **Type safety** — compile-time trait bounds, runtime type checks via downcasting
- **Zero-cost** — no virtual calls unless hooks are registered
- **No FFI** — all plugins are Rust crates, no unsafe dynamic loading
- **Familiar pattern** — similar to Python's plugin system but with Rust's guarantees
- **Locality** — plugin implementation concentrated in one crate, tests isolated

### Negative
- **Compile-time coupling** — plugins must be compiled crates, not hot-swappable `.py` files
- **Dependency management** — each plugin crate needs its own `Cargo.toml` with dependencies
- **No runtime discovery** — must scan directories at startup, can't discover new plugins without restart
- **Versioning** — plugin API versioning needed to avoid breaking changes

### Trade-offs

| Choice | Rationale | Alternative considered |
|--------|-----------|----------------------|
| Trait objects (`Box<dyn Trait>`) | Type-safe, no FFI needed | Dynamic linking with `libloading` |
| Compiled crates | Compile-time safety, type checking | Hot-swappable modules |
| Central `PluginManager` | Single source of truth for hooks | Distributed registration |
| Configuration-driven | User can enable/disable without recompiling | CLI flags or env vars |

## Implementation Phases

**Phase 1: Core plugin system**
- `Plugin` trait
- `PluginContext` for registration
- `PluginManager` for lifecycle
- Hook system

**Phase 2: Extension points**
- Context engine trait (replace `ContextCompressor`)
- Tool registration (extend `ToolRegistry`)
- Command registration (CLI commands)
- Skill registration

**Phase 3: Plugin loading**
- Configuration-driven (YAML config)
- Directory scanning for plugin crates
- Entry-point plugin discovery (like Python's `entry_points`)

## Related Decisions

- **ADR-XXX: Context Engine Architecture** — defines `ContextEngine` trait location and interface
- **ADR-XXX: Transport Architecture** — defines provider trait for pluggable backends
- **ADR-XXX: Platform Adapters** — defines gateway platform registration

## References

- [Hermes Agent plugin system](../hermes-agent/plugins/context_engine/__init__.py)
- [Hermes Agent plugin discovery](../hermes-agent/hermes_cli/plugins.py)
- [Hermes Agent plugin context](../hermes-agent/hermes_cli/plugins.py) (PluginContext)
- [Hermes Agent hook system](../hermes-agent/hermes_cli/plugins.py) (invoke_hook)
