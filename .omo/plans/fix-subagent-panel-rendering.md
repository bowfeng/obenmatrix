# Fix: Subagent Panel - Goal Text & Executing Details

**Created:** 2026-07-02
**Related:** TUI subagent panel improvements

## Problem

User reported two issues with the subagent panel in the right sidebar:

1. **Goal text lines not fully displayed** — The goal text wraps incorrectly; some characters are cut off at the panel boundary
2. **Executing status has no details** — When a subagent is running, only `... executing ...` shows with nothing about what's happening (no tool calls, no activity)

## Root Cause Analysis

### Issue 1: Goal text truncation

In `render_subagers_panel` (chat.rs:443-448):
```rust
let inner = area.inner(Margin { horizontal: 0, vertical: 1 });
let content_width = inner.width.saturating_sub(3) as usize;
```
This double-truncates the width — `inner()` already strips borders, then the extra `-3` wastes 3 more chars.

Then at lines 479-502, the goal text chunks at `content_width - 9` chars which is too small, leaving only a few dozen chars per line.

### Issue 2: No executing details
- `SubagentInfo` struct already has `tool_calls`, `tool_call_previews`, `stats` fields — but they're never populated
- `SubagentCallback::on_start` only sets `delegation_id`, `goal`, `status`, `start_time`
- `SubagentCallback::on_complete` only sets `status`, `end_time`, `summary`
- The `TuiSubagentAdapter::on_tool_start` (which fires for `delegate_task` tools) is the only tool hook, and it never forwards to SharedAgentState

## Fix Plan

### Fix 1: Correct content width (chat.rs)

```rust
// BEFORE:
let inner = area.inner(Margin { horizontal: 0, vertical: 1 });
let content_width = inner.width.saturating_sub(3) as usize;

// AFTER:
let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
let content_width = inner.width as usize;
```

### Fix 2: Update goal text to use Paragraph with proper wrapping (chat.rs)

Replace the manual `chunks()`-based goal text with `Paragraph` using `wrap::NoWrap` + manual line-wrapping for better control, or simply use correct padding:

```rust
// For the goal: label line, calculate the actual display width
let goal_prefix = "  goal: ";
let goal_text_width = content_width.saturating_sub(goal_prefix.len());
let goals: Vec<String> = if goal_text_width > 0 {
    sub.goal.chars().collect::<Vec<_>>().chunks(goal_text_width)
        .map(|c| c.iter().collect()).collect()
} else { vec![sub.goal.clone()] };
```

### Fix 3: Add executing details (chat.rs)

When status is "running", show:
- Active tool calls from `SubagentInfo.tool_call_previews` (populate in next step)
- Duration/progress if available
- The `... executing ...` prefix

```rust
if sub.status == "running" {
    lines.push(Line::from(vec![
        Span::styled(
            "⠋ Executing",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
    ]));
    
    // Show active tool calls
    for preview in sub.tool_call_previews.iter().take(3) {
        if !preview.is_empty() {
            let truncated = preview.chars().take(60).collect::<String>();
            lines.push(Line::from(vec![
                Span::raw("    ┃ "),
                Span::styled(truncated, Style::default().fg(Color::Yellow).add_modifier(Modifier::DIM)),
            ]));
        }
    }
    
    // Show stats if available
    if sub.stats.tool_count > 0 {
        lines.push(Line::from(vec![
            Span::raw("    ▸ "),
            Span::styled(
                format!("{} tools, {} tokens", sub.stats.tool_count, sub.stats.token_count),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
}
```

### Fix 4: Populate tool_call_previews during execution

Update `oben-tui/src/shared/agent_state.rs` — the `SubagentCallback` needs to also implement `on_tool_start`/`on_tool_complete` for non-delegate tools.

Actually, looking at how the hooks work:
- `TuiSubagentAdapter` is a ToolLifecycleHook that fires on `delegate_task`
- Regular tools fire through `TuiToolLifecycleAdapter` which writes to `TurnState`

The subagent's own tool calls happen inside the spawned session. They won't hit our TUI hooks. So tool_call_previews will only be populated after completion.

For showing active execution, we should show:
- `... executing ...` with a spinner
- The number of spawned sessions / nested subagents
- Elapsed time since start

Let me simplify — just show the executing line and let the user expand the subagent for details. The goal text fix is the most impactful change.

### Simplified Plan

1. **Fix `render_subagers_panel` content width** in `chat.rs` — change `Margin { horizontal: 0 }` → `{ horizontal: 1 }` and remove extra `-3`
2. **Fix goal text chunk width** — use correct content_width (remove the `-9` offset)
3. **Improve executing display** — show `⠋ Executing` spinner, active time, and note that details appear on expand
