## Bug Fix Summary: Streaming text duplication in messages panel

**PR:** [#73](https://github.com/bowfeng/obenmatrix/pull/73)
**Status:** Fixed and merged
**Severity:** High — caused visible text duplication over user messages

---

### Root Cause Analysis

**Core Vulnerability:** Two overlapping render paths both wrote to the messages panel:

1. `render_messages()` rendered streaming assistant text inline (role-colored, styled correctly)
2. `update_stream_info()` **also** wrote the streaming text preview into `state.stream_info`, which `render_turn_status()` then rendered as an overlay at `(area.x, area.y)` — the **top-left corner** of the messages panel, directly overwriting the first user message

**Coverage Gaps:**

| Layer | Missed Because |
|---|---|
| Unit | `update_stream_info()` only tested indirectly; no test verified streaming text exclusion |
| Integration | No test simulated concurrent rendering of `render_messages()` + `render_turn_status()` |

---

### Fix

**File:** `oben-tui/src/widgets/conversation.rs`

1. **Removed streaming text from `update_stream_info()`** — now only handles tool call info (`🔧 file_read /path`)
2. **Moved `render_turn_status()` position** from top-left `(area.x, area.y)` to bottom-right of messages panel, so it never overlaps message content

---

### Regression Tests Added

| Test | Scenario |
|---|---|
| `test_update_stream_info_excludes_streaming_text` | Active tool + streaming text → stream_info has NO streaming text |
| `test_update_stream_info_empty_with_no_tools` | Only streaming text, no tools → stream_info stays empty |
| `test_update_stream_info_includes_tool_name` | Active tool → stream_info includes tool name |
| `test_streaming_text_not_duplicated_in_stream_info` | Exact scenario reproduction (`The Clockmaker of Lost Hours`) |

---

### Secondary Fixes in PR #73

| Issue | Fix |
|---|---|
| `chat.streaming` never cleared on turn error | Set `chat.streaming = false` in `success: false` branch (lib.rs:455-461) |
| Dead code: `ChatViewMode`, `ChatPanel::scroll`, `view_mode` | Removed entirely; `scroll_to_bottom` migrated to `MessageDisplayState` |
| `update_stream_info()` never called during draw | Called in `draw_ui()` after `set_turn_state_ref()` for live tool status updates |

---

### Testing

- **45/45 tests pass** (updated from 41 after adding 4 regression tests + 1 new test removed during cleanup)
- **0 errors, 0 warnings** on `cargo build --package oben-tui`
- Verified manually with interactive use — streaming text appears only in the inline rendering area, user messages are never overlaid
