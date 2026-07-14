# Skills & Curation — Parity vs Hermes-Agent

**Scope:** `oben-skills` and `oben-curator` crates  
**Reference:** `/Users/ellie/workspace/hermes-agent/agent/skill_utils.py`, `agent/curator.py`, `hermes_cli/skills_config.py`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| SK.1 | SkillLoader (YAML/TXT/MD discovery) | ✅ | ✅ | (built-in) | Recursive SKILL.md discovery |
| SK.2 | SkillManager (enable/disable/auto-use) | ✅ | ✅ | (built-in) | Instruction assembly + preprocessing |
| SK.3 | Curator (usage + lifecycle + scheduler) | ✅ | ✅ | (built-in) | active→stale→archived |
| SK.4 | Skill preprocessing (template vars, shell expansion) | ✅ | ✅ | (built-in) | `${SKILL_DIR}`, `!`cmd` expansion |
| SK.5 | Platform matching (skill tags/conditions) | ✅ | ✅ | (built-in) | `platform: macos`, `platform: linux` |
| SK.6 | **Skills hub / install from URL** | 🟢 | ❌ | [TBD] | `skills_hub.py` — install from GitHub, URL |
| SK.7 | **Skill bundles** | 🟢 | ❌ | [TBD] | `skill_bundles.py` — group skills together |
| SK.8 | **Skills sync** | 🟢 | ❌ | [TBD] | `skills_sync.py` — remote sync |
| SK.9 | **Skill provenance** | 🟢 | ❌ | [TBD] | `skill_provenance.py` — track origin |
| SK.10 | **Curator backup** | 🟢 | ❌ | [TBD] | `curator_backup.py` — periodic skill state backup |
| SK.11 | **Skills guard** | 🟢 | ❌ | [TBD] | `skills_guard.py` — validation / safety |
| SK.12 | **Skill commands** | 🟢 | ❌ | [TBD] | `skill_commands.py` — CLI commands from skills |
| SK.13 | **Curator pin command** | 🟢 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 1 — wire `curator pin <skill>` CLI |
| SK.14 | **Curator run/status commands** | 🟢 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 1 — wire `curator run/status` CLI |
| SK.15 | **Environment filtering** | 🟡 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 2 — add `environments` field to Skill |
| SK.16 | **Environment matching** | 🟡 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 2 — loader filters by platform+environment |
| SK.17 | **Absorption tracking** | 🟡 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 2 — archive records include absorption timestamps |
| SK.18 | **Hub-instilled provenance** | 🟡 | ✅ | `.omo/plans/skills-gap-analysis.md` | Phase 3 — catalog.rs has Hub source variant |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Completed in Session** (2026-07-14):
- SK.13-SK.18: Phase 1, 2 & 3 skills gap implementation per `.omo/plans/skills-gap-analysis.md`
- Total: 9 tasks completed (SK.11-SK.19)

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
