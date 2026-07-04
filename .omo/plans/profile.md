# profile - Work Plan

## TL;DR (For humans)

**What you'll get:** A `--profile <name>` CLI flag that partitions config and data directories per profile. Default profile (`obenmatrix --profile xin` or just `obenmatrix` without a flag) keeps existing global paths. Named profiles get `~/.config/obenmatrix/xin/config.yaml` + `~/.obenmatrix/xin/`.

**Why this approach:** The `Env` struct already exists with the right logic but has bugs and isn't exported. We fix it, wire it through `AppConfig::load()`/`save()`, and pass the profile from CLI through dispatch. Minimal surface area, no `Env` threading needed.

**What it will NOT do:** Touch memory/skills/tools paths in other crates, implement `profiles.yaml`, add interactive profile management commands, or change `AgentBuilder` API.

**Effort:** Medium
**Risk:** Low — backwards compatible; default profile (`None`) preserves existing behavior
**Decisions I made for you:** `--profile` is a top-level `Cli` flag (all commands share one profile); `AppConfig::load(profile: Option<&str>)` accepts the string and internally creates `Env`; `abenmatrix` → `obenmatrix` typo fix in `Env`. You can veto any.

Your next move: approve (yes/proceed) or request changes.

---

> TL;DR (machine): Medium effort, Low risk — add `--profile` CLI flag, fix `abenmatrix` typos, wire `Env` through config load/save/dispatch/tests, ~5 files changed.

## Scope
### Must have
- Fix `abenmatrix` → `obenmatrix` typo in `Env` struct, comments, and ALL test assertions
- Export `Env` from `oben-config/src/lib.rs`
- Add `--profile` top-level CLI flag to `Cli` struct in `oben-cli/src/cli.rs`
- Modify `AppConfig::load(profile: Option<&str>)` to use `Env` for path resolution
- Modify `AppConfig::save(profile: Option<&str>)` to use `Env` for path resolution
- Wire `--profile` from parsed `Cli` through dispatch handlers: `run_chat`, `run_one_shot`, `run_setup`, `run_config`, `run_compact_session`, `run_models`, `goal_start`, `gateway_setup`
- Update `get_gateway_pid_path()` to respect profile
- Update `AppConfig::config_path()` usage in `run_config` (Edit action)
- Fix ALL test assertions referencing `abenmatrix` in env.rs and config.rs
- Update wizard.rs to show correct profile-specific path in success message
- Update README.md to document profile usage

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Do NOT touch hardcoded paths in `oben-sessions`, `oben-tools`, `oben-skills`, `oben-cron`, `oben-curator`, `oben-gateway`, `oben-utils` — these are follow-up
- Do NOT implement `profiles.yaml` manifest or profile listing/switching commands
- Do NOT change `AppConfig::config_dir()` or `config_path()` static methods
- Do NOT change `AgentBuilder` API or `Agent` internals
- Do NOT add a `run_tui` profile change (TUI is a follow-up)
- Do NOT modify `.omo/` files beyond reading the draft and appending to the plan

## Verification strategy
> Zero human intervention — all verification is agent-executed.
- Test decision: TDD for Env path resolution; tests-after for dispatch wiring
- Evidence: `cargo test --package oben-config --lib` (unit), `cargo test --package oben-cli --lib` (unit)
- Manual: `oben --profile xin chat --help` shows `--profile`, `oben setup` writes to profile path

## Execution strategy
### Parallel execution waves
> 3 waves: (1) foundational fix + export, (2) dispatch wiring + tests, (3) docs + final verification

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T1: Fix Env typos + export | — | T2 | T3 (tests) |
| T2: AppConfig load/save signature + Env usage | T1 | T4-T9 | — |
| T3: Env tests | T1 | T6 | T2 |
| T4: Add --profile to Cli | — | T5 | — |
| T5: Run/Chat/Setup dispatch wiring | T4, T2 | — | T6-T9 |
| T6: Sessions/Models/Goals dispatch wiring | T2, T4 | — | T5 |
| T7: Gateway PID path + run_config wiring | T2 | — | T5-T6 |
| T8: Wizard + README + test fixes | T1, T2 | — | T5-T7 |
| T9: Final compilation check | T5-T8 | — | — |
| F1-F4: Final verification | T9 | — | all |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [ ] 1. Fix `abenmatrix` → `obenmatrix` typos in `Env` struct and export from lib.rs
  What to do / Must NOT do: Replace all `.abenmatrix` with `.obenmatrix` in env.rs lines 38, 45, 48, 49, 96, 97, 107, 111, 141, 150. Add `pub mod env;` and `pub use env::*;` to lib.rs.
  Parallelization: Wave 1 | Blocked by: none | Blocks: T2, T3
  References (executor has NO interview context - be exhaustive): env.rs:38, 45, 48, 49, 96, 97, 107, 111, 141, 150; lib.rs:5-11
  Acceptance criteria (agent-executable): `cargo test --package oben-config --lib` passes after this todo (before T3 tests exist)
  QA scenarios (name the exact tool + invocation): happy: run `cargo test --package oben-config --lib` after T1 — no test should exist yet for env module. Failure: grep for "abenmatrix" returns 0 matches — any remaining match means typo was missed. Evidence .omo/evidence/task-1-profile.md
  Commit: Y | fix(config): fix obenmatrix typos in Env and export from lib
- [ ] 2. Modify `AppConfig::load()` and `save()` to accept optional profile and use `Env`
  What to do / Must NOT do: Change `pub fn load()` → `pub fn load(profile: Option<&str>) -> anyhow::Result<Self>`. Inside, create `Env::new(profile.map(str::to_string))`, use `env.config_path()` for reading, `env.config_dir()` for writing. Same for `save()` → `pub fn save(&self, profile: Option<&str>)`. Keep all env-var fallback logic intact. Update `config_path()` method used by `run_config` Edit action.
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: T5-T7
  References: config.rs:1006-1085, env.rs:33-85
  Acceptance criteria (agent-executable): `cargo check --package oben-config` compiles with the new signatures (dispatch.rs will fail until wired — that's expected)
  QA scenarios: happy: `cargo check --package oben-config` succeeds. Failure: compilation errors beyond expected dispatch.rs missing args means signature change is wrong. Evidence .omo/evidence/task-2-profile.md
  Commit: Y | feat(config): wire Env into AppConfig load/save for profile support
- [ ] 3. Add tests for `Env` path resolution in profile mode
  What to do / Must NOT do: Add test assertions to env.rs test module verifying: (a) `Env::new(None)` produces global `.config/obenmatrix/` and `.obenmatrix/`, (b) `Env::new(Some("xin".to_string()))` produces `.config/obenmatrix/xin/` and `.obenmatrix/xin/`, (c) `config_path()` matches expected paths, (d) `is_default()` works correctly.
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: (independent, can run anytime after T1)
  References: env.rs:88-154 (test module)
  Acceptance criteria (agent-executable): `cargo test --package oben-config env::tests` — all tests pass
  QA scenarios: happy: `cargo test --package oben-config env::tests` passes. Failure: any assertion fails means path mismatch. Evidence .omo/evidence/task-3-profile.md
  Commit: Y | test(config): add Env profile path resolution tests
- [ ] 4. Add `--profile` flag to `Cli` struct in cli.rs
  What to do / Must NOT do: Add `#[arg(long)] pub profile: Option<String>` field to the `Cli` struct (line 7-16). This replaces the `verbose` field scope — add it as `/// Profile name — uses subdirectories in ~/.config/obenmatrix/<profile>/ and ~/.obenmatrix/<profile>/` and `#[arg(long)] pub profile: Option<String>`. Keep `verbose` as well.
  Parallelization: Wave 2 | Blocked by: none | Blocks: T5-T7
  References: cli.rs:7-16
  Acceptance criteria (agent-executable): `oben --help` shows `--profile <PROFILE>` option
  QA scenarios: happy: `oben --profile xin --help` succeeds and shows subcommands. Failure: `--profile` not recognized in help output. Evidence .omo/evidence/task-4-profile.md
  Commit: Y | feat(cli): add --profile top-level flag to Cli
- [ ] 5. Wire `--profile` through `run_chat`, `run_one_shot`, `run_setup`, `run_config`
  What to do / Must NOT do: In `dispatch.rs`, add `profile: Option<&str>` parameter to each of these 4 functions. In the main `run_cli()` match arm at lines 38-95, pass `cli.profile.as_deref()` to each. Every call changes from `AppConfig::load()` to `AppConfig::load(cli.profile.as_deref())` and from `config.save()` to `config.save(cli.profile.as_deref())`. For `run_config`, also change the Edit action's `AppConfig::config_path()` to use `Env`-derived path.
  Parallelization: Wave 2 | Blocked by: T2, T4 | Blocks: T9
  References: cli.rs:7-16, dispatch.rs:27-96 (run_cli entry), dispatch.rs:100-182 (run_chat), dispatch.rs:186-238 (run_one_shot), dispatch.rs:242-260 (run_setup, run_config)
  Acceptance criteria (agent-executable): `cargo check --package oben-cli` compiles (no dispatch.rs errors)
  QA scenarios: happy: `cargo check --package oben-cli` succeeds. Failure: unused import or missing argument errors. Evidence .omo/evidence/task-5-profile.md
  Commit: Y | feat(cli): wire profile through chat/run/setup/config handlers
- [ ] 6. Wire `--profile` through `run_compact_session`, `run_models`, `goal_start`
  What to do / Must NOT do: Same pattern as T5. Add `profile: Option<&str>` parameter to `run_compact_session` (line 309), `run_models` (line 464), `goal_start` (line 552). Update match arms at lines 50-51, 57, 77 to pass `cli.profile.as_deref()`. Note: `goal_store()` returns default store — scope out of profile for goals/cron as these need separate `store` support.
  Parallelization: Wave 2 | Blocked by: T2, T4 | Blocks: T9
  References: dispatch.rs:309-395 (run_compact_session), 464-523 (run_models), 552-743 (goal_start)
  Acceptance criteria (agent-executable): `cargo check --package oben-cli` compiles
  QA scenarios: happy: `cargo check --package oben-cli` succeeds. Failure: unexpected compilation errors. Evidence .omo/evidence/task-6-profile.md
  Commit: Y | feat(cli): wire profile through compact/models/goals handlers
- [ ] 7. Wire `--profile` through `gateway_setup`, update `get_gateway_pid_path`
  What to do / Must NOT do: Update `gateway_setup` (line 1244) to accept `profile: Option<&str>`, pass through to `AppConfig::load()` and `config.save()`. Update `get_gateway_pid_path()` (line 1096) to accept optional profile and use `Env` for PID path. Change from hardcoded `dirs::config_dir().join("obenmatrix").join("gateway.pid")` to `env.config_dir().join("gateway.pid")`. Also update `run_config` Edit action to show the profile-aware config path.
  Parallelization: Wave 2 | Blocked by: T2, T4 | Blocks: T9
  References: dispatch.rs:1096-1242 (get_gateway_pid_path, gateway_start/stop/status, gateway_setup)
  Acceptance criteria (agent-executable): `cargo check --package oben-cli` compiles
  QA scenarios: happy: `cargo check --package oben-cli` succeeds. Failure: wrong path format. Evidence .omo/evidence/task-7-profile.md
  Commit: Y | feat(cli): wire profile through gateway_setup and PID path
- [ ] 8. Update wizard.rs success message, fix README, add `run_config` Edit path resolution
  What to do / Must NOT do: Update `wizard.rs` line 117 to show correct profile-aware path (e.g., `~/.config/obenmatrix/config.yaml` for default, `~/.config/obenmatrix/<name>/config.yaml` for named). Update `README.md` configuration section to document profile usage. For `run_config` Edit action, change from `AppConfig::config_path()` (which returns wrong static path) to `AppConfig::load`-derived path via `Env`.
  Parallelization: Wave 3 | Blocked by: T2 | Blocks: T9
  References: wizard.rs:117, README.md:100-136, dispatch.rs:248-261
  Acceptance criteria (agent-executable): README documents `--profile`; wizard message shows correct path for any profile
  QA scenarios: happy: read README.md — contains profile documentation. wizard.rs message: read the output string. failure: missing profile docs or wrong path in message. Evidence .omo/evidence/task-8-profile-messaging.md
  Commit: Y | docs: update wizard message, README, and run_config Edit for profile support
- [ ] 9. Final compilation verification
  What to do / Must NOT do: Run `cargo check --workspace` to verify all packages compile. Then run `cargo test --package oben-config --lib` and `cargo test --package oben-cli --lib`. No product code changes in other packages are expected.
  Parallelization: Wave 3 | Blocked by: T5-T8 | Blocks: F1-F4
  References: All crates
  Acceptance criteria (agent-executable): `cargo check --workspace` returns 0 errors. `cargo test --package oben-config --lib && cargo test --package oben-cli --lib` all pass.
  QA scenarios (name the exact tool + invocation): happy: `cargo check --workspace` succeeds with no errors. `cargo test --package oben-config --lib` — all tests green. `cargo test --package oben-cli --lib` — all tests green. failure: any compilation error in `oben-config` or `oben-cli` means a dependency was broken. Evidence .omo/evidence/task-9-profile-compile.md
  Commit: Y | chore: final compilation verification

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit — agent verifies every requirement in "Scope / Must have" is implemented
- [ ] F2. Code quality review — agent checks Rust idioms: no any/unwrap/panic, enums over traits, proper error prop
- [ ] F3. Real manual QA — agent runs `oben --help` to verify `--profile` flag, checks `cargo check` output
- [ ] F4. Scope fidelity — agent greps for `abenmatrix` to confirm zero remaining references, confirms no product code touched outside scope

## Commit strategy

Single atomic commit: `feat(cli): add --profile support for config/data directory partitioning`

Changes: 5 files (env.rs, lib.rs, config.rs, cli.rs, dispatch.rs, wizard.rs, README.md, tests)

## Success criteria

1. `oben --profile xin` reads config from `~/.config/obenmatrix/xin/config.yaml`
2. `oben --profile xin` stores data in `~/.obenmatrix/xin/`
3. `oben` (no flag) still uses `~/.config/obenmatrix/config.yaml` and `~/.obenmatrix/` (backwards compatible)
4. `abenmatrix` typos removed — 0 references remaining
5. `cargo check --workspace` compiles
6. All tests pass
