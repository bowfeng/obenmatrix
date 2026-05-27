# Utilities — Parity vs Hermes-Agent

**Scope:** `oben-utils` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/agent/`, `/Users/ellie/workspace/hermes-agent/utils.py`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| U.1 | Logging (tracing-subscriber) | ✅ | ✅ | (built-in) | `oben-utils/logging.rs` |
| U.2 | Terminal spinner / progress | ✅ | ✅ | (built-in) | `oben-utils/terminal.rs` |
| U.3 | Path security | ✅ | ✅ | (built-in) | `oben-utils/path_security.rs` |
| U.4 | Table formatter | ✅ | ✅ | (built-in) | `oben-utils/table.rs` |
| U.5 | **Rate limit tracker** | 🟢 | ❌ | [TBD] | per-provider rate limiting |
| U.6 | **Usage pricing** | 🟡 | ❌ | [TBD] | cost estimation per call |
| U.7 | **Credential management** | 🟡 | ❌ | [TBD] | credential sources, credential pool |
| U.8 | **Redact / sanitize** | 🟢 | ✅ | [TBD] | PII redaction (`PR #99`, implemented inline) |
| U.9 | **URL safety** | 🟢 | ✅ | [TBD] | URL validation (`PR #99`, implemented inline) |
| U.10 | **File safety** | 🟢 | ✅ | [TBD] | safe file operations (`PR #99`, implemented inline) |
| U.11 | **Checkpoint system** | 🟡 | ❌ | [TBD] | save/restore state |
| U.12 | **Trajectory compressor** | 🟡 | ❌ | [TBD] | conversation compression |
| U.13 | **Debug helpers** | 🟡 | ✅ | [TBD] | debugging utilities (`PR #99`, implemented inline) |
| U.14 | **Clipboard** | 🟢 | ✅ | [TBD] | clipboard integration (`PR #99`, implemented inline) |
| U.15 | **Security advisories** | 🟢 | ❌ | [TBD] | CVE checks |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
