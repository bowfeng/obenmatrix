# Plan: Add proper markdown rendering with pulldown-cmark

## Goal
Replace the custom tokenizer in `oben-tui/src/widgets/message_renderer.rs` with proper markdown rendering using pulldown-cmark.

## Implementation Steps

### Step 1: Add dependency to Cargo.toml
Add `pulldown-cmark = "0.12"` to `oben-tui/Cargo.toml` in the dependencies section, after `textwrap = "0.16"` line.

### Step 2: Replace message_renderer.rs markdown rendering

Replace `oben-tui/src/widgets/message_renderer.rs` to use pulldown-cmark:

1. **Add imports at top of file:**
```rust
use pulldown_cmark::{Parser, Event, Tag};
```

2. **Replace `render_body_lines()` function** with:
```rust
fn render_body_lines(text: &str, palette: &ratatui_themes::ThemePalette) -> Vec<StyledLine> {
    let parser = Parser::new(text);
    let mut lines: Vec<StyledLine> = Vec::new();
    let mut current_line = String::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();
    
    for event in parser {
        match event {
            Event::Text(t) => {
                current_line.push_str(&t.to_string());
            }
            Event::Code(t) => {
                // Inline code
                if !current_line.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            current_line.clone(),
                            Style::default().fg(palette.info).add_modifier(Modifier::DIM),
                        )),
                        role_color: None,
                    });
                }
                current_line = format!("`{}`", t);
            }
            Event::SoftBreak | Event::HardBreak => {
                current_line.push(' ');
            }
            Event::End(Tag::Paragraph) => {
                if !current_line.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            current_line.clone(),
                            Style::default().fg(palette.info),
                        )),
                        role_color: None,
                    });
                    current_line.clear();
                }
            }
            Event::Start(Tag::Paragraph) => {}
            Event::Start(Tag::Heading(level, _, _)) => {
                if !current_line.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            current_line.clone(),
                            Style::default().fg(palette.info),
                        )),
                        role_color: None,
                    });
                    current_line.clear();
                }
            }
            Event::Start(Tag::CodeBlock(c)) => {
                if let Some(lang) = match c {
                    pulldown_cmark::CodeBlockKind::Fenced(s) => Some(s.clone()),
                    pulldown_cmark::CodeBlockKind::Indented => None,
                } {
                    code_lang = lang;
                }
                in_code = true;
            }
            Event::End(Tag::CodeBlock(_)) => {
                let lang_label = if !code_lang.is_empty() && code_lang != "text" {
                    format!("[{}] ", code_lang.to_lowercase())
                } else {
                    String::new()
                };
                
                if !lang_label.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            lang_label,
                            Style::default().fg(palette.accent),
                        )),
                        role_color: None,
                    });
                }
                
                for line in &code_lines {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            line.clone(),
                            Style::default().fg(palette.success).add_modifier(Modifier::DIM),
                        )),
                        role_color: None,
                    });
                }
                code_lines.clear();
                in_code = false;
            }
            _ => {}
        }
    }
    
    lines
}
```

3. **Remove old functions:**
   - Remove `tokenize()` function
   - Remove `tokens_to_spans()` function
   - Remove `Token` enum

4. **Keep these unchanged:**
   - `StyledLine` struct
   - `MessageRenderEntry` struct
   - `render_message_entry()` function
   - `MessageRenderer` struct and methods

### Step 3: Verification
Run:
```bash
cd /Users/ellie/workspace/oben-alien
cargo check -p oben-tui
cargo test -p oben-tui
```

## Files Changed
- `oben-tui/Cargo.toml` - add pulldown-cmark dependency
- `oben-tui/src/widgets/message_renderer.rs` - replace markdown rendering

## Verification Criteria
1. cargo check passes without errors
2. cargo test passes all existing tests
3. Tables render properly with monospace alignment
4. Lists show bullet/number prefixes
5. Code blocks render dim + green with language labels
6. Headings show in bold + accent color
