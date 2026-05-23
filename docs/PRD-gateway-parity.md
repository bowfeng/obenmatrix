# Gateway & Platforms — Parity vs Hermes-Agent

**Scope:** `oben-gateway` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/gateway/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| G.1 | PlatformAdapter trait | ✅ | ✅ | (built-in) | Trait defined in `oben-gateway` |
| G.2 | **Telegram adapter** | 🔴 | ❌ | [TBD] | webhook + polling, file handling |
| G.3 | **Discord adapter** | 🔴 | ❌ | [TBD] | bot, slash commands |
| G.4 | **Slack adapter** | 🔴 | ❌ | [TBD] | RTM + Socket Mode |
| G.5 | **WhatsApp adapter** | 🟡 | ❌ | [TBD] | WA Web API |
| G.6 | **Session context per platform** | 🟡 | ❌ | [TBD] | per-platform session isolation |
| G.7 | **Delivery routing** | 🟡 | ❌ | [TBD] | platform-aware delivery |
| G.8 | **Slash command routing** | 🟢 | ❌ | [TBD] | /pause, /resume, /status |
| G.9 | **Pairing** | 🟢 | ❌ | [TBD] | user ↔ platform registration |
| G.10 | **Memory monitor** | 🟢 | ❌ | [TBD] | memory usage tracking |
| G.11 | **Sticker cache** | 🟢 | ❌ | [TBD] | media caching |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
