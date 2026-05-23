# AGENTS.md

## Agent skills

### Issue tracker

Issues are tracked in this repo's GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary used: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout. See `docs/agents/domain.md`.

### PRD / Progress

The project PRD and progress tracker lives at `docs/PRD.md`. Update the progress table as work advances.

**Parity documentation layout:**
- `docs/PRD.md` — overall architecture, milestones, high-level progress
- `docs/PRD-{area}-parity.md` — feature-area parity trackers (one file per area): session, transport, conversation, tools, skills, gateway, goals, cli, utils
- Each parity file lists gaps vs Hermes-Agent with severity, status (✅/❌), and GitHub issue references
- When working on a parity gap: create a GitHub issue, link the issue to the relevant parity file row, update its status

### ADRs

Architectural decisions are tracked in `docs/adr/`.


### Coding Rules

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

## 5. Rust Project Practices & BDD Test-First

**Rust idioms + behavior-driven test-first + three-tier coverage.**

### Rust style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/): `snake_case` for functions/vars, `PascalCase` for types, `ALL_CAPS` for constants.
- Use `Result` with `anyhow` for error handling; prefer `?` over `match`.
- Prefer `impl Trait` in return position over named types. Use `impl Iterator<Item = &T>` not `std::iter::Filter<...>`.
- Derive `Default`, `Clone`, `Copy`, `Debug`, `PartialEq` where they add value without loss of meaning.
- Prefer `Arc<Mutex<T>>` for shared mutable state across threads; prefer `RwLock` when reads dominate.
- Use `tracing` for logging, not `println!` in production code.
- Crate boundaries: single responsibility.

### BDD Test-First (given / when / then)

When you have an implementation plan, **write tests before code**:

1. **Integration test first** — external behavior at the crate boundary (multi-module, real dependencies via mock).
2. **Unit test second** — pure logic in isolation.
3. **Implement** to make both pass.
4. **Live test** — after unit+integration pass, add a live test in `oben-scenario-test/` against the real LLM server.

Each test MUST start with a `given/when/then` comment block:

```rust
/// Tests that `parse_header` rejects malformed input.
///
/// given: a header string with no separator
/// when: parse_header("nodash") is called
/// then: returns Err(ParseError::MissingSeparator)
#[test]
fn test_missing_separator() { ... }
```

- **given**: preconditions — state, inputs, environment setup.
- **when**: the action being tested — the function/method call.
- **then**: expected outcome — assertions on return value, side effects, or state changes.

### Three-Tier Test Coverage

| Tier | Location | What it tests | Network |
|---|---|---|---|
| Unit | `src/*.rs` #[test] | Pure logic, no I/O | No |
| Integration | `tests/*.rs` | API boundaries, multi-module flows | No (mock) |
| Live | `oben-scenario-test/tests/` | Real LLM server, full stack | Yes |

- **Live test rules**: only in `oben-scenario-test/`, idempotent, prefix `test_live_`.
- Never add network-dependent assertions to unit or integration tests.

### GitHub Workflow

Every code change must be tracked by a GitHub issue.

1. **Open issue** if one doesn't exist. Reference the issue number in your branch name: `git checkout -b #30-fix-write-concurrency`.
2. **Work in the branch** — all commits should be small and focused on that single issue.
3. **Push and open a PR** with the title `#30: Fix write concurrency with jittered retry`.
4. **After merge** to `main`, close the related issue with a comment summarizing what was done.

Branch naming convention: `#<number>-<short-desc>` (e.g., `#31-memory-providers`, `#33-compaction-lineage`).
PR titles: `#<number>: <brief description>`.

### Parity Feature Workflow

When working on a parity feature (porting a Hermes-Agent feature):

1. **Check the parity file** — find the relevant `docs/PRD-{area}-parity.md` file
2. **Open a GitHub issue** if one doesn't exist, with severity label (`priority-critical`, `priority-high`, etc.)
3. **Reference the parity file row** — link the issue to the relevant row in the parity document
4. **Create a branch** following convention: `#<number>-<short-desc>`
5. **Implement with BDD tests**: Unit → Integration → Live (`oben-scenario-test/`)
6. **Open a PR** with title `#<number>: <description>`
7. **After merge**, close the issue and update the parity file status to ✅

### Test-Driven Execution for Parity Features

**For each parity feature, follow this exact sequence:**

1. **Open the parity file** — locate the relevant `docs/PRD-{area}-parity.md` and find the gap row
2. **Write BDD tests first** — before writing any code, write integration tests in the target crate's `tests/` and unit tests in `src/*.rs`
3. **Add given/when/then comments** — every test MUST start with this comment block:
   ```rust
   /// Tests that X does Y.
   ///
   /// given: [preconditions]
   /// when: [action]
   /// then: [expected outcome]
   ```
4. **Implement to make tests pass** — code until the tests compile and all pass
5. **Run targeted tests** — only run tests for the changed crate, NOT the full suite:
   - `cargo test --package oben-sessions` for session changes
   - `cargo test --package oben-transport` for transport changes
   - `cargo test --package oben-agent` for agent changes
6. **Add live test in `oben-scenario-test/`** — only after unit+integration tests pass, add a live test in `oben-scenario-test/tests/` against the real LLM server
7. **Run live tests** — `cargo test --package oben-scenario-test --test live_{area}`
8. **Update parity file** — change status to ✅ and link the GitHub issue

**Critical rule:** Never run `cargo test --workspace` during development. It wastes time. Only run the crate-level tests for your changes. Run the full suite only when submitting a PR.

### GitHub Workflow for Parity Features

Every parity feature MUST follow this GitHub workflow:

1. **Open issue** — create a GitHub issue referencing the parity file row
2. **Create branch** — use branch naming `#<issue-number>-<short-desc>` (e.g., `#36-persistence-on-error`)
3. **Work in the branch** — all commits should be small and focused on that single issue
4. **Push and open PR** — title format: `#<number>: <brief description>` (e.g., `#36: Save session on all error exit paths`)
5. **After merge** — close the GitHub issue with a summary of what was done

**Branch naming:** `#<number>-<short-desc>` (e.g., `#31-memory-providers`, `#36-persistence-on-error`)
**PR titles:** `#<number>: <description>` (e.g., `#31: Add MemoryProvider trait + prefetch/recall system`)

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, clarifying questions come before implementation rather than after mistakes, and every feature has three-tier test coverage.
