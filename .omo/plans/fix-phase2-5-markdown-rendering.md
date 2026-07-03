# Markdown Rendering Fix for Phase 2.5 Streaming

## Problem
Phase 2.5 流式渲染路径（`conversation.rs:692-705`）直接 `raw.lines()` 按行拆分文本，完全绕过了 `render_body_lines()` 的 pulldown-cmark markdown 处理。因此用户看到的 assistant 回复中的 markdown 符号（`###`、`-`、`|` 等）以原始文本形式显示。

## Root Cause
- `render_body_lines()` 在 `message_renderer.rs:38` 存在并正确使用 pulldown-cmark
- `render_message_entry()` 在 line 303 正确调用 `render_body_lines`
- 但 Phase 2.5 流式渲染路径（conversation.rs:692-705）绕过这两个函数，直接：
  ```rust
  let mut stream_lines: Vec<Line<'static>> = raw
      .lines()  // ← 直接按行拆分，跳过 markdown 解析
      .map(|l| Line::from(Span::styled(l.to_string(), ...)))
      .collect::<Vec<_>>();
  ```

## Solution
修改 `conversation.rs` 的 Phase 1.5 流式文本处理路径：

### 1. 修改 imports（line 17）
```rust
use crate::widgets::message_renderer::{render_body_lines, MessageRenderEntry, MessageRenderer};
```

### 2. 替换 Phase 1.5 流式渲染逻辑（conversation.rs:692-705）
将 raw.lines() 改为调用 `render_body_lines()`:

```rust
// Before (lines 692-705):
let raw = ts_ref
    .streaming_text
    .trim_start_matches(|c: char| c.is_whitespace());
let mut stream_lines: Vec<Line<'static>> = raw
    .lines()
    .map(|l| {
        Line::from(Span::styled(
            l.to_string(),
            Style::default()
                .fg(palette.info)
                .add_modifier(Modifier::DIM),
        ))
    })
    .collect::<Vec<_>>();

// After:
let raw = ts_ref
    .streaming_text
    .trim_start_matches(|c: char| c.is_whitespace());
let body_lines = render_body_lines(raw, palette);
let stream_lines: Vec<Line<'static>> = body_lines
    .into_iter()
    .map(|sl| sl.content)
    .collect();
```

### 3. 同样处理 reasoning_text（lines 708-723）
reasoning_text 也应该走 markdown 处理：
```rust
if !ts_ref.reasoning_text.is_empty() {
    let reasoning_lines = render_body_lines(&ts_ref.reasoning_text, palette);
    let reasoning_lines: Vec<Line<'static>> = reasoning_lines
        .into_iter()
        .map(|line| Line::styled(
            line.content.to_string(),
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::DIM),
        ))
        .collect();
    stream_lines.splice(0..0, reasoning_lines);
}
```

## Verification
- `cargo check -p oben-tui` 通过
- `cargo build -p oben-tui` 通过
- 重启 TUI，assistant 回复中的 markdown 格式正常渲染（`###` → 标题, `-` → 列表, `|` → 表格）
