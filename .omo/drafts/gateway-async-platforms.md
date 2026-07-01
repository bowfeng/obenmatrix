---
slug: gateway-async-platforms
status: awaiting-approval
intent: clear
pending-action: present brief to user for approval
approach: Platform discovery + async concurrent startup + health state registry
---

# Draft: gateway-async-platforms

## Components (topology ledger)
| id | outcome | status | evidence path |
|---|---|---|---|
| main.rs:platform_init | 5 platforms defined in config, 1 wired by hand | active | main.rs:183-209 |
| gateway.rs:start_platforms | QQ Bot only, sequential | active | gateway.rs:83-121 |
| router.rs:ResponseRouter | HashMap, supports multi-platform | active | router.rs:14 |
| platform.rs:PlatformAdapter | trait with health_check() | active | platform.rs:29-44 |
| config.rs:GatewayConfig | 5 platform fields (tg/dc/sl/wa/qq) | active | config.rs:323-330 |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|---|---|---|---|
| Platform registry concurrency | Arc<RwLock<HashMap>> | reads don't block writes on health checks | yes |
| Adapter discovery pattern | config-driven iteration | no new config format needed | yes |
| Error handling | log + continue | gateway stays up for other platforms | yes |

## Findings (cited - path:lines)
- **main.rs:183-209**: Only QQ Bot is wired. Telegram, Discord, Slack, WhatsApp are in GatewayConfig but never constructed or started. Each new platform requires manual code editing.
- **gateway.rs:83-121**: `start_platforms()` has a single QQ Bot branch. No loop over platforms.
- **router.rs:14**: ResponseRouter already uses `HashMap<String, Box<dyn PlatformAdapter>>` — inherently supports multi-platform routing.
- **platform.rs:29-44**: PlatformAdapter trait has `health_check()` method already — no trait changes needed.
- **config.rs:323-330**: GatewayConfig has `telegram`, `discord`, `slack`, `whatsapp`, `qq_bot` fields — all Option<config>.
- **dispatcher.rs:225**: Response routing hardcodes `"qq_bot"` string — this is per-session routing, not startup. Should use session's platform field later (deferred).

## Decisions (with rationale)
1. **State tracking via Arc<RwLock<HashMap>>>**: Lightweight, no extra crate dependency
2. **PlatformStatus enum**: Idle → Connecting → Running | Failed(String) — covers all lifecycle states
3. **Per-platform try-catch in main.rs**: Failures are logged and stored; other platforms continue
4. **No config changes needed**: Use existing `GatewayConfig` fields + `enabled` flag per platform
5. **platform_handles as HashMap<String, AbortHandle>**: keyed by platform name for easy lookup during shutdown

## Scope IN
- PlatformState enum + PlatformInfo struct
- PlatformRegistry inside Gateway
- Platform discovery function in main.rs
- Concurrent adapter construction
- start_platforms() iteration over config
- Health status query

## Scope OUT (Must NOT have)
- Implementing Telegram/Discord/Slack/WhatsApp adapters
- Webhook-to-agent routing changes
- CLI commands for platform management
- WebSocket auto-reconnect (logging failure state is enough)
- Pluggable platform discovery from external files

## Open questions
None — all decisions covered by defaults.

## Approval gate
status: awaiting-approval
pending action: present brief to user
