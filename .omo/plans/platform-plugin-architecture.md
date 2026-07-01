# Platform Plugin Architecture

## Problem

Users want to write third-party messaging platform adapters (e.g. Signal, Matrix, IRC, WeChat Work) that integrate seamlessly into the Oben gateway without forking the entire codebase.

## Current Architecture (Before Plugins)

```
oben-platform-sdk/       # PlatformAdapter trait, shared types
oben-config/             # TelegramConfig, DiscordConfig, SlackConfig, WhatsAppConfig — ALL HARDCODED
oben-gateway/            # PlatformRegistry, each adapter has #[cfg(feature = "...")]
oben-gateway/src/main.rs # Hardcoded if/elif chain creating factories
oben-cli/                # Hardcoded platform list in setup wizard
```

The `GatewayConfig` struct in `oben-config` has five hardcoded fields:
```rust
pub struct GatewayConfig {
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub slack: Option<SlackConfig>,
    pub whatsapp: Option<WhatsAppConfig>,
    pub qq_bot: Option<QQBotConfig>,
}
```

Each adapter is a `#[cfg(feature = "...")]` conditional compilation block in `main.rs`. The CLI setup wizard hardcodes a 5-item select menu. None of this is extensible.

## Design Decisions

### 1. Compile-time-first, runtime-scan second

We reject `dlopen`/`.so` dynamic loading in favor of **compile-time registration**. Two reasons:

1. Rust ABI stability concerns across versions
2. The project already has `Cargo.toml` — no new distribution mechanism needed
3. Type errors are caught at compile time, not runtime

### 2. Generic factory interface (already exists)

`PlatformFactory` trait is already generic — `PlatformRegistry::register()` takes `Box<dyn PlatformFactory>`. No changes needed here.

### 3. Config-driven discovery

Platform declarations move from **Rust structs** into **YAML config**. Config defines intent; Rust implements it.

### 4. Feature-gated plugins

Plugins are declared as optional crates in the project's `Cargo.toml`. Each plugin maps to a `Cargo.toml` feature gate. The feature gate becomes the bridge between config declaration and implementation.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│ ~/.obenalien/config.yaml                                │
│   gateway:                                              │
│     platforms:                                          │
│       custom_bot:                                       │
│         type: "custom_bot"                              │
│         name: "Custom Bot"                              │
│         feature: "custom-bot"                           │  ← maps to Cargo.toml
│         enabled: true                                   │
│         config:                                         │
│           api_key: "..."                                │
│                                                           │
│       telegram:                                         │
│         enabled: true                                   │  ← built-in
│         token: "..."                                    │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│ project/Cargo.toml                                      │
│   [dependencies]                                        │
│   oben-platform-sdk = { path = "../oben-platform-sdk" } │
│   oben-custom-bot = { path = "../../oben-custom-bot",   │  ← optional plugin
│                       optional = true }                 │
│                                                           │
│   [features]                                            │
│   custom-bot = ["dep:oben-custom-bot"]                  │
│                                                           │
│   [workspace]                                           │
│   members = ["oben-gateway", "oben-custom-bot"]          │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│ ../../oben-custom-bot/Cargo.toml (the plugin)          │
│   [package]                                             │
│   name = "oben-custom-bot"                              │
│                                                           │
│   [dependencies]                                        │
│   oben-platform-sdk = { path = "../oben-platform-sdk" } │
│   async-trait = "0.1"                                   │
│   tracing = "0.1"                                       │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│ ../../oben-custom-bot/src/lib.rs (the plugin)          │
│ use oben_platform_sdk::*;                               │
│                                                           │
│ pub struct CustomBotAdapter { ... }                     │
│                                                           │
│ #[async_trait]                                          │
│ impl PlatformAdapter for CustomBotAdapter {             │
│     fn name(&self) -> &str { "custom_bot" }             │
│     async fn listen(&mut self) -> Result<()> { ... }    │
│     async fn stop(&mut self) { ... }                    │
│     async fn send(&self, msg: OutgoingMessage)          │
│         -> Result<()> { ... }                           │
│     async fn health_check(&self) -> bool { ... }        │
│ }                                                       │
│                                                           │
│ pub struct CustomBotFactory { ... }                     │
│                                                           │
│ impl PlatformFactory for CustomBotFactory {             │
│     fn spawn(&self) -> tokio::task::AbortHandle { ... } │
│ }                                                       │
└─────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Config-driven platform declarations (no runtime change)

**Changes to `oben-config/src/config.rs`:**

Add a `platforms` field to `GatewayConfig` that holds a map of named platform declarations:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    // --- Built-in (existing, unchanged) ---
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub slack: Option<SlackConfig>,
    pub whatsapp: Option<WhatsAppConfig>,
    pub qq_bot: Option<QQBotConfig>,

    // --- Plugin platforms (new) ---
    #[serde(default)]
    pub platforms: HashMap<String, PlatformDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformDeclaration {
    /// Display name for CLI wizard
    pub name: String,
    /// Feature gate in Cargo.toml (e.g. "custom-bot")
    pub feature: String,
    /// Whether this platform is enabled
    pub enabled: bool,
    /// Arbitrary key-value config. Keys/values vary by plugin.
    #[serde(default)]
    pub config: HashMap<String, String>,
}
```

**Changes to `oben-cli/src/dispatch.rs` (`gateway_setup`):**

Merge built-in platforms with `config.gateway.platforms` for the setup menu:

```rust
fn gateway_setup() -> Result<()> {
    let platforms = {
        let mut items = vec![
            "QQ Bot (Tencent)",
            "Telegram",
            "Discord",
            "Slack",
            "WhatsApp",
        ];
        // Append config-declared plugin platforms
        if let Some(gw) = &config.gateway {
            for (name, decl) in &gw.platforms {
                if decl.enabled {
                    items.push(&format!("{} *", decl.name));
                }
            }
        }
        items
    };
    // ... rest of setup unchanged
}
```

**Changes to `oben-gateway/src/main.rs`:**

The existing hardcoded `if cfg!(feature = "...")` blocks remain for built-in platforms. Add a helper function that reads `config.gateway.platforms` and registers plugins:

```rust
fn register_plugins(
    registry: &mut PlatformRegistry,
    platforms: &HashMap<String, PlatformDeclaration>,
    dispatcher: Arc<Dispatcher>,
    response_router: Arc<ResponseRouter>,
) {
    // Example: a future #2778-gateway-plugins.rs module
    #[cfg(feature = "custom-bot")]
    if let Some(decl) = platforms.get("custom_bot") {
        if decl.enabled {
            let factory = CustomBotFactory::new(
                parse_plugin_config(decl.config.clone()),
                dispatcher.clone(),
                response_router.clone(),
            );
            registry.register("custom_bot", &decl.name, factory);
        }
    }
}
```

### Phase 2: Plugin project template + documentation

Create `oben-alien-platform-template/` — a `cargo new` starter crate:

```
oben-alien-platform-template/
├── Cargo.toml
├── src/
│   └── lib.rs          # Full PlatformAdapter impl skeleton
│   └── factory.rs      # PlatformFactory impl skeleton
└── README.md           # Step-by-step: add to workspace, enable feature, rebuild
```

### Phase 3: Plugin discovery scan (UX polish)

```bash
$ oben gateway plugin ls
```

This scans `~/.config/obenalien/config.yaml` for declared but unknown plugins, and prompts:

```
The following platforms are declared in config but the plugin
is not installed. Install them to your workspace?

  • WeChat Work (feature: wechat-work)
  • Signal (feature: signal)

Install these plugins? [Y/n]
```

On approval:
1. Prompt for plugin crate URL/path
2. Add to `Cargo.toml` as `[dependencies]` with `optional = true`
3. Add feature gate under `[features]`
4. Add `members = [...]` to `[workspace]`
5. Rebuild

### Phase 4: Runtime plugin scan (future)

Optional future addition: scan `~/.obenalien/plugins/*.toml` for plugin metadata at startup, enabling hot-swap of platform implementations without recompile. Requires `dlopen`/`.dylib` support — mark as advanced.

## Risks & Tradeoffs

| Risk | Mitigation |
|------|-----------|
| Users must fork/modify Cargo.toml | Template repo + `oben gateway plugin add` automates it |
| Feature gates must be kept in sync | Validation: warn at startup if `config.gateway.platforms[].feature` doesn't match any `[features]` entry |
| Config `HashMap<String, String>` loses type safety | Acceptable for Phase 1 — plugins define their own config schema. Future: per-plugin config validation via custom Deserialize |
| CLI setup needs to handle plugin config input | Setup wizard delegates to plugin-specific input if plugin is compiled in (callable from a trait method) |

## Summary

| Component | Change Needed |
|-----------|--------------|
| `oben-platform-sdk` | ✅ Publish as crate. Add helper types (docstrings, examples). No breaking changes. |
| `oben-config` | Add `platforms: HashMap<String, PlatformDeclaration>` to `GatewayConfig` |
| `oben-gateway` | Add `#[cfg(feature = "...")]` blocks for each plugin crate's factory registration |
| `oben-gateway/src/main.rs` | Call `register_plugins()` alongside existing built-in block |
| `oben-cli` | Merge built-in + declared plugins into setup wizard menu |
| `oben-platform-template/` | New crate in workspace as reference |

## Files to create/modify

**New files:**
1. `oben-alien-platform-template/Cargo.toml` — plugin starter
2. `oben-alien-platform-template/src/lib.rs` — full PlatformAdapter skeleton
3. `docs/architecture/PLUGIN-ARCHITECTURE.md` — this document

**Modified files:**
1. `oben-config/src/config.rs` — add `PlatformDeclaration` struct + `platforms` field
2. `oben-gateway/src/main.rs` — add plugin registration function call
3. `oben-gateway/src/platform.rs` — add plugin registration helper trait bound if needed
4. `oben-cli/src/dispatch.rs` — merge built-in + plugin platforms in setup wizard
