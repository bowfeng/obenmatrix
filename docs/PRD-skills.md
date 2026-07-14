# Skills & Curation — LLM-Driven Consolidation

**Reference:** `hermes-agent/agent/curator.py:403-554` (CURATOR_REVIEW_PROMPT)

---

## Overview

Hermes-Agent includes an LLM-driven consolidation pass that runs in the background via the `curator.py` skill. This pass **consolidates narrow skills into class-level umbrella skills** to improve the skill library's discoverability and maintainability.

**ObenAgent Status:** 🟡 **Planned** — Document the approach; no implementation yet.

> **Key distinction:** Hermes consolidation is **LLM-driven** (not automatic/programmatic). It requires an LLM call to analyze skill clusters and make consolidation decisions. ObenAgent's curator currently only tracks usage and lifecycle states.

---

## Goal

The skill collection should be a **library of class-level instructions and experiential knowledge**, not a collection of hundreds of narrow skills.

- ✅ **Correct:** Broad umbrella skills with labeled subsections
- ❌ **Failure:** One-session-one-skill micro-entries

An agent searching skills matches on **descriptions**, not exact names. One broad umbrella skill beats five narrow siblings for discoverability.

---

## Approach

### 1. Umbrella-Building Pattern

The curator identifies **prefix clusters** — skills sharing a first word or domain keyword — and consolidates them under class-level umbrellas.

**Examples of clusters to identify:**
- `hermes-config-*`, `hermes-dashboard-*`
- `gateway-*`, `codex-*`, `ollama-*`
- `anthropic-*`, `gemini-*`, `mcp-*`
- `salvage-*`, `pr-*`, `competitor-*`
- `python-*`, `security-*`

**Three consolidation tactics:**

| Method | When to Use | Tool |
|--------|-------------|------|
| **Merge into existing umbrella** | One skill already covers the class broadly | `skill_manage action=patch` |
| **Create new umbrella** | No existing member is broad enough | `skill_manage action=create` |
| **Demote to support files** | Narrow content is session-specific | `skill_manage action=write_file` + archive |

Support file re-homing targets:
- `references/<topic>.md` — session-specific detail, quoted research, API docs
- `templates/<name>.<ext>` — starter files meant to be copied/modified
- `scripts/<name>.<ext>` — re-runnable verification/fixture/probe scripts

### 2. Priority Rules for Skill Merging

**Hard rules (must not violate):**

1. **External-dir skills untouched** — Don't modify skills in `skills.external_dirs` (externally owned, read-only)
2. **No deletion** — Maximum destructive action is **archiving** (moving to `~/.hermes/skills/.archive/`)
3. **Pinned skills skipped** — Skip skills with `pinned=yes`
4. **Protected built-ins preserved** — Never archive/modify skills in the protected built-ins list (currently: `plan`)
5. **Cron skills protected** — Don't prune `cron=yes` skills (but may consolidate them; cron references get rewritten)

**Anti-rules (how NOT to decide):**

- ❌ Never archive based solely on `use_count` — zero usage is not evidence of value or obsolescence
- ❌ Never reject consolidation on "distinct triggers" — the right bar is whether a human would write N separate skills or one umbrella with subsections

### 3. Absorption vs Pruning Decision Logic

| Decision | Criteria | Tool action |
|----------|----------|-------------|
| **Absorb** | Content is reusable, relevant, not obsolete | `skill_manage action=patch` or `create`, archive source with `absorbed_into=<umbrella>` |
| **Demote to reference** | Session-specific detail with broad relevance | Move to `references/`, `templates/`, or `scripts/`; archive old skill |
| **Prune (archive only)** | Truly stale, obsolete, irrelevant (≥30 days old) | `skill_manage action=delete` with `absorbed_into=""` |

**Archive naming convention for source skills:**
- `consolidations`: Skills merged into umbrellas (`absorbed_into=<umbrella>` set)
- `prunings`: Skills archived with no forwarding target (`absorbed_into=""`)

---

## Future ObenAgent ImplementationPlan

### Phase 1: Planning & Documentation

- ✅ **This task** — Document Hermes consolidation approach in `docs/PRD-skills.md`
- List current prefix clusters in ObenAgent skill directory
- Identify candidate umbrella skills for each cluster

### Phase 2: CLI Integration (Manual Trigger)

- Add `oben curator consolidate` subcommand
- Trigger LLM call with curated skill list
- Parse structured summary output (YAML block from Hermes)
- Apply archives / patches only after user confirmation

### Phase 3: Scheduled Runs (Optional)

- Wire curator scheduler to run consolidation pass periodically
- Add config flag: `curator.consolidation_enabled`
- Write structured summaries to `~/.hermes/logs/curator/` (same format as Hermes)

### Phase 4: LLM Integration

- Use ObenAgent's LLM provider chain (fallback, retry, streaming)
- Craft prompt matching Hermes' `CURATOR_REVIEW_PROMPT`
- Implement YAML block parsing for structured output

---

## Reference: Hermes Agent Prompt (abridged)

```python
CURATOR_REVIEW_PROMPT = (
    "You are running as Hermes' background skill CURATOR. This is an "
    "UMBRELLA-BUILDING consolidation pass, not a passive audit and not a "
    "duplicate-finder.\n\n"

    "# Hard rules (do not violate):\n"
    "1. DO NOT touch bundled, hub-installed, or external-dir skills\n"
    "2. DO NOT delete any skill. Archiving is maximum destructive action\n"
    "3. DO NOT touch skills shown as pinned=yes\n"
    "3b. DO NOT modify protected built-ins (currently: plan)\n"
    "3c. DO NOT archive cron=yes skills (but may consolidate)\n"
    "4. DO NOT use usage counters as reason to skip/merge\n"
    "5.DO NOT reject on 'distinct triggers' — judge on class-level utility\n\n"

    "How to work:\n"
    "1. Scan candidate list. Identify PREFIX CLUSTERS (skills sharing first word)\n"
    "2. For each cluster with 2+ members, ask: 'what is the UMBRELLA CLASS?'\n"
    "3. Three ways to consolidate:\n"
    "   a. MERGE INTO EXISTING UMBRELLA\n"
    "   b. CREATE A NEW UMBRELLA SKILL.md\n"
    "   c. DEMOTE TO REFERENCES/TEMPLATES/SCRIPTS\n"
    "4. Package integrity: move all support files, rewrite paths, or archive whole package\n"
    "5. Iterate until no obvious clusters remain\n\n"

    "Expected output: human summary + YAML structured block:\n"
    "## Structured summary (required)\n"
    "```yaml\n"
    "consolidations:\n"
    "  - from: <old-skill-name>\n"
    "    into: <umbrella-skill-name>\n"
    "    reason: <one short sentence>\n"
    "prunings:\n"
    "  - name: <skill-name>\n"
    "    reason: <one short sentence>\n"
    "```\n\n"

    "Every skill archived MUST appear in exactly one list."
)
```

---

## Notes

- **This is NOT automatic** — requires LLM reasoning, not simple heuristics
- **Umbrella skills** should have rich `SKILL.md` bodies + `references/`, `templates/`, `scripts/`
- **Session-specific content** moves into support files under umbrellas, not as new skills
- **Priority:** Hub > External > User > Plugin > Builtin (inherited)
- **Target shape:** 10-25 clusters processed; ≥10 archives per pass indicates insufficient effort

---

## See Also

- [`docs/PRD-skills-parity.md`](./PRD-skills-parity.md) — Feature gaps vs Hermes
- `hermes-agent/agent/curator.py:403-554` — Full `CURATOR_REVIEW_PROMPT`
- `oben-curator/src/curator.rs` — Current usage + lifecycle implementation
