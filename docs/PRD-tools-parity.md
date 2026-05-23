# Tools — Parity vs Hermes-Agent

**Scope:** `oben-tools` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/tools/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| TL.1 | ToolRegistry + auto-registration | ✅ | ✅ | (built-in) | Dynamic registration, dispatch |
| TL.2 | Shell tool (safe command execution) | ✅ | ✅ | (built-in) | Terminal fg/bg, path security |
| TL.3 | read_file / write_file | ✅ | ✅ | (built-in) | File I/O tools |
| TL.4 | http_get | ✅ | ✅ | (built-in) | HTTP fetch tool |
| TL.5 | web_search | 🟡 | ✅ | (built-in) | Stub with configurable provider |
| TL.6 | search_files (ripgrep) | ✅ | ✅ | (built-in) | `rg`-backed file search |
| TL.7 | patch (fuzzy) | ✅ | ✅ | (built-in) | Multi-search-replace patch |
| TL.8 | web_extract (SSRF + HTML) | ✅ | ✅ | (built-in) | HTML content extraction |
| TL.9 | memory (add/replace/remove) | ✅ | ✅ | (built-in) | Memory CRUD with injection scanning |
| TL.10 | clarify | ✅ | ✅ | (built-in) | Clarification tool |
| TL.11 | todo (JSON store) | ✅ | ✅ | (built-in) | Todo list tool |
| TL.12 | code_execution (sandbox) | ✅ | ✅ | (built-in) | Sandboxed code execution |
| TL.13 | osv_check | ✅ | ✅ | (built-in) | OSV vulnerability check |
| TL.14 | skill (list/view) | ✅ | ✅ | (built-in) | Skill management tool |
| TL.15 | **Search provider** (DuckDuckGo, Brave) | 🟡 | ❌ | [TBD] | Configurable search backend |
| TL.16 | **Browser automation** (CUA-driver) | 🟡 | ❌ | [TBD] | `browser_dialog_tool.py` → `cua-driver` |
| TL.17 | **Voice** (STT/TTS) | 🟡 | ❌ | [TBD] | Whisper, Edge TTS, ElevenLabs |
| TL.18 | **Image generation** (FLUX, DALL-E, Midjourney) | 🟡 | ❌ | [TBD] | `image_gen_provider.py` |
| TL.19 | **MCP integration** | 🟢 | ❌ | [TBD] | `mcp_oauth.py`, `mcp_tool.py` |
| TL.20 | **Cron scheduler** | 🟢 | ❌ | [TBD] | Scheduled task delivery |
| TL.21 | **Delegate tool** | 🟡 | ❌ | [TBD] | Subagent delegation (`delegate_tool.py`) |
| TL.22 | **Kanban tools** | 🟡 | ❌ | [TBD] | Task management board |
| TL.23 | **Computer use** | 🟡 | ❌ | [TBD] | `computer_use_tool.py` — macOS GUI control |
| TL.24 | **Video generation** | 🟢 | ❌ | [TBD] | `video_gen_provider.py` |
| TL.25 | **Home Assistant** | 🟢 | ❌ | [TBD] | `homeassistant_tool.py` |
| TL.26 | **Mixture of Agents** | 🟢 | ❌ | [TBD] | `mixture_of_agents_tool.py` |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
