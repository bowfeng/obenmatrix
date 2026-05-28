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
| U.5 | **Rate limit tracker** | 🟢 | ✅ | [#100](https://github.com/bowfeng/oben-alien/pull/100) | per-provider rate limiting (`PR #100`, implemented inline) |
| U.6 | **Usage pricing** | 🟡 | ✅ ([#101](https://github.com/bowfeng/oben-alien/pull/101)) | `PR #101` | cost estimation per call |
| U.7 | **Credential management** | 🟡 | ✅ ([#102](https://github.com/bowfeng/oben-alien/pull/102)) | `credential_pool.py` | Core pool: data model (PooledCredential), rotation strategies (fill_first/round_robin/random/least_used), cooldown recovery, JSON auth persistence |
| U.8 | **Redact / sanitize** | 🟢 | ✅ | [TBD] | PII redaction (`PR #99`, implemented inline) |
| U.9 | **URL safety** | 🟢 | ✅ | [TBD] | URL validation (`PR #99`, implemented inline) |
| U.10 | **File safety** | 🟢 | ✅ | [TBD] | safe file operations (`PR #99`, implemented inline) |
| U.11 | **Checkpoint system** | 🟡 | ✅ ([#103](https://github.com/bowfeng/oben-alien/pull/103)) | `checkpoint_manager.py` | save/restore/diff/prune/list/delete, JSONL metadata, config with policies |
| U.12 | **Trajectory compressor** | 🟡 | ✅ ([#104](https://github.com/bowfeng/oben-alien/pull/104)) | `trajectory_compressor.py` | CompressionConfig, TrajectoryMetrics, AggregateMetrics, SummaryGenerator trait, NoopSummarizer, compress_trajectory, process_entry, protected regions, token counting heuristic |
| U.13 | **Debug helpers** | 🟡 | ✅ | [TBD] | debugging utilities (`PR #99`, implemented inline) |
| U.14 | **Clipboard** | 🟢 | ✅ | [TBD] | clipboard integration (`PR #99`, implemented inline) |
| U.15 | **Security advisories** | 🟢 | ✅ ([#105](https://github.com/bowfeng/oben-alien/pull/105)) | `security_advisories.py`, `osv_check.py` | Advisory catalog (Severity, Advisory, CompromisedPackage), detect_compromised with closure lookup, banner/render helpers, severity labels, OSV malware check for MCP extensions (npx/uvx/pipx ecosystem parsing) |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
