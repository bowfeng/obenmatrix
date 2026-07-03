---
slug: rename
status: awaiting-approval
intent: clear
pending-action: await user approval to execute rename plan
approach: systematic find-and-replace rename "oben-alien" → "oben-matrix" across ~35 locations in 6 waves
---

# Draft: rename

## Components (topology ledger)
- Root Cargo.toml | package+repo | status: active | Cargo.toml:29,69
- 25+ source files | config paths | status: active | *.rs files across all crates
- 3 service/plist files | service labels | status: active | .service, .plist
- README.md + AGENTS.md + PRD.md | documentation | status: active | docs + root
- Doc comment GitHub URLs | source links | status: active | *.rs source files

## Open assumptions (announced defaults)
1. Package name: `oben_alten` → `obenmatrix` (not `oben_matrix`; Rust convention + consistency with `oben-cron`, `oben-tools`)
2. Config paths: `~/.obenalien` → `~/.obenmatrix`, `~/.config/obenalien` → `~/.config/obenmatrix`
3. Context file: `.obenalien.md` → `.obenmatrix.md`
4. Historical parity table URLs preserved (not changed — they reference closed PRs/issues)
5. Git remote/origin NOT changed by automation (user action)

## Findings (cited - path:lines)

**Binary name `obenalien` references (36 lines, 8 files):**
- `Cargo.toml:69` — package name
- `oben-cron/src/lib.rs:13,24,25,26,27,37,44` — cron_exec_binary() function
- `oben-cli/src/dispatch.rs:1098,1592,1613,1617-1619,1677,1694,1713,1718` — test assertions + gateway pid
- `README.md:49,62,65,68,71,74,77,80,81,84,94,154,157,160,163,173` — usage docs
- `docs/PRD.md:89,244` — architecture diagram + command ref
- `oben-scenario-test/tests/live_session.rs:52,174` — test config paths
- `oben-scenario-test/tests/live_tools.rs:3` — doc comment
- `oben-scenario-test/tests/live_transport.rs:3` — doc comment
- `oben-utils/src/debug.rs:149` — paste dir path

**Config path `.obenalien` references (22 files):**
- `oben-tools/src/voice.rs:192`, `todo.rs:58`, `skill.rs:16,494`, `memory.rs:158`, `tts.rs:16`
- `oben-gateway/src/main.rs:18,76,297`
- `oben-skills/src/loader.rs:345,364,492,494`
- `oben-curator/src/curator.rs:26`, `usage.rs:8-17`
- `oben-utils/src/debug.rs:149`, `logging.rs:9,13`
- `oben-config/src/config.rs:8,1010,1022,1025,1030`, `wizard.rs:117`
- `oben-cron/src/jobs.rs:3,269`
- `oben-sessions/src/skill_curation.rs:102,133,134`, `manager.rs:1848,1849`
- `oben-agent/src/system_prompt.rs:19`
- `AGENTS.md:207,226`

**Context file `.obenalien.md` references:**
- `oben-config/src/config.rs:572,793` — default file lists
- `oben-agent/src/system_prompt.rs:34,35,116` — context file discovery

**GitHub URLs (3 files):**
- `oben-utils/src/credential_pool.rs:3` — `github.com/ellie/oben-alien`
- `oben-utils/src/checkpoint.rs:8` — `github.com/bowfeng/oben-alien`
- `oben-cron/services/oben-cron.service:3` — `github.com/bowfeng/oben-alien`

**Package metadata:**
- `Cargo.toml:29` — `repository = "https://github.com/bowfeng/oben-alien"`
- `oben-cron/services/org.obenmatrix.cron.plist:6` — label `org.obenalien.cron`

## Decisions (with rationale)
1. **Keep parity table URLs unchanged**: They reference closed PRs/issues on the old repo name. GitHub preserves redirects for renamed repos, but modifying them in historical documentation is unnecessary noise.
2. **Use `obenmatrix` not `oben_matrix`**: Rust crate naming convention favors no underscores between words; matches existing `oben-cron`, `oben-tools` naming.

## Scope IN
- All 6 waves of find-and-replace edits
- Final verification (grep compliance, compile, tests)

## Scope OUT (Must NOT have)
- Git remote/origin rename
- Local directory rename
- Parity table URL updates
- Cargo.lock modification
- .codegraph / .omo directory changes

## Approval gate
status: awaiting-approval
pending-action: execute the 6-wave rename plan via $start-work
