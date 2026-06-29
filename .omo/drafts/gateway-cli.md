---
slug: gateway-cli
status: awaiting-approval
intent: unclear
pending-action: write .omo/plans/gateway-cli.md
approach: Add `gateway` subcommand to oben-cli with start/stop/status lifecycle management (cron-matching spawner) + `gateway setup` wizard for message platform credentials.

---

# Draft: gateway-cli

## Components (topology ledger)

| # | Component | Status | Evidence Path |
|---|-----------|--------|---------------|
| 1 | CLI subcommand skeleton (`cli.rs` Commands enum, `dispatch.rs` handler) | active | `oben-cli/src/cli.rs:18-76`, `oben-cli/src/dispatch.rs:35-81` |
| 2 | Gateway lifecycle spawner (start/stop/status — PID tracking, build-on-demand) | active | `oben-cli/src/dispatch.rs:900-1019` (cron pattern reference) |
| 3 | `GatewayConfig` struct + serialization in `oben-config` | active | `oben-config/src/config.rs:290-320` |
| 4 | `gateway setup` wizard (interactive credential entry for qq_bot, telegram stubs) | active | `oben-config/src/wizard.rs:1-134`, `oben-config/src/config.rs:314-326` |
| 5 | `oben-gateway/src/main.rs` currently standalone — no CLI integration | risk | `oben-gateway/src/main.rs:109-199` |

## Open assumptions (announced defaults)

| Assumption | Adopted Default | Rationale | Reversible? |
|------------|-----------------|-----------|-------------|
| Lifecycle commands | `start`/`stop`/`status` — mirror cron pattern | Already proven in codebase (`cron_start`, `cron_info`, `is_cron_running`) | Yes |
| Gateway binary | `oben-gateway` — same binary, spawner builds on-demand | `oben-gateway/src/main.rs` already exists as standalone binary | Yes |
| Setup wizard scope | qq_bot interactive (full), telegram/discord/slack/whatsapp stub/enabled toggle | Only qq_bot has real adapter code; others are in `GatewayConfig` struct but not implemented | Yes |
| PID state path | `~/.config/obenalien/gateway.pid` | Consistent with cron's `~/.config/obenalien/cron.pid` pattern | Yes |
| Config path | `~/.config/obenalien/config.yaml` | Already the canonical config path used throughout | Yes |
| Build behavior | Spawner runs `cargo build --package oben-gateway` if binary not found | Same as cron (see `dispatch.rs:949-963`) | Yes |
| Test strategy | TDD for lifecycle spawner logic; wizard tested via unit + roundtrip config save | Zero human intervention requirement | Yes |

## Findings (cited)

1. **CLI structure** (`oben-cli/src/cli.rs:18-76`): Commands enum defined with clap derive. New `Gateway { action: Option<GatewayCommand> }` variant needed, similar to `Cron`, `Goals`, `Sessions`.

2. **Dispatch handler** (`oben-cli/src/dispatch.rs:56-71`): Match arm for cron demonstrates the pattern for sub-subcommand dispatch. Similar pattern for gateway.

3. **Lifecycle management** (`oben-cli/src/dispatch.rs:939-1018`): `cron_start()` builds binary on-demand, spawns background process, waits for PID file, handles stale PID. **Exact pattern to replicate** for gateway.

4. **Setup wizard** (`oben-config/src/wizard.rs`): Interactive config wizard using `dialoguer::Select` and `dialoguer::Input`. Template for gateway setup wizard.

5. **GatewayConfig struct** (`oben-config/src/config.rs:314-320`): Already has `telegram`, `discord`, `slack`, `whatsapp`, `qq_bot` — wizard writes to these fields.

6. **QQBotConfig struct** (`oben-config/src/config.rs:282-311`): Has `enabled`, `app_id`, `app_secret`, `intents`, `shard`, `sandbox` — wizard prompts for required fields.

7. **oben-gateway main** (`oben-gateway/src/main.rs:109-199`): `#[tokio::main]` async binary. Reads config, creates dispatcher/router/adapters, calls `gateway.start_blocking()`. Currently standalone.

8. **oben-gateway workspace member** (`Cargo.toml:9`): Already in workspace members. `oben-cli` already has `oben-gateway` in dependencies (`oben-cli/Cargo.toml:20`).

## Decisions (with rationale)

1. **Don't rename above-cli — create `gateway` subcommand.** The existing workspace binary `obenalien` (the root package) already re-exports `oben-cli`. Adding `Commands::Gateway` is the minimum-change approach. The `oben-gateway` binary stays as is for direct `cargo run` usage.

2. **Spawner vs. in-process.** Spawn as background process (same as cron), NOT in-process. The gateway is a long-running async blocking server — running it synchronously inside the CLI's tokio runtime would cause shutdown ordering issues and prevent graceful CLI exit.

3. **Setup wizard — interactive prompts.** Same UX model as existing `oben setup`: `dialoguer::Select` for platform choice, `dialoguer::Input` for credentials. Write to existing `GatewayConfig` fields without requiring full YAML manual editing.

## Scope IN

- `oben gateway start|stop|status` lifecycle commands in `oben-cli`
- `oben gateway setup` interactive wizard for message platform credentials
- `oben-gateway` binary build-on-demand if missing
- PID file tracking for gateway process
- Config persistence to `~/.config/obenalien/config.yaml`
- BDD tests for lifecycle spawner (start/stop/status logic)
- BDD tests for setup wizard config save/load roundtrip

## Scope OUT (Must NOT have)

- Gateway in-process integration (spawn is external process only)
- Adding new platform adapters (telegram, discord, slack, whatsapp implementation)
- Gateway health check endpoint or HTTP API
- Gateway log management or log forwarding
- Auto-update/self-healing gateway process
- `oben gateway logs` — no log streaming

## Open questions

None — best-practice defaults adopted above. User can veto at the gate.

## Approval gate

status: awaiting-approval
pending-action: write .omo/plans/gateway-cli.md
approach: See TL;DR section above
