# rename - Work Plan

## TL;DR (For humans)

**What you'll get:** Every reference to "oben-alien"/"obenalien"/"oben_alien" throughout the codebase will be systematically renamed to "oben-matrix"/"obenmatrix" — covering the binary name, config paths, crate package name, doc links, service files, and GitHub URLs.

**Why this approach:** A single coordinated rename across ~35 affected locations with parallel edit waves, followed by full compile + test verification. Source code path constants are bulk-replaced; documentation GitHub URLs are updated for product links only (historical parity table references left unchanged).

**What it will NOT do:**
- Rename the git remote/origin (you do that via GitHub after creation of the new repo)
- Rename the local repository directory on disk
- Update historical parity table URLs (they link to closed PRs/issues — changing them would break GitHub links)

**Effort:** Medium
**Risk:** Low — purely mechanical string replacement; no logic changes. Compile + test gate confirms nothing breaks.
**Decisions I made for you:**
1. Rust package name: `obenalien` → `obenmatrix` (Rust crate naming convention — no underscores between words, consistent with `oben-cron`, `oben-tools`, etc.)
2. Config paths: `~/.obenalien` → `~/.obenmatrix`, `~/.config/obenalien` → `~/.config/obenmatrix`
3. Context file: `.obenalien.md` → `.obenmatrix.md` (follows the same brand pattern)
4. macOS plist label: `org.obenalien.cron` → `org.obenmatrix.cron`

Your next move: approve this plan. Execution begins on your command.

---

> TL;DR (machine): Medium — full-codebase rename "oben-alien" → "oben-matrix" across Cargo.toml, 25+ source files, docs, CI, macOS plist, service files. 6 sequential waves + compile/test verification.

## Scope
### Must have
- **Cargo.toml**: package name `obenalien` → `obenmatrix`; repository URL `bowfeng/oben-alien` → `bowfeng/obenmatrix`
- **Source code path constants**: `~/.obenalien` → `~/.obenmatrix`; `~/.config/obenalien` → `~/.config/obenmatrix`; `~/.config/oben` → `~/.config/obenmatrix` (in `oben-config/src/config.rs:1009`)
- **Binary name**: `obenalien` → `obenmatrix` across `oben-cron/src/lib.rs`, `oben-cli/src/dispatch.rs` test strings, README.md, PRD.md, scenario tests
- **Context file discovery**: `.obenalien.md` → `.obenmatrix.md` (in `oben-config/src/config.rs`, `oben-agent/src/system_prompt.rs`)
- **macOS plist**: `org.obenalien.cron` → `org.obenmatrix.cron`
- **Linux service file**: description and documentation URL (service)
- **README.md**: clone path, binary commands
- **AGENTS.md**: `~/.obenalien/config.yaml` paths
- **Live test file comments**: `~/.obenalien/config.yaml`
- **Doc comment GitHub URLs**: `github.com/bowfeng/oben-alien` → `github.com/bowfeng/obenmatrix`; `github.com/ellie/oben-alien` → `github.com/ellie/obenmatrix`
- **PRD.md**: directory structure diagram references, `obenalien agent` command

### Must NOT have (guardrails)
- **DO NOT** change historical parity table URLs in `docs/PRD-*-parity.md` (e.g., `github.com/bowfeng/oben-alien/pull/100`) — these reference closed PRs/issues; changing them would break GitHub links
- **DO NOT** rename the git remote/origin
- **DO NOT** rename the local directory
- **DO NOT** change the project display name "ObenAgent" in the README
- **DO NOT** touch `.codegraph/`, `.omo/`, `target/`, `Cargo.lock`

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: **tests-after** — replace all strings, then run targeted test suites
- Evidence: grep scan pre/post, compile output, test output

## Execution strategy
> Sequential waves (dependencies between waves due to scope overlap).

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

### Wave 1: Config paths (15+ source files)
- [ ] 1. Rename `~/.obenalien` → `~/.obenmatrix` and `~/.config/obenalien` → `~/.config/obenmatrix` across all source files
  What to do: Bulk replace path constants. `.obenalien` → `.obenmatrix`, `config/obenalien` → `config/obenmatrix`, `config/oben` → `config/obenmatrix`.
  Parallelization: Wave 1 (first — sets up path consistency)
  References: `oben-tools/src/voice.rs:192`, `oben-tools/src/todo.rs:58`, `oben-tools/src/skill.rs:16,494`, `oben-tools/src/memory.rs:158`, `oben-tools/src/tts.rs:16`, `oben-gateway/src/main.rs:18,76,297`, `oben-skills/src/loader.rs:345,364,492,494`, `oben-curator/src/curator.rs:26`, `oben-curator/src/usage.rs:8-17`, `oben-utils/src/debug.rs:149`, `oben-utils/src/logging.rs:9,13`, `oben-config/src/config.rs:8,1010,1022,1025,1030`, `oben-config/src/wizard.rs:117`, `oben-cron/src/jobs.rs:3,269`, `oben-sessions/src/skill_curation.rs:102,133,134`, `oben-sessions/src/manager.rs:1848,1849`, `oben-agent/src/system_prompt.rs:19`, `AGENTS.md:207,226`
  Acceptance criteria: `grep -rn '\.obenalien' src/` returns 0 matches after edit
  QA: `grep -rn '\.obenalien' --include='*.rs'` before and after comparison
  Commit: Y | chore(rename): rename .obenalien config paths across all crates

### Wave 2: Context file naming (blocked by W1)
- [ ] 2. Rename `.obenalien.md` → `.obenmatrix.md` in discovery logic
  What to do: Replace `.obenalien.md` with `.obenmatrix.md` in default file lists and doc comments.
  Parallelization: Wave 2 | Blocked by: Wave 1
  References: `oben-config/src/config.rs:572,793`, `oben-agent/src/system_prompt.rs:19,34,35,116`
  Acceptance criteria: `grep -rn '\.obenalien\.md' src/` returns 0 matches
  QA: grep pre/post, verify both config.rs lines and system_prompt.rs updated
  Commit: Y | chore(rename): rename .obenalien.md context file

### Wave 3: Binary name strings (can start parallel with W2)
- [ ] 3. Rename binary `obenalien` → `obenmatrix` across all references
  What to do: Replace binary name in `oben-cron/lib.rs` (cron_exec_binary), test path strings in `oben-cli/dispatch.rs`, README.md examples, PRD.md, scenario test comments.
  Parallelization: Wave 3 | Blocked by: — | Blocks: none
  References: `oben-cron/src/lib.rs:13,24,25,26,27,37,44`, `oben-cli/src/dispatch.rs:1098,1592,1613,1617-1619,1677,1694,1713,1718`, `README.md:49,62,65,68,71,74,77,80,81,84,94,154,157,160,163,173`, `docs/PRD.md:89,244`, `oben-scenario-test/tests/live_session.rs:52,174`, `oben-scenario-test/tests/live_tools.rs:3`, `oben-scenario-test/tests/live_transport.rs:3`
  Acceptance criteria: `grep -rn 'obenalien' --include='*.rs' src/` returns 0 matches (excluding parity URLs)
  QA: grep before/after, compile check
  Commit: Y | chore(rename): rename binary obenalien → obenmatrix

### Wave 4: Package metadata and Cargo.toml
- [ ] 4. Update Cargo.toml package name and repository URL
  What to do: `name = "obenalien"` → `name = "obenmatrix"`; `repository = "https://github.com/bowfeng/oben-alien"` → `repository = "https://github.com/bowfeng/obenmatrix"`
  Parallelization: Wave 4 | Blocked by: —
  References: `Cargo.toml:29,69`
  Acceptance criteria: `cargo metadata` shows package as `obenmatrix`
  QA: `cargo metadata --format-version 1 | jq '.packages[] | select(.name=="obenmatrix")'`
  Commit: Y | chore(rename): update Cargo.toml package and repo URL

### Wave 5: Doc comment GitHub URLs
- [ ] 5. Update GitHub URLs in Rust source and service files
  What to do: `github.com/bowfeng/oben-alien` → `github.com/bowfeng/obenmatrix`; `github.com/ellie/oben-alien` → `github.com/ellie/obenmatrix`
  Parallelization: Wave 5 | Blocked by: —
  References: `oben-utils/src/credential_pool.rs:3`, `oben-utils/src/checkpoint.rs:8`, `oben-cron/services/oben-cron.service:3`
  Acceptance criteria: `grep -rn 'github.com.*oben-alien' --include='*.rs' --include='*.service'` returns 0
  QA: grep before/after
  Commit: Y | chore(rename): update GitHub URLs in doc comments

### Wave 6: macOS plist
- [ ] 6. Update macOS launch agent plist label
  What to do: `org.obenalien.cron` → `org.obenmatrix.cron`
  Parallelization: Wave 6 | Blocked by: —
  References: `oben-cron/services/org.obenalien.cron.plist:6`
  Acceptance criteria: `grep 'obenalien' oben-cron/services/org.obenalien.cron.plist` returns 0
  QA: grep before/after
  Commit: Y | chore(rename): update macOS plist label

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE.
- [ ] F1. Plan compliance audit
  Run: `grep -rn 'obenalien\|oben-alien\|oben_alien' --include='*.rs' --include='*.toml' --include='*.md' --include='*.service' --include='*.plist' /Users/ellie/workspace/oben-alien/ --exclude-dir=.codegraph --exclude-dir=.omo --exclude-dir=target`
  Verify: Zero matches outside `.omo/plans/rename.md` and historical parity table URLs.
- [ ] F2. Code quality review
  Run: `cargo check --workspace`
  Verify: Clean compile, no broken imports
- [ ] F3. Scope fidelity
  Verify: No parity table URLs changed; no unrelated files touched.
- [ ] F4. Integration smoke test
  Run: `cargo test -p oben-config -p oben-cli -p oben-cron --lib 2>&1`
  Verify: All tests pass

## Commit strategy
Single squashed commit: `chore(rename): rename oben-alien → oben-matrix across entire codebase`

## Success criteria
- `grep 'obenalien\|oben-alien\|oben_alien'` returns zero matches outside parity docs and plan artifacts
- `cargo check --workspace` compiles cleanly
- `cargo test -p oben-config -p oben-cli -p oben-cron --lib` passes
- Package name resolves to `obenmatrix`
- No historical parity table URLs were broken
