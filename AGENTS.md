# AGENTS.md

## Agent skills

### Issue tracker

Issues are tracked in this repo's GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary used: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### PRD / Progress

Port Hermes Agent to Rust in this repo.
- Hermes Agent [https://github.com/nousresearch/hermes-agent], its local repo should be ~/workspace/hermes-agent.
- The project PRD and progress tracker lives at `docs/PRD.md`. Update the progress table as work advances.

**Parity documentation layout:**
- `docs/PRD.md` — overall architecture, milestones, high-level progress
- `docs/PRD-{area}-parity.md` — feature-area parity trackers (one file per area): session, transport, conversation, tools, skills, gateway, goals, cli, utils
- `docs/PRD-{area}.md` our current feature architecture.
- Each parity file lists gaps vs Hermes-Agent with severity, status (✅/❌), and GitHub issue references
- When working on a parity gap: create a GitHub issue, link the issue to the relevant parity file row, update its status **in the same PR that implements the feature**

### Coding Rules

## 1. Think Before Coding - MANDATORY OUTPUT FORMAT

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing ANYTHING, you MUST output the following block:

```
## 🤔 Thinking: [Feature/Bug #N]

**My assumptions:**
- [Assumption 1]
- [Assumption 2]

**What I'm unclear about:**
- [Question 1 - if none, say "None"]
- [Question 2]

**Simpler approach considered:**
- [Alternative] → [Why chosen/rejected]

**Tradeoffs:**
- [Tradeoff 1]
```

**If you have any questions or unclear points, STOP and ask before writing code.**

**Do not proceed until all unclarities are resolved.**

---

## 2. Simplicity First - WITH EXAMPLES

**Minimum code that solves the problem. Nothing speculative.**

### Allowed vs Not Allowed:

| Scenario | ❌ Too complex (reject) | ✅ Simple enough (accept) |
|----------|------------------------|--------------------------|
| Parse a header | `HeaderParser` trait + `HeaderParserImpl` struct + builder pattern | `fn parse_header(s: &str) -> Result<Header>` |
| Handle config | `ConfigBuilder` with 10 optional methods | `Config { api_key: String }` with `from_yaml` |
| Error handling | Custom error type with 5 variants for unreachable cases | `anyhow::Result<T>` or `thiserror` for actual recoverable errors |
| Session state | `Arc<RwLock<HashMap<...>>>` with generics | `Arc<Mutex<Session>>` or just pass mutable reference |
| Tool registry | `ToolRegistry` trait + `DynamicToolRegistry` impl + factory pattern | `HashMap<String, Box<dyn Tool>>` |

### The "Senior Engineer Test":
Ask yourself: "Would a senior engineer say 'this is overcomplicated'?"
- If YES → Delete 80% and try again
- If "maybe" → Delete 50% and see

**Rule of thumb:** If you wrote a trait for something used in exactly one place, you added abstraction too early.

---

## 3. Surgical Changes - WITH BOUNDARIES

**Touch only what you must. Clean up only your own mess.**

### What IS your mess:
- Variables/functions YOU introduced and are now unused
- Imports YOUR code made obsolete
- Comments YOU wrote that are now wrong
- Test fixtures YOUR change made unnecessary

### What IS NOT your mess (leave alone):
- Pre-existing unused imports (mention in PR, don't delete)
- Adjacent code with inconsistent formatting
- "Dead code" that existed before your change
- Someone else's old comment that's slightly inaccurate
- Helper functions that are unrelated but "could be cleaned up"

### When you see unrelated issues:
```
NOTE: While working on #42, I noticed that `parse_header` in session.rs:45
has a typo. This is not fixed in this PR but should be addressed in #50.
```
**Do NOT fix it. Only mention it.**

**The test:** Every changed line should trace directly to the user's request. If you can't explain why a line changed, don't change it.

---

## 4. Goal-Driven Execution - ALIGNED WITH BDD

**Define success criteria as tests. Loop until verified.**

### Transform tasks into verifiable goals:

| Task → | Verifiable goal |
|--------|-----------------|
| "Add session validation" | Write unit test for invalid session ID → make it pass |
| "Fix persistence bug" | Write integration test that reproduces → make it pass |
| "Refactor tool execution" | Run existing tests before → run after → ensure same results |
| "Add prefetch to memory" | Write unit test that fails without prefetch → pass |

### For multi-step tasks, state plan in this format:

```
## 📋 Execution Plan for #42

**Goal:** Add MemoryProvider with prefetch

**Verification criteria:**
1. Unit test `test_prefetch_before_recall` passes
2. Integration test `test_mock_prefetch_order` passes
3. Live test `test_live_memory_recall` passes
4. All existing tests pass

**Steps:**
1. [Write failing unit test] → verify: red
2. [Write failing integration test] → verify: red
3. [Implement core logic] → verify: unit + integration pass
4. [Write live test] → verify: green
5. [Update parity file] → verify: ✅ in file

**Stop conditions:**
- Test fails after 3 attempts → ask for help
- Unclear Hermes behavior → check reference before continuing
```

**Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.**

---

## 1-4 Quick Reference (Before ANY Task)

```
Before coding, mentally check:

□ 1. Assumptions stated? Unclear things asked?
□ 2. Simplest possible? (Would senior say overcomplicated?)
□ 3. Only my code touched? (Or am I "improving" adjacent stuff?)
□ 4. Can I state the goal as a failing test?

If all YES → Proceed to Section 5 (BDD workflow)
If any NO → Stop, rethink, rewrite plan
```

---

## 5. MANDATORY: BDD Test-First + GitHub Workflow (NO EXCEPTIONS)

### Complete Workflow for ANY Change

**Every code change follows this EXACT sequence:**

```
1. Create GitHub issue
2. Create branch (#<number>-<desc>)
3. Check Hermes reference
4. Write tests (unit → integration → live)
5. Run tests (must FAIL)
6. Implement
7. Run tests (must PASS)
8. Update parity file (in feature branch)
9. Create PR (#<number>: <desc>)
10. Merge PR
11. Close issue with summary (mention parity in PR)
12. Delete branch (local + remote)
```

---

### Test Architecture (THREE LAYERS, ONE LOCATION EACH)

| Tier | Location | What it tests | Network | Config |
|------|----------|---------------|---------|--------|
| **Unit** | `src/*.rs` with `#[cfg(test)]` | Pure logic, no I/O | No | None |
| **Integration** | `tests/*.rs` | API boundaries, mocks | No | None |
| **Live** | `oben-scenario-test/tests/*.rs` | Full stack, real LLM | Yes | `~/.obenagent/config.yaml` |

**CRITICAL RULES:**
- Unit tests live **INSIDE** the source file, not separate
- Integration tests use **mocks only** - never real network
- Live tests read model config from `~/.obenagent/config.yaml`

**Live test config format (`~/.obenagent/config.yaml`):**
```yaml
model:
  provider: "openai"  # or "anthropic", "local"
  name: "gpt-4"
  endpoint: "https://api.openai.com/v1"
  api_key: "${OPENAI_API_KEY}"  # env var resolution
```

---

### STEP 1: NEW FEATURE (Parity Gap)

#### 1.1 Create GitHub Issue

```bash
# Create issue with proper labels
gh issue create \
  --title "feat: add MemoryProvider trait" \
  --body "**Parity:** docs/PRD-memory-parity.md row 3
**Hermes reference:** ~/workspace/hermes-agent/memory/provider.py
**Severity:** priority-high
**Acceptance criteria:**
- [ ] Unit tests in src/memory/provider.rs
- [ ] Integration tests in tests/memory_tests.rs with mocks
- [ ] Live test in oben-scenario-test/tests/live_memory.rs
- [ ] All tests pass
- [ ] Parity file updated to ✅ in PR" \
  --label "priority-high,needs-triage"

# Note the issue number (e.g., #42)
```

#### 1.2 Create Branch

```bash
# Branch naming: #<issue-number>-<short-desc>
git checkout -b #42-memory-provider
```

#### 1.3 Check Hermes Reference

```bash
# Before writing ANY code, check how Hermes-Agent does it
ls ~/workspace/hermes-agent/
grep -r "MemoryProvider" ~/workspace/hermes-agent/src/

# If unclear, STOP and ask: "Hermes does X in this scenario. Should we match or diverge?"
```

#### 1.4 Write Tests in Order (ALL must fail)

```rust
// 1. UNIT TEST - in src/memory/provider.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    /// Tests that Prefetch loads data before recall.
    ///
    /// given: empty memory with prefetch enabled
    /// when: query is executed
    /// then: prefetch runs before recall
    /// reference: Hermes-Agent line 42
    #[test]
    fn test_prefetch_before_recall() {
        // pure logic test, no I/O
    }
}

// 2. INTEGRATION TEST - in tests/memory_tests.rs
/// Tests MemoryProvider with mock storage.
///
/// given: MockMemoryStorage with prefetch flag
/// when: query() is called
/// then: storage.prefetch() called before storage.recall()
/// note: uses MockMemoryStorage, no real network
#[test]
fn test_mock_prefetch_order() {
    let mock = MockMemoryStorage::new();
    // ... test with mocks
}

// 3. LIVE TEST - in oben-scenario-test/tests/live_memory.rs
/// Tests real memory with actual LLM.
///
/// given: config at ~/.obenagent/config.yaml
/// when: agent stores and recalls memory
/// then: recalled content matches stored
#[test]
fn test_live_memory_recall() {
    let config = load_config("~/.obenagent/config.yaml");
    // Real LLM call, full stack
}
```

#### 1.5 Run Tests (Must FAIL)

```bash
# Unit tests
cargo test --package oben-memory --lib

# Integration tests
cargo test --package oben-memory --test '*' -- --nocapture

# Live tests (only if real LLM needed)
cargo test --package oben-scenario-test --test live_memory -- --nocapture
```

**All must FAIL. If any pass, you're testing existing behavior - write a different test.**

#### 1.6 Show Confirmation

```
"📋 PARITY: MemoryProvider trait
- Issue: #42
- Branch: #42-memory-provider
- Hermes reference: ~/workspace/hermes-agent/memory/provider.py:42-58
- Unit test: src/memory/provider.rs:test_prefetch_before_recall (FAILING)
- Integration test: tests/memory_tests.rs:test_mock_prefetch_order (FAILING)
- Live test: oben-scenario-test/tests/live_memory.rs (FAILING)
- Parity file: docs/PRD-memory-parity.md row 3 (will update to ✅ in PR)

Ready to implement. Confirm?"
```

#### 1.7 Implement Until Tests Pass

```bash
# Write code
# Run tests iteratively
cargo test --package oben-memory --lib        # until pass
cargo test --package oben-memory --test '*'   # until pass
cargo test --package oben-scenario-test       # until pass (if live needed)
```

#### 1.8 Update Parity File (IN FEATURE BRANCH, before PR)

```bash
# While still in feature branch #42-memory-provider

# Update the parity file
sed -i 's/| MemoryProvider trait | ❌ |/| MemoryProvider trait | ✅ (#42) |/' docs/PRD-memory-parity.md
sed -i 's/| Notes |/| Notes | [#42](https:\/\/github.com\/...\/42) |/' docs/PRD-memory-parity.md

# Commit parity update alongside the code
git add docs/PRD-memory-parity.md
git commit -m "docs: update memory parity for #42 - MemoryProvider trait implemented"

# Now the branch has both code change AND parity update
```

#### 1.9 Create Pull Request

```bash
# Push branch (includes parity update)
git push origin #42-memory-provider

# Create PR with template
gh pr create \
  --title "#42: Add MemoryProvider trait with prefetch/recall" \
  --body "**Closes:** #42
**Parity:** docs/PRD-memory-parity.md row 3 (updated to ✅ in this PR)
**Tests added:**
- Unit: src/memory/provider.rs (prefetch_before_recall)
- Integration: tests/memory_tests.rs (mock_prefetch_order)
- Live: oben-scenario-test/tests/live_memory.rs

**Checklist:**
- [x] Tests written before implementation
- [x] All tests pass
- [x] Hermes behavior matched
- [x] Parity file updated (✅) in this PR" \
  --label "parity,ready-for-review"
```

#### 1.10 Merge PR (includes parity update)

```bash
# Merge PR - this brings both code AND parity update to main
gh pr merge #42 --merge --delete-branch
```

#### 1.11 Close Issue with Summary

```bash
gh issue close 42 --comment "✅ MemoryProvider trait implemented with prefetch/recall.
- Added unit tests in src/memory/provider.rs
- Added integration tests with mocks
- Added live tests (reads from ~/.obenagent/config.yaml)
- **Parity file updated:** docs/PRD-memory-parity.md row 3 now ✅ (included in PR #42)
- Behavior matches Hermes-Agent reference

Post-merge:
- [x] Parity update merged as part of PR
- [x] Branch deleted"
```

#### 1.12 Delete Branch (if not auto-deleted)

```bash
# Delete locally
git branch -d #42-memory-provider

# Delete remotely (if not auto-deleted)
git push origin --delete #42-memory-provider
```

---

### STEP 2: BUG FIX

#### 2.1 Create GitHub Issue with bug template

```bash
gh issue create \
  --title "fix: session loses data on tool error" \
  --body "**Bug:** Session state not saved when tool returns error
**Severity:** priority-critical
**Parity file row:** docs/PRD-session-parity.md row 5
**Hermes reference:** ~/workspace/hermes-agent/session/persistence.py:42

**Root cause analysis (required):**
- What happened: [description]
- Why existing tests missed it:
  - Unit tests: [gap in pure logic]
  - Integration tests: [missing mock scenario]
  - Live tests: [didn't exercise error path]

**Acceptance criteria:**
- [ ] Create docs/session-bug.md
- [ ] Write reproducing test in correct tier
- [ ] Test fails (red)
- [ ] Implement fix
- [ ] Test passes (green)
- [ ] Parity file updated (in PR)
- [ ] PR merged
- [ ] Issue closed" \
  --label "priority-critical,needs-triage"
```

#### 2.2 Create Branch

```bash
git checkout -b #43-fix-session-data-loss
```

#### 2.3 Create Bug Tracking File

```bash
# Create doc/session-bug.md
cat > docs/session-bug.md << 'EOF'
## Bug #43: Session data lost on tool error

**Date:** 2026-05-24
**Issue:** #43
**Parity file row:** docs/PRD-session-parity.md row 5

**Root cause:**
- What happened: Session.save() not called before returning Err from tool execution
- Why tests missed it: 
  - Unit tests: All pure logic tests pass (save logic not tested)
  - Integration: All mocks simulate success only (never test error path)
  - Live: Only test happy path with real LLM (error path uncovered)
- Hermes-Agent behavior: Saves on every mutation, including error paths (line 42)

**Missing test scenario:**

Integration test (tests/session_tests.rs):
```rust
/// given: MockPersistence with save counter
/// when: session.process() returns Err
/// then: mock.save() called exactly once
```

Live test (oben-scenario-test/tests/live_session.rs):
```rust
/// given: config from ~/.obenagent/config.yaml
/// when: tool returns error
/// then: session can be restored with previous state
```

**Fix:**
- Call session.save() before returning Err in tool execution path

**Prevention:**
- All integration tests must include error paths
- Add save verification to all error scenarios
EOF

# Commit bug tracking file
git add docs/session-bug.md
git commit -m "docs: add bug tracking for #43 - session data loss"
```

#### 2.4 Write Reproducing Test (Must FAIL)

```rust
// In tests/session_tests.rs
/// Tests that session saves before returning error.
///
/// given: MockPersistence with save counter starting at 0
/// when: session.process() returns Err
/// then: mock.save() count == 1
/// note: This test FAILS before fix (shows bug)
#[test]
fn test_save_before_error() {
    let mock = MockPersistence::new();
    mock.expect_save().times(1).returning(Ok);
    
    let session = Session::with_persistence(mock);
    let result = session.process(bad_input);
    
    assert!(result.is_err());
    mock.verify();  // Fails before fix
}
```

#### 2.5 Run Test (Must FAIL)

```bash
cargo test --package oben-sessions --test session_tests test_save_before_error
# Output: FAILED - mock.save() called 0 times, expected 1
```

#### 2.6 Implement Fix

```rust
// In src/session.rs
impl Session {
    pub fn process(&mut self, input: Input) -> Result<Output, Error> {
        // ... existing code ...
        if let Err(e) = result {
            self.save()?;  // ADD THIS LINE
            return Err(e);
        }
        // ...
    }
}
```

#### 2.7 Run Test (Must PASS)

```bash
cargo test --package oben-sessions --test session_tests test_save_before_error
# Output: PASSED
```

#### 2.8 Add Missing Unit + Live Tests

```rust
// Unit test in src/session.rs
#[cfg(test)]
mod tests {
    #[test]
    fn test_save_called_on_error() {
        // Pure logic test without mocks
    }
}

// Live test in oben-scenario-test/tests/live_session.rs
#[test]
fn test_live_session_error_recovery() {
    let config = load_config("~/.obenagent/config.yaml");
    // Real LLM error scenario
}
```

#### 2.9 Update Parity File (IN FEATURE BRANCH, before PR)

```bash
# While still in feature branch #43-fix-session-data-loss

# Update the parity file - change ❌ to ✅
sed -i 's/| Persistence on all exit paths | ❌ |/| Persistence on all exit paths | ✅ (#43) |/' docs/PRD-session-parity.md

# Add note about bug fix
sed -i 's/| Notes |/| Notes | Fixed with coverage analysis (see docs\/session-bug.md) |/' docs/PRD-session-parity.md

# Commit parity update alongside the fix
git add docs/PRD-session-parity.md
git commit -m "docs: update session parity for #43 - persistence on all exit paths"

# Now the branch has both bug fix AND parity update
```

#### 2.10 Create Pull Request

```bash
gh pr create \
  --title "#43: Save session on tool error before returning" \
  --body "**Closes:** #43
**Bug:** Session data lost on tool error
**Root cause:** Missing save() call before error return
**Coverage gap fixed:**
- Integration tests only tested success path
- Added test_save_before_error (failing before fix)

**Changes:**
- Added session.save() before returning Err
- Added integration test with mock verification
- Added unit test for save logic
- Added live test with real LLM
- Added bug tracking at docs/session-bug.md
- **Parity file updated** (✅) in this PR

**Test results:**
```bash
# All pass after fix
cargo test --package oben-sessions --lib
cargo test --package oben-sessions --test '*'
cargo test --package oben-scenario-test
```

**Checklist:**
- [x] Bug tracking file created
- [x] Reproducing test written first (FAILED)
- [x] Fix implemented (PASSED)
- [x] All tests pass
- [x] Parity file updated (✅) in this PR" \
  --label "bug,priority-critical,ready-for-review"
```

#### 2.11 Merge PR (includes parity update)

```bash
gh pr merge #43 --merge --delete-branch
```

#### 2.12 Close Issue with Summary

```bash
gh issue close 43 --comment "✅ Fixed session data loss on tool error.

**Root cause:** Missing save() before error return

**Coverage gap analysis:**
- Unit tests: Only tested happy path
- Integration tests: All mocks simulated success
- Live tests: No error path coverage

**Added tests:**
- Unit: test_save_called_on_error (src/session.rs:120)
- Integration: test_save_before_error (tests/session_tests.rs:45)
- Live: test_live_error_recovery (oben-scenario-test/tests/live_session.rs:78)

**Post-merge:**
- [x] Parity file updated in PR #43: docs/PRD-session-parity.md row 5 now ✅
- [x] Bug tracking at docs/session-bug.md

**Prevention:** All future error paths will include save verification."

# Delete branch (if not auto-deleted)
git branch -d #43-fix-session-data-loss
git push origin --delete #43-fix-session-data-loss
```

---

### Summary: Parity File Update Flow

```
Before PR:
    Parity file shows ❌ for the feature
    (Work in feature branch)

During PR preparation:
    1. Update parity file in the SAME branch
    2. Change ❌ to ✅ (#issue-number)
    3. Add issue link to Notes column
    4. Commit with "docs: update {area} parity for #{issue}"
    5. Include in the SAME PR

PR created → reviewed → merged
    ↓
AFTER MERGE:
    1. Close issue with summary (mention parity was updated in PR)
    2. Delete branch
```

**Critical: Parity file IS updated in the feature branch, as part of the same PR that implements the feature.**

This way:
- One PR contains both code + parity update
- One commit to main (not two)
- Parity is reviewed alongside the implementation
- No extra post-merge commit

---

### The ONLY commands you run during development:

```bash
# UNIT TESTS (in src/*.rs)
cargo test --package oben-sessions --lib

# INTEGRATION TESTS (in tests/*.rs)
cargo test --package oben-sessions --test '*' -- --nocapture

# SINGLE INTEGRATION TEST
cargo test --package oben-sessions --test session_tests test_name

# LIVE TESTS (after unit+integration pass)
cargo test --package oben-scenario-test --test live_session -- --nocapture

# NEVER RUN THESE:
cargo test --workspace  # Wastes 5+ minutes
cargo test --all        # Same problem
```

### Branch & PR naming rules (ENFORCED)

| Type | Branch | PR Title |
|------|--------|----------|
| New feature | `#42-memory-provider` | `#42: Add MemoryProvider trait` |
| Bug fix | `#43-fix-session-data-loss` | `#43: Save session on all error paths` |
| Refactor | `#44-refactor-persistence` | `#44: Extract persistence layer` |
| Documentation | `#45-docs-api` | `#45: Add API documentation` |

**Rules:**
- Branch must start with `#<number>-`
- PR title must start with `#<number>: `
- Issue number must match branch/PR number
- One issue per branch (no bundling)

---

### Breaking these rules = immediate rework:

**Thinking/Simplicity violations (Section 1-4):**
- ❌ Writing code without `## 🤔 Thinking` block
- ❌ Adding traits/abstractions for single-use code
- ❌ "Improving" adjacent code not related to the change
- ❌ Cannot state goal as a failing test

**GitHub workflow violations:**
- ❌ Writing code without creating issue first
- ❌ Branch name missing issue number
- ❌ PR title missing issue number
- ❌ Merging without closing issue
- ❌ Leaving branch after merge
- ❌ One branch for multiple issues
- ❌ **Parity file missing from PR** (must be in same PR)

**Testing violations:**
- ❌ Writing `impl` before a failing test
- ❌ Putting unit test in `tests/` instead of `src/*.rs`
- ❌ Putting network call in integration test (use mocks)
- ❌ Running live test without `~/.obenagent/config.yaml`
- ❌ Fixing bug without `docs/{area}-bug.md`
- ❌ Fixing bug without reproducing test
- ❌ Guessing behavior without checking Hermes source
- ❌ Running `cargo test --workspace`
- ❌ Forgetting `given/when/then` comments

**Parity violations:**
- ❌ Merging without parity file update (parity must be in the PR)
- ❌ Not linking issue to parity file row
- ❌ Creating separate parity commit after merge (should be in feature branch)

---

### Error handling decision tree:

```
Starting a change?
    ├─ Output `## 🤔 Thinking` block
    ├─ Create GitHub issue (get number N)
    ├─ Create branch #N-desc
    └─ Continue

Is behavior unclear?
    ├─ YES → Check Hermes source (~/workspace/hermes-agent/src/)
    │         ├─ Found reference → Use as spec
    │         └─ Still unclear → ASK with specific lines
    └─ NO → Continue

Is this a bug or feature?
    ├─ BUG → Create docs/{area}-bug.md
    │         ├─ Analyze coverage gaps (unit/integration/live)
    │         ├─ Write reproducing test in correct tier
    │         ├─ Test must FAIL
    │         └─ Then fix
    └─ FEATURE → Write tests in order (unit→integration→live)
                  All must FAIL

Which test tier?
    ├─ Pure logic, no I/O → Unit test in src/*.rs
    ├─ Needs mocks, I/O → Integration test in tests/*.rs
    ├─ Real LLM, config → Live test in oben-scenario-test/
    └─ Don't know → Stop and ask

Tests written and failing?
    ├─ YES → Implement
    └─ NO → STOP. Write tests first.

Implementation done?
    ├─ Run tests (unit→integration→live)
    ├─ All pass? → Update parity file, commit, create PR
    └─ Some fail? → Fix

PR merged?
    ├─ YES → 
    │   ├─ Close issue with summary (mention parity was in PR)
    │   └─ Delete branch
    └─ NO → Wait for merge

Did you just write `impl` without tests?
    ├─ YES → STOP. Delete it. Start over from issue creation.
    └─ NO → Good, continue
```

---

### Quick reference checklist (copy this for each change)

```markdown
## Change Checklist

**Before coding (Section 1-4):**
- [ ] `## 🤔 Thinking` block written
- [ ] Assumptions stated
- [ ] Unclear points resolved
- [ ] Simpler approach considered
- [ ] Goal stated as verifiable test

**GitHub:**
- [ ] Issue created: #_____
- [ ] Branch created: #_____-_____
- [ ] PR created: #_____: _____
- [ ] PR merged
- [ ] Issue closed with summary (mentioning parity in PR)
- [ ] Branch deleted (local + remote)

**Testing:**
- [ ] Hermes reference checked
- [ ] Unit test written (src/*.rs) - FAILS
- [ ] Integration test written (tests/*.rs) - FAILS
- [ ] Live test written (oben-scenario-test/) - FAILS (if needed)
- [ ] Implementation done
- [ ] Unit test passes
- [ ] Integration test passes
- [ ] Live test passes (if applicable)
- [ ] No `cargo test --workspace` used

**Bug fixes only:**
- [ ] docs/{area}-bug.md created
- [ ] Coverage gap analyzed
- [ ] Reproducing test written first

**Parity (in feature branch, before PR):**
- [ ] Updated parity file (❌ → ✅ (#issue))
- [ ] Added issue link to Notes column
- [ ] Committed parity update in feature branch
- [ ] Parity update included in PR

**Code quality:**
- [ ] All tests have `given/when/then` comments
- [ ] No guessing - Hermes behavior matched
- [ ] No unrelated changes (surgical)
- [ ] No over-engineering (simplicity first)
```

---

**These guidelines are working if:** 
- Every change starts with a `## 🤔 Thinking` block
- No traits/abstractions for single-use code
- Every change has an issue, branch, PR, and closed issue
- Parity file is updated in the SAME PR as the implementation
- Only one commit to main per feature (code + parity together)
- Branches are deleted after merge
- Unit tests live inside source files
- Integration tests use mocks only
- Live tests read from `~/.obenagent/config.yaml`
- Every bug fix has `docs/{area}-bug.md` with coverage gap analysis
- Every PR includes tests written before implementation
- You check Hermes source before asking questions
- No guessing - only references and verified behavior
- Issue close comments always mention parity update was in the PR