# Goals & Planning — Parity vs Hermes-Agent

**Scope:** `oben-goals` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/hermes_cli/goals.py`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| GO.1 | PlanNode (tree + builder) | ✅ | ✅ | (built-in) | Plan tree structure |
| GO.2 | PlanState (CRUD + markdown + save/load) | ✅ | ✅ | (built-in) | Plan persistence |
| GO.3 | Judge verdict parser | ✅ | ✅ | (built-in) | `parse_judge_response()` |
| GO.4 | GoalState (turn budget) | ✅ | ✅ | (built-in) | `GoalState` with turn limits |
| GO.5 | Plan parser (markdown) | ✅ | ✅ | (built-in) | `parse_plan_from_markdown()` |
| GO.6 | **Plan decomposition** | 🟡 | ❌ | [TBD] | `kanban_decompose.py` |
| GO.7 | **Swarm planning** | 🟡 | ❌ | [TBD] | `kanban_swarm.py` |
| GO.8 | **Kanban board** | 🟡 | ❌ | [TBD] | task board UI + logic |
| GO.9 | **Checkpoint manager** | 🟡 | ❌ | [TBD] | save/restore progress |
| GO.10 | **Session recap** | 🟢 | ❌ | [TBD] | session summary generation |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
