# Fix: render_body_lines markdown rendering — rewrite following ironclaw pattern

## Problem
`render_body_lines` in `oben-tui/src/widgets/message_renderer.rs` fails to render markdown styles (bold, italic, inline code) because:
1. No style stack to track nested emphasis
2. No segments to accumulate styled text snippets
3. Inline code manually adds backticks instead of applying style
4. All text merged into single span, losing all style info

## Reference
ironclaw's `render_markdown` in `ironclaw/crates/ironclaw_tui/src/render.rs` (lines 138-343) uses:
- `MdContext.style_stack: Vec<Style>` — tracks current style scope
- `MdContext.segments: Vec<(String, Style)>` — accumulated styled text
- `MdContext.flush()` — emits styled lines from segments

## Fix Plan

### File: `oben-tui/src/widgets/message_renderer.rs`
Replace `render_body_lines` with a rewrite following ironclaw pattern:

1. **Add style stack** — `Vec<Style>` to track `Strong`/`Emphasis` nesting
2. **Add segments** — `Vec<(String, Style)>` to accumulate text with current style
3. **Handle Strong/Emphasis** — push/pop style to stack
4. **Handle Code** — push segment with code_style
5. **Handle HardBreak** — flush segment into line
6. **Handle Paragraph end** — flush segments into line

### Changes in `oben-tui/src/panels/chat.rs`
- Remove redundant `use crate::widgets::message_renderer::{MessageRenderEntry, MessageRenderer, StyledLine, render_body_lines}` import line (will be merged onto single line by rustfmt)

## Verification
- `cargo check -p oben-tui` passes
- `cargo build -p oben-tui` passes

## Given/When/Then
- **Given**: Assistant response contains markdown (`**bold**`, `*italic*`, `` `code` ``)
- **When**: Markdown is parsed by `render_body_lines`
- **Then**: Bold is rendered with `Modifier::BOLD`, italic with `Modifier::ITALIC`, code with `code_style`
