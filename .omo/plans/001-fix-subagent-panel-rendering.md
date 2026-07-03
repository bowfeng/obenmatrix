# Fix: Subagent Panel Rendering Issues

**Created:** 2026-07-02
**Goal:** Fix two rendering bugs in the subagent panel (right sidebar)

## Bugs

1. **`goal:` text lines not fully displayed** — Goal text overflows panel boundary instead of wrapping
2. **Executing status shows no details** — Running subagents only show `executing` with no activity info

## Files Affected

- `oben-tui/src/panels/chat.rs` — `render_subagers_panel()` (line ~425)
- `oben-tui/src/shared/agent_state.rs` — `SubagentInfo` struct already has fields; just need to wire them in rendering

### SubagentInfo fields available (already defined):

```rust
pub struct SubagentInfo {
    pub delegation_id: u32,
    pub goal: String,
    pub status: String,       // "idle", "running", "complete", "error"
    pub tool_calls: Vec<String>,
    pub tool_call_previews: Vec<String>,
    pub stats: SubagentStats, // tool_count, token_count, duration
    pub children: Vec<SubagentInfo>,
    pub summary: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}
```

## Fix 1: Goal text wrapping

**Problem:** `render_subagers_panel()` uses `Line::from()` with `Span::styled()` for goal, but `content_width` calculation wastes space. Uses `Margin { horizontal: 0 }` on an already-bordered inner area.

**Fix:** Change `render_subagers_panel()` to:
1. Use `Paragraph` widget with automatic word/character wrapping instead of manual `lines.push()`
2. Or fix the width calculation to properly account for inner content bounds
3. Add padding: `Margin { horizontal: 1, vertical: 1 }` instead of `0`

Target code at lines ~425-576. Specifically fix the area calculation and text wrapping.

## Fix 2: Executing details

**Problem:** When `status == "running"`, the subagent only shows `executing` with no details about what it's doing. Even expanded state doesn't show tools/steps.

**Fix:** In `render_subagers_panel()` during the "running" status rendering (around line 504-512), show:
1. Tool call previews if available: `sub.tool_call_previews`
2. Activity: `sub.tool_calls` count
3. Stats if available: `sub.stats` (tool_count, duration)
4. Current running status with more context

Target: Add sub-detail rendering for running subagents.

## Implementation Steps

1. Open `oben-tui/src/panels/chat.rs`
2. Find `fn render_subagers_panel(&self, frame: &mut Frame, area: Rect, subagers: &[SubagentInfo])`
3. Fix `content_width` calculation (change `Margin { horizontal: 0 }` to `{ horizontal: 1 }`)
4. Fix goal text wrapping to not overflow (use `Paragraph` with max width or fix chunking)
5. Add executing details: tool_call_previews, stats, current activity
6. Run `cargo check -p oben-tui`
7. Run `cargo test -p oben-tui --lib`
