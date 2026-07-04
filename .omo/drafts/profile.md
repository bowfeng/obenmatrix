---
slug: profile
status: awaiting-approval
intent: unclear → clear defaults
pending-action: write .omo/plans/profile.md
approach: Fix Env typos, export Env, add --profile to Cli, wire through load/save/dispatch/tests
---

# Draft: profile

## Components (topology ledger)
| id | outcome | status | evidence path |
|---|---|---|---|
| Env struct | Fixabenmatrix→obenmatrix typos, export from lib.rs | active | env.rs:38-50, lib.rs |
| AppConfig::load/save | Accept profile param, use Env for path resolution | active | config.rs:1006-1085 |
| CLI Cli | Add --profile top-level flag | active | cli.rs:7-16 |
| dispatch.rs | Thread profile through run_chat/run_one_shot/run_setup/run_config | active | dispatch.rs:100-250, 248-250, 1096-1099 |
| Tests | Fix allabenmatrix→obenmatrix assertions | active | env.rs:88-154, config.rs tests, live tests |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|---|---|---|---|
| `--profile` location | Top-level `Cli` field (global flag) | All commands share the same profile; cleaner API than repeating on every subcommand | reversible — per-command is easier to scope |
| Profile loading path | `AppConfig::load(profile: Option<&str>)` — accepts optional profile, internally creates `Env` | No need to thread `Env` everywhere; callers just pass the profile string from CLI | reversible — inject `Env` is more testable but adds surface area |
| `Env` usage | Default profile (`None`) = `~/.config/obenmatrix/` + `~/.obenmatrix/`; named profile = subdirectory | Matches user's explicit requirement | — |
| Scope | Config + data + dispatch wiring + tests only. Other hardcoded paths (memory, skills, tools) LEFT AS-IS | User asked specifically about config.yaml and data directory paths | explicit scope boundary |
| Backwards compatibility | `--profile` defaults to `None` which keeps existing global paths | No breaking change for existing users | — |
| profiles.yaml manifest | NOT in scope | User didn't mention it; skip for v1 | out-of-scope |

## Findings (cited - path:lines)

1. **`Env` struct exists but has typos** — env.rs:43-50: uses `.abenmatrix` instead of `.obenmatrix` for named profiles. Tests (env.rs:93-154) assert against the wrong paths.
2. **`Env` NOT exported** — env.rs exists but lib.rs:5-11 only exports `config`, `defaults`, `wizard`. No `pub mod env;`.
3. **`AppConfig::load()`/`save()` are hardcoded** — config.rs:1030-1085: uses `config_path_legacy()` which hardcodes `~/.config/obenmatrix/config.yaml`. No profile support.
4. **No `--profile` CLI flag** — cli.rs:7-16: `Cli` has only `verbose` and `command`. No profile field.
5. **dispatch.rs has hardcoded paths** — dispatch.rs:1096-1099: gateway PID path uses `dirs::config_dir().join("obenmatrix")`. Also dispatch.rs:103, 187, 243: calls `AppConfig::load()` with no args.
6. **Many other crates have hardcoded paths** — sessions: manager.rs:1847-1849, skill_curation.rs:132-134; tools: memory.rs:158, todo.rs:58, voice.rs:192, tts.rs:16, skill.rs:16; utils: logging.rs:13, debug.rs:149; curator: curator.rs:26, usage.rs:11-17; cron: jobs.rs:269; gateway: main.rs:18, 76, 297. These are LEFT AS-IS for v1.
7. **AgentBuilder has `with_config()`** — dispatch.rs:154-159: takes `config` by value. Config carries model/tools info, not paths. No change needed here.
8. **DBSessionManager has `new_with_path()`** — sessions:manager.rs:1854-1857: already supports passing custom path. Good — makes future profile wiring easier.

## Decisions (with rationale)
1. **Fix `abenmatrix` → `obenmatrix` in Env** — clear bug. Tests must be fixed too.
2. **Add profile to `Cli` struct (top-level)** — simplest integration point. `Cli::parse()` gives us the profile before any dispatch.
3. **`AppConfig::load(profile: Option<&str>)` pattern** — callers pass the string; internally `Env` is created. Keeps the change localized.
4. **Scope: config + dispatch only** — user explicitly asked about config.yaml and data directories. Other subsystems (memory, tools) use `.obenmatrix/` directly but those changes can be a follow-up.

## Scope IN
- Fix `abenmatrix` → `obenmatrix` typos in `Env` struct, comments, and ALL test assertions
- Export `Env` from `oben-config/src/lib.rs`
- Add `--profile` top-level CLI flag to `Cli` struct
- Modify `AppConfig::load(profile: Option<&str>)` and `save(profile: Option<&str>)` to use `Env`
- Wire `--profile` from `Cli` through `dispatch.rs`: run_chat, run_one_shot, run_setup, run_config, goals_start, goal_list, goal_status, goal_pause, goal_resume, goal_clear
- Fix gateway PID path to respect profile
- Update TUI session manager initialization to respect profile
- Fix all tests (env.rs tests, any config.rs tests that assert paths)
- Update README.md to document profile usage
- Update wizard.rs success message to show correct profile path

## Scope OUT (Must NOT have)
- Do NOT touch memory/sessions/tools/curator/cron hardcoded paths — those are follow-up
- Do NOT implement `profiles.yaml` manifest or auto-discovery
- Do NOT add interactive profile management commands (create/delete/switch)
- Do NOT change `AppConfig::config_dir()` or `config_path()` static methods (they are legacy, never called in practice — `load()` uses `config_path_legacy()`)
- Do NOT change `AgentBuilder` API or `Agent` internals

## Open questions
None. All discovered facts resolved by research.

## Approval gate
status: awaiting-approval
pending-action: write .omo/plans/profile.md after approval
