# AGENTS.md

## 🚀 Core Coding Principles

### 1. Think Before Coding (Mandatory Output Format)
Before implementing **ANYTHING**, you MUST first and independently output the following block. **If you have any questions or unclear points, STOP and ask before writing code. Do not guess.**

```markdown
## 🤔 Thinking: [Task #N]

**1. Core Assumptions & Intentions:**
- [Assumption 1] -> Expected impact/outcome
- [Assumption 2]

**2. What I'm Unclear About (Unclarities):**
- [Question 1 - if none, write "None"]

**3. Simpler Approach & Architectural Tradeoffs:**
- Complex Alternative Considered: [e.g., Introducing a new Trait/Generic/Builder]
- Chosen Simple Approach: [Why the most direct, abstraction-free approach was selected]
- Tradeoffs/Limitations: [Boundaries of this simple choice]

```

### 2. Simplicity First

* **No Speculative Abstraction**: Write the minimum code needed to pass the current tests. **Never write a trait if it is used in exactly one place.**
* **Lean Error Handling**: Use `anyhow` or existing explicit `thiserror` variants for actual recoverable errors. Do not construct complex custom error trees for unreachable or unhandled cases.

### 3. Surgical Changes

* **Own Your Mess**: Clean up or modify only what is **directly affected by your change** (e.g., unused variables you introduced, comments your code made obsolete).
* **Leave Adjacent Code Alone**: If you spot pre-existing typos, unused imports, or inaccurate comments in adjacent code, **do not touch them**. Mention them as a `NOTE:` in your PR or create a separate GitHub issue.

---

## 🦀 Idiomatic Rust Style & Design Patterns

To ensure maximum performance and scannability, you must write idiomatic, modern Rust. Reject traditional Object-Oriented Design Patterns (e.g., Factories, single-use Builders, Strategy Patterns via heavy trait objects) in favor of data-driven Rust patterns.

### 1. Data Modeling over Polymorphism

* **Enums over Traits**: Use algebraic data types (enums) paired with `match` blocks for handling variant behavior instead of dynamic trait objects (`Box<dyn Trait>`), unless the system explicitly requires open-ended, runtime-pluggable extensions.
* **Composition over Inheritance**: Model data structures cleanly using plain structs. Avoid deeply nested abstraction layers.

### 2. Lifetimes, Ownership, and Smart Pointers

* **Prefer References**: Pass data by mutable reference (`&mut`) or shared reference (`&`) down the call stack instead of defaulting to thread-safe smart pointers (`Arc<Mutex<T>>` or `Arc<RwLock<T>>`), unless shared multi-threaded ownership is an absolute prerequisite.
* **Explicit Cloning**: Never hide allocations. If an explicit `.clone()` is necessary to satisfy the borrow checker, evaluate if moving ownership (`self`) or passing references is possible first.

### 3. Errors, Options, and Combinators

* **Idiomatic Control Flow**: Rely on functional combinators (`.map()`, `.and_then()`, `.unwrap_or_else()`) for cleaner, localized logic handling instead of deeply nested `if let` blocks.
* **No Silent Failures**: Propagate errors explicitly using the `?` operator. Never hide critical subsystem errors behind broad unlogs or silent `Default::default()` returns.

---

## 📄 Documentation Architecture

Documentation is bifurcated into high-level specs and explicit parity trackers to strictly catalog system behavior against the reference Python implementation (`hermes-agent`), which is at ~/workspace/hermes-agent

### 1. Directory & File Map

* `docs/PRD.md` — Overall system architecture, high-level milestones, and macro progress trackers.
* `docs/PRD-{area}.md` — Design, internal architecture, and specs for our current feature layout. **All brand-new, custom functionality unrelated to parity gaps lives here.**
* `docs/PRD-{area}-parity.md` — Feature-area parity trackers containing explicit gap ledgers vs Hermes-Agent. One file per area: `session`, `transport`, `conversation`, `tools`, `skills`, `gateway`, `goals`, `cli`, `utils`.

### 2. Parity Table Formatting Schema

Every parity tracking file contains a table structured exactly like the following block. You must maintain this exact formatting layout when executing updates:

```markdown
| Feature Matrix / Gap | Severity | Status | GitHub Issue | Reference File & Lines | Notes / Context |
| :--- | :--- | :--- | :--- | :--- | :--- |
| MemoryProvider prefetch | priority-high | ✅ | [#42](https://github.com/.../42) | `memory/provider.py:42-58` | Verified parity with TDD coverage |
| In-memory caching layer | priority-medium | ❌ | None | `memory/cache.py:12-24` | Gap identified; needs tracking issue |

```

---

## 🛠 Task Routing Workflow

Upon receiving a task, the Agent must automatically classify it into one of the following four tracks and strictly execute its steps:

```
                    ┌──────────────────────────────┐
                    │        Received Task         │
                    └──────────────┬───────────────┘
                                   │
         ┌─────────────────────────┼────────────────────────┐
         ▼                         ▼                        ▼
【 Track A: Feature/Parity 】  【 Track B: Bug Fix 】  【 Track C/D: Refactor/Enhance 】
   ├─ 1. Gap Analysis & Plan     ├─ 1. Create Bug Doc    ├─ 1. Branch Setup
   ├─ 2. Check Hermes Source     ├─ 2. Write BDD Test    ├─ 2. Run Existing Tests (Green)
   ├─ 3. Three-Tier TDD (BDD)    ├─ 3. Implement Fix     ├─ 3. Refactor / Enhance Code
   └─ 4. Inline Doc Update       └─ 4. Verify Bounds     └─ 4. Verification Check (Green)

```

---

### Track A: New Feature / Parity Gap

Use this when introducing a new capability, closing a discrepancy with the reference repository (`hermes-agent`), or adding brand-new custom functionality.

#### 📋 1. Mandatory Feature Analysis & Execution Plan

Directly below your Thinking block, you must determine if the feature is a **Parity Gap Match** or a **New Custom Feature**, then output the appropriate analytical block:

##### Path A1: If it is a Parity Gap Match

```markdown
## 🔍 Feature Parity Gap Analysis: [Task #N]

**Reference File:** `~/workspace/hermes-agent/[path/to/source.py]`
**Target Rust Crate/Module:** `src/[module]/[file.rs]`
**Parity Document Location:** `docs/PRD-[area]-parity.md`

**Gap Identification:**
| Feature Matrix | Hermes-Agent (Reference) | Our Rust Implementation (Current) | Action Required |
| :--- | :--- | :--- | :--- |
| [Component State] | Tracks retry counts internally | Missing entirely from struct | Add `retries: u32` to session state |

```

##### Path A2: If it is a New Custom Feature (Not in Hermes-Agent)

```markdown
## 🗺️ Custom Feature Architecture Spec: [Task #N]

**Target Rust Crate/Module:** `src/[module]/[file.rs]`
**Specification Document Location:** `docs/PRD-[area].md`

**Functional Specification Additions:**
- **System Impact:** [Describe how this custom feature hooks into existing architecture]
- **New Structs/Interfaces:** [Outline the new inputs, outputs, or configuration keys]
- **Documentation Action:** Append this new feature design to `docs/PRD-[area].md` under a dedicated `# Custom Extensions` or feature header in this same branch.

```

##### 📋 Shared Execution Plan for #N

```markdown
## 📋 Execution Plan for #N

**Goal:** [One-sentence description of the capability]

**Automated Verification Criteria (BDD Mapping):**
1. **Unit Test** (`src/path/to/mod.rs`): 
   - *Given*: [Initial local state/mock-free input]
   - *When*: [Method or logic is executed]
   - *Then*: [Pure logical assertion occurs]
2. **Integration Test** (`tests/feature_tests.rs`):
   - *Given*: [Mock environment configurations applied]
   - *When*: [Crate boundary API is invoked]
   - *Then*: [Verify internal mocks intercept correctly]
3. **Live Test** (`oben-scenario-test/tests/live_*.rs`):
   - *Given*: [Valid config inside ~/.obenalien/config.yaml]
   - *When*: [Full agent workflow runs with real LLMs]
   - *Then*: [Verify output semantic correctness]

**Steps:**
1. Write the 3-layer FAILING tests exactly following the BDD specs above.
2. Implement core logic using idiomatic Rust design patterns until all 3 layers pass sequentially.
3. **Update Ledger:** - *If Parity Match*: Update the targeted row in `docs/PRD-*-parity.md` directly to ✅.
   - *If Custom Feature*: Document the new design directly inside `docs/PRD-*-{area}.md`.

```

#### 2. Test Architecture & BDD Code Formatting

Every single test function block MUST contain structured markdown-styled Doc Comments clearly outlining the Given/When/Then scenario. No exceptions.

* **Unit Tests**: Live inside `src/*.rs` under `#[cfg(test)]`. **Absolutely no I/O, network calls, or filesystem access.**
* **Integration Tests**: Live in `tests/*.rs`. **Must use mocks exclusively** to isolate boundaries. No real network dependencies.
* **Live Tests**: Live in `oben-scenario-test/tests/*.rs`. Executes full-stack sequences against real LLMs using configurations dynamically loaded from `~/.obenalien/config.yaml`.

```rust
/// BDD Test Block Example
/// Given: An empty memory provider configuration
/// When: A prefetch action is requested on an existing key
/// Then: The system returns a clear PrefetchError instead of panicking
#[test]
fn test_prefetch_behavior() {
    // Test code follows...
}

```

---

### Track B: Bug Fixes

Use this when rectifying unintended failures or broken behavior. **Rule: No code without a reproducing BDD test.**

#### 1. Mandatory Bug Tracking File

Before writing any reproduction code, create a tracking document at `docs/bugs/bug-#N-[desc].md`:

```markdown
## 🐛 Bug #N: [Brief Description]

**Root Cause Analysis:**
- Core Vulnerability: [Why the logic fails under specific conditions]
- Coverage Gaps:
  - Unit Layer Missed Because: [e.g., Pure logic branch did not test error state]
  - Integration Layer Missed Because: [e.g., Mocks only simulated successful returns]

**Regression BDD Test Scenario:**
- **Given**: [Precise conditions that cause the defect to trigger]
- **When**: [The broken path is executed]
- **Then**: [Assertion proving the bug is eradicated]
- **Location**: Adding `test_fix_validation` to [Target Test Tier]. Current execution must result in FAILING.

```

#### 2. Fix Sequence

1. Write the reproducing test using explicit BDD comments and run it to verify it **FAILS** (Red light).
2. Apply surgical changes to fix the logic and verify the test **PASSES** (Green light).
3. If this fix remedies a known parity gap, flip the status in the relevant parity MD document.

---

### Track C: Code Refactoring

Use this for internal design improvements, optimization, or clean-up **without changing external APIs or behaviors**.

#### ⚙️ Streamlined Refactor Flow

Refactoring does not require writing new test tiers. The priority is **ensuring zero regression**:

1. **Establish Baseline**: Run existing tests for the target package before making changes to ensure everything is Green:
```bash
cargo test --package oben-[module] --lib

```


2. **Atomic Changes**: Modify the structure in small, incremental steps. Never attempt multi-module sweeping refactors in one pass. Ensure refactors improve adherence to idiomatic Rust patterns (e.g., replacing trait abstractions with cleaner enums).
3. **Regression Check**: Run the target package test suite frequently during changes.
4. **No Piggybacking**: Do not pack new feature code or unrelated bug fixes into a refactoring branch.

---

### Track D: Enhancements

Use this for non-parity improvements, non-breaking micro-adjustments, or feature additions (e.g., introducing a cache layer, expanding logs, improving method ergonomics).

#### 📈 Enhancement Protocol

1. **Classify Boundary**: Determine if the change is strictly internal (falls back to **Track C: Refactor**) or modifies input/output/configuration boundaries.
2. **Extend Contract**: If input/output behaviors change, write a new test matching the expected behavior first using the standard **Given/When/Then** comment syntax (FAILING), then implement the enhancement until it passes.

---

## 🧬 Git Automation & GitHub Protocol

Branch lifecycles and documentation updates must be entirely atomic and self-contained within a single Pull Request.

```
[Local Branch #N-desc] ──> [Failing Test & Implementation] ──> [Modify Docs in same branch] ──> [PR Submission] ──> [Merge & Purge Branch]

```

### 1. Strict Naming Conventions

| Task Type | Branch Naming Convention | Pull Request Title Convention |
| --- | --- | --- |
| **Feature / Parity** | `#42-memory-provider` | `#42: Add MemoryProvider trait` |
| **Bug Fix** | `#43-fix-session-loss` | `#43: Save session on all error paths` |
| **Refactor** | `#44-refactor-storage` | `#44: Extract inner persistence storage` |
| **Enhancement** | `#45-enhance-logging` | `#45: Add structured contextual tracer` |

### 2. Atomic Documentation Updates

* **No Post-Merge Patches**: Changing feature states from `❌` to `✅ (#N)` within `docs/PRD-*-parity.md` or adding custom configurations into `docs/PRD-{area}.md` **must be done in the same local feature branch and committed alongside the code**.
* The PR body must explicitly state that the documentation files are updated. Once the PR merges, the spec ledger on `main` is automatically up-to-date.

### 3. Target-Specific Testing (Never test the workspace)

To avoid massive build compilation waits, **never** invoke global workspace test commands. Target specific packages or tests directly:

```bash
# ✅ Recommended: Test logic inside a specific crate
cargo test --package oben-sessions --lib

# ✅ Recommended: Run a specific integration test file
cargo test --package oben-sessions --test session_tests

# ❌ Forbidden: Triggers full workspace compilation and massive wait times
cargo test --workspace
cargo test --all

```

---

## 🚨 Defensive Rejection Checklist

Before submitting or asking a human for code review, verify your changes against this list. Non-compliance requires an immediate restart:

* [ ] Did I generate the `## 🤔 Thinking` block before altering any file?
* [ ] Did I write out either the `## 🔍 Feature Parity Gap Analysis` or the `## 🗺️ Custom Feature Architecture Spec` block? (Track A)
* [ ] Do my planned automated verification criteria clearly outline the **Given / When / Then** conditions?
* [ ] Did I embed explicit **Given / When / Then** doc comments directly inside all newly created or updated test functions?
* [ ] Does the implementation honor the idiomatic Rust style guide (Enums over dynamic Traits, simple references over structural Arc allocations)?
* [ ] Did I look up the correct documentation file? (`docs/PRD-*-parity.md` for parity files or `docs/PRD-{area}.md` for brand-new custom features)
* [ ] Did I follow test-first (TDD) protocols for Features and Bugs (impl written only *after* failing tests)?
* [ ] Did I avoid modifying adjacent code or fixing arbitrary typos out of scope?
* [ ] Are the ledger and document modifications bundled within the same commit/branch as the implementation?
* [ ] Do the branch name and PR title accurately match the `#N-` and `#N:` structural prefixes?