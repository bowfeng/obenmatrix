# CLI & Configuration — Parity vs Hermes-Agent

**Scope:** `oben-cli` and `oben-config` crates  
**Reference:** `/Users/ellie/workspace/hermes-agent/hermes_cli/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| CLI.1 | CLI subcommands (chat, run, setup, config, tools, skills, sessions, info, models, agent) | ✅ | ✅ | (built-in) | 9 subcommands |
| CLI.2 | YAML config + setup wizard | ✅ | ✅ | (built-in) | `AppConfig` in `oben-config` |
| CLI.3 | Model discovery (`/v1/models`) | ✅ | ✅ | (built-in) | Auto-detect max model length |
| CLI.4 | **Multi-provider config** | 🟡 | ❌ | [TBD] | provider fallback chains |
| CLI.5 | **Auth commands** | 🟡 | ❌ | [TBD] | platform authentication |
| CLI.6 | **Backup / restore** | 🟡 | ❌ | [TBD] | session + skill backup |
| CLI.7 | **Doctor / diagnostics** | 🟢 | ❌ | [TBD] | health check |
| CLI.8 | **Cron commands** | 🟡 | ❌ | [TBD] | scheduled task management |
| CLI.9 | **Profile management** | 🟢 | ❌ | [TBD] | named config sets |
| CLI.10 | **MCP config** | 🟡 | ❌ | [TBD] | MCP server configuration |
| CLI.11 | **Plugin management** | 🟡 | ❌ | [TBD] | plugin system |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
