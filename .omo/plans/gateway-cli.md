# gateway-cli - Work Plan

## TL;DR (For humans)

**What you'll get:** Two new CLI capabilities — `oben gateway start/stop/status` to manage the gateway server lifecycle, and `oben gateway setup` to interactively configure message platform credentials (QQ Bot, Telegram stubs).

**Why this approach:** Mirrors the proven cron lifecycle pattern (spawn external binary, PID tracking, build-on-demand) so the implementer is reusing battle-tested code. The setup wizard follows the same `dialoguer` pattern as the existing `oben setup` wizard.

**What it will NOT do:** Add new platform adapters (Telegram, Discord, etc.), implement health checks, manage gateway logs, or auto-restart a crashed gateway process.

**Effort:** Medium
**Risk:** Low — adds new CLI paths, no changes to gateway runtime logic
**Decisions I made for you:**
- Lifecycle = spawner (external process), NOT in-process — same as cron. Reversible.
- `oben gateway setup` prompts for qq_bot fully, and stub-enabled toggles for telegram/discord/slack/whatsapp. Reversible.
- PID file at `~/.config/obenalien/gateway.pid` — same convention as cron. Reversible.

Your next move: approve (say "yes" / "proceed"), or run a Momus high-accuracy review first.

---

> TL;DR (machine): <1 line - effort, risk, deliverables>

## Scope
### Must have
1. `oben gateway start/stop/status` lifecycle commands in cli.rs + dispatch.rs
2. Gateway spawner (build-on-demand, PID file, stale PID cleanup)
3. `oben gateway setup` interactive wizard for platform credentials
4. Config save/load roundtrip for gateway config fields
5. Failing tests → green tests (TDD) — spawner logic + wizard config roundtrip

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Gateway in-process integration (spawn is external process only)
- Adding new platform adapters (telegram, discord, slack, whatsapp implementation)
- Health check endpoint, HTTP API, log management, auto-restart
- Modifying `oben-gateway/src/main.rs` or `oben-gateway/src/gateway.rs`
- Modifying `oben-gateway/Cargo.toml` dependencies

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: TDD — write FAILING tests first, then implement until green
- Framework: existing `cargo test -p oben-cli --lib` for unit tests
- Evidence: `.omo/evidence/task-<N>-gateway-cli.txt` (test output logs)
- CI check: `cargo test -p oben-cli --lib` (must show all green)
- BDD tests: spawner start/stop/status + config save/load roundtrip

## Execution strategy
### Parallel execution waves
Waves 1-3 independent work. Wave 4 final verification only.

| Wave | Focus | Why |
|------|-------|-----|
| Wave 1 | CLI subcommand struct (cli.rs) | Pure type definitions, no deps on other waves |
| Wave 2 | Gateway spawner lifecycle (start/stop/status) | Depends on Wave 1 enum; blocks Wave 3 |
| Wave 3 | Gateway setup wizard | Depends on Wave 1 enum; independent of spawner |
| Wave 4 | Final verification (parallel: F1-F4) | After all implementation |

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T0 (add deps) | - | T3 | T1, T2 |
| T1 (CLI enum) | - | T2, T3 | T0 |
| T2 (Spawner) | T1 | - | T0, T3 |
| T3 (Setup wizard) | T1, T0 | - | T2 |
| T4 (Config tests) | T1, T3 | - | T2 |
| T5 (Spawner tests) | T2 | - | T3, T4 |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
**Prerequisite (NOT a todo — do first):** Add `dialoguer = { workspace = true }` to `oben-cli/Cargo.toml`. `dialoguer` is already in workspace deps (`Cargo.toml:57`). This is needed by `oben-config/src/wizard.rs` which the gateway setup will call. Also add `dirs = { workspace = true }` to `oben-cli/Cargo.toml` for path resolution.

- [x] 0. Add daemonize + dirs deps to oben-cli/Cargo.toml (daemonize="0.5" for process daemonization, dirs="5" for PID path resolution)
  What to do / Must NOT do: Add `dialoguer = { workspace = true }` and `dirs = { workspace = true }` to `[dependencies]` in `oben-cli/Cargo.toml`. Must NOT change any other dependency versions. Must NOT add any other dependencies.
  Parallelization: Pre-flight | Blocked by: - | Blocks: T3 (setup wizard needs dialoguer)
  References: `oben-cli/Cargo.toml:11-23` (current deps), `Cargo.toml:57` (`dialoguer = "0.11"` workspace dep), `Cargo.toml:53` (`dirs = "5"` workspace dep)
  Acceptance criteria: `cargo check -p oben-cli` succeeds. ` oben-config` already has dialoguer, so no transitive dep issues.
  QA scenarios: (1) happy: `cargo check -p oben-cli` compiles. (2) failure: verify no workspace dep version conflicts. Evidence: `.omo/evidence/task-0-gateway-cli.txt`
  Commit: N (intermediate — will squash into T1)

- [x] 1. Add Gateway subcommand enum to cli.rs
  What to do / Must NOT do: Add `GatewayCommand` enum with 4 variants (Start, Stop, Status, Setup) to cli.rs. Add `Gateway { action: Option<GatewayCommand> }` variant to `Commands` enum. Add `#[command(subcommand)]` to action field. Must NOT change any other existing commands.
  Parallelization: Wave 1 | Blocked by: - | Blocks: T2, T3
  References: `oben-cli/src/cli.rs:43` (Config command pattern - use as template), `oben-cli/src/cli.rs:66-75` (Goals pattern), `oben-cli/src/cli.rs:67-70` (Cron with subcommands pattern)
  Acceptance criteria: `cargo check -p oben-cli` compiles with no errors. The new variant matches the existing subcommand pattern exactly. `clap` derive works correctly.
  QA scenarios: (1) happy: `cargo check -p oben-cli` succeeds with exit 0. (2) failure: verify no existing Commands variants are broken (grep for `Commands::` in dispatch.rs to confirm all match arms still compile). Evidence: `.omo/evidence/task-1-gateway-cli.txt`
  Commit: Y | chore(cli): add GatewayCommand enum with start/stop/status/setup subcommands

- [x] 2. Add gateway dispatch handler to dispatch.rs
  What to do / Must NOT do: Add match arm for `Commands::Gateway { action }` in `run_cli()` dispatch. For `None` action, print help message listing available gateway commands. For `Some(action)`, route to `gateway_start()`, `gateway_stop()`, `gateway_status()`, `gateway_setup()` functions. Must NOT implement the actual logic yet.
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: -
  References: `oben-cli/src/dispatch.rs:42` (Config dispatch pattern), `oben-cli/src/dispatch.rs:56-71` (Cron dispatch pattern - template), `oben-cli/src/dispatch.rs:72-80` (Goals dispatch pattern)
  Acceptance criteria: `cargo check -p oben-cli` compiles. The match arm route to stub functions (which are implemented in T3/T5 context). Must NOT add any logic here — only routing.
  QA scenarios: (1) happy: compile check passes. (2) failure: verify no other match arms modified. Evidence: `.omo/evidence/task-2-gateway-cli.txt`
  Commit: N (intermediate — will amend with spawner in next todo)

- [x] 3. Implement gateway spawner (start/stop/status)
  What to do / Must NOT do: Implement `gateway_start()`, `gateway_stop()`, `gateway_status()` following EXACTLY the cron pattern at `dispatch.rs:889-1018`. PID file at `~/.config/obenalien/gateway.pid` (NOT home dir). Binary discovery: check `target/debug/oben-gateway`, `target/release/oben-gateway`, then `cargo build --package oben-gateway`. Start: spawn detached process (null stdio), wait for PID file up to 10s. Stop: read PID file, send SIGTERM, wait up to 5s, SIGKILL if needed. Status: check PID file + process existence via `kill -0`. Must NOT modify gateway binary itself.
  Parallelization: Wave 2 | Blocked by: T1+T2 | Blocks: -
  References: `oben-cli/src/dispatch.rs:889-1018` (cron lifecycle — EXACT pattern to copy and adapt), `oben-cli/src/dispatch.rs:107-114` (pid_path function), `oben-cli/src/dispatch.rs:916-936` (binary discovery), `oben-cli/src/dispatch.rs:939-987` (start with spawn + waitForPID)
  Acceptance criteria: (1) `gateway_start()` builds if binary missing, spawns, waits for PID file. (2) `gateway_status()` returns correctly for running/stopped/stale-pid states. (3) `gateway_stop()` sends SIGTERM and cleans up PID file. All functions follow the cron pattern's error handling (anyhow bail for unrecoverable errors, println for user-friendly messages).
  QA scenarios: (1) happy: `cargo test -p oben-cli --lib` passes. (2) failure: verify `is_cron_running` pattern is correctly adapted (not called incorrectly — rename to `is_gateway_running`). Verify PID clean on stale state. Evidence: `.omo/evidence/task-3-gateway-cli.txt`
  Commit: Y | feat(cli): add gateway start/stop/status lifecycle management

- [x] 4. Implement gateway setup wizard
  What to do / Must NOT do: Implement `gateway_setup()` function. Interactive wizard: (1) List enabled platforms (Select: qq_bot, telegram, discord, slack, whatsapp). (2) For qq_bot: prompt for app_id, app_secret, sandbox (bool toggle), intents (multi-select: guilds, c2c, group_at). (3) For telegram/discord/slack/whatsapp: prompt enabled=bool, token=input. (4) Save config using `AppConfig::save()`. Must NOT prompt for values that don't exist in `PlatformConfig` or `QQBotConfig` structs. Must NOT modify `oben-gateway` source files.
  Parallelization: Wave 3 | Blocked by: T1 | Blocks: -
  References: `oben-config/src/wizard.rs:1-134` (wizard pattern: Input + Select), `oben-config/src/config.rs:282-320` (QQBotConfig + PlatformConfig + GatewayConfig structs), `oben-config/src/config.rs:290-311` (QQBotConfig fields: enabled, app_id, app_secret, intents, shard, sandbox), `oben-config/src/config.rs:322-326` (PlatformConfig: enabled, token)
  Acceptance criteria: (1) Wizard prompts for qq_bot credentials, sets them on `GatewayConfig::qq_bot`. (2) Wizard can enable/disable telegram/stub platforms and set tokens. (3) `config.save()` persists to file. (4) On second run, existing values are shown as defaults. All fields use `dialoguer::` exactly like the existing wizard.
  QA scenarios: (1) happy: Wizard completes, config file has correct gateway section with enabled platform. (2) failure: verify config roundtrip — load saved config, assert gateway section matches user input. (3) empty token handling: user skips token, field remains None or empty as appropriate. Evidence: `.omo/evidence/task-4-gateway-cli.txt`
  Commit: Y | feat(cli): add gateway setup wizard for platform credential configuration

- [x] 5. Add BDD tests for gateway lifecycle
  What to do / Must NOT do: Write unit tests in `oben-cli/src/dispatch.rs` or a new `oben-cli/src/gateway_lifecycle.rs` test module. Tests: (1) `test_gateway_status_not_running` — PID file doesn't exist → status = "not running". (2) `test_gateway_status_stale_pid` — PID file has non-existent process → status = "stale PID, removed". (3) `test_gateway_start_builds_binary` — no binary exists → cargo build is invoked (mock the spawn). Must NOT test actual gateway process lifecycle (that's integration test scope — out of scope). Must NOT use `cargo test --workspace` (use `cargo test -p oben-cli --lib`).
  Parallelization: Wave 2 | Blocked by: T3 | Blocks: -
  References: `oben-cli/src/dispatch.rs:930-914` (cron test patterns — use similar assertions), `oben-cli/src/dispatch.rs:889-898` (cron_pid_path pattern)
  Acceptance criteria: All tests green with `cargo test -p oben-cli --lib`. At least 3 unit tests covering: not-running state, stale-pid detection, and start with binary existence check. Tests must use temp dirs for PID file paths (never write to real `~/.config/obenalien/`).
  QA scenarios: (1) happy: `cargo test -p oben-cli --lib` shows all gateway tests passing. (2) failure: verify temp PID paths don't leak to disk. (3) verify no workspace-wide test invocation. Evidence: `.omo/evidence/task-5-gateway-cli.txt`
  Commit: Y | test(cli): add BDD unit tests for gateway lifecycle management

- [x] 6. Add BDD tests for gateway setup config roundtrip
  What to do / Must NOT do: Write integration tests (in `oben-cli/tests/gateway_setup_test.rs` or `oben-config/src/config.rs` test module) for config save/load roundtrip with gateway config. Test: (1) Serialize AppConfig with GatewayConfig containing qq_bot credentials → deserialize → assert fields match. (2) Serialize with telegram+discord enabled → assert both are present. Must NOT test wizard UI interaction (dialoguer cannot be unit-tested in-process without tty). Must NOT test actual spawner + gateway binary.
  Parallelization: Wave 3 | Blocked by: T4 | Blocks: -
  References: `oben-config/src/config.rs:666-700` (existing roundtrip test — pattern to follow), `oben-config/src/config.rs:682-700` (test_config_yaml_roundtrip_with_gateway — use as template)
  Acceptance criteria: Test serializes `AppConfig` with `GatewayConfig { qq_bot: Some(QQBotConfig { enabled: true, app_id: "...", app_secret: "..." }), telegram: Some(PlatformConfig { enabled: true, token: Some("...") }) }` and asserts roundtrip deserialization preserves all fields.
  QA scenarios: (1) happy: `cargo test -p oben-config --lib` shows gateway roundtrip test passing. (2) failure: verify serialized YAML structure matches expected `~/.config/obenalien/config.yaml` format. Evidence: `.omo/evidence/task-6-gateway-cli.txt`
  Commit: Y | test(config): add BDD roundtrip tests for gateway config serialization

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [x] F1. Plan compliance audit — PASS (T4 fixed: setup wizard fully implemented with 5 platform helpers)
- [x] F2. Code quality review — PASS (F2 flagged issues only in oben-gateway/ which are pre-existing, out of scope)
- [x] F3. Real manual QA — PASS (oben gateway start/stop/status/setup CLI commands verified)
- [x] F4. Scope fidelity — PASS (only 5 plan-specified files modified, 0 scope creep)

## Commit strategy
Single squash commit on feature branch: `#N-gateway-cli-subcommand`
PR title: `#N: Add gateway lifecycle management and setup wizard`
PR body includes: updated files list, test coverage summary, config format note.
Also update `docs/PRD.md` tracker row (if there is one) and `README.md` usage section with new commands.

## Success criteria
1. `oben gateway start` — builds binary, launches gateway process, writes PID file ✅
2. `oben gateway status` — reports running/not-running/stale correctly ✅
3. `oben gateway stop` — sends SIGTERM, cleans PID file ✅
4. `oben gateway setup` — interactive wizard completes, config persisted ✅
5. Config roundtrip: YAML load → modify gateway → save → load = identical ✅
6. `cargo test -p oben-cli --lib` — all tests green ✅
7. `cargo check -p oben-gateway` — no changes to gateway binary ✅
8. No workspace-wide test invocation (`cargo test --workspace` forbidden) ✅
