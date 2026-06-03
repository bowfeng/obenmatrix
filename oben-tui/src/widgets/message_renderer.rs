//! Message renderer — renders `oben_models::Message` into structured display data.
//!
//! Returns `MessageRenderEntry` which the conversation widget can render as
//! a bordered block (Assistant/User/System) or a compact box (Tool).

use ratatui::prelude::*;
use ratatui_themes::ThemeName;

use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageContent, MessageRole};

/// State machine token types produced by the inline markdown lexer.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    Plain(String),
    Code(String),
    Bold(String),
    Italic(String),
    FencedBlock(String, Vec<String>),
}

/// Split markdown text into tokens using a simple state machine.
fn tokenize(md: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = md.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut buf = String::new();

    while i < len {
        // Fenced code block
        if i + 2 < len && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            if !buf.is_empty() {
                tokens.push(Token::Plain(buf.clone()));
                buf.clear();
            }
            let mut code_lines = Vec::new();
            i += 3;
            let mut lang_buf = String::new();
            while i < len && chars[i] != '\n' {
                lang_buf.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            while i + 2 < len {
                if chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
                    i += 3;
                    break;
                }
                code_lines.push(chars[i].to_string());
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let language = lang_buf.trim().to_string();
            tokens.push(Token::FencedBlock(language, code_lines));
        }
        // Bold
        else if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !buf.is_empty() {
                tokens.push(Token::Plain(buf.clone()));
                buf.clear();
            }
            i += 2;
            let mut inner = String::new();
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            tokens.push(Token::Bold(inner));
        }
        // Inline code
        else if chars[i] == '`' {
            if !buf.is_empty() {
                tokens.push(Token::Plain(buf.clone()));
                buf.clear();
            }
            i += 1;
            let mut inner = String::new();
            while i < len && chars[i] != '`' {
                inner.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token::Code(inner));
        }
        // Italic
        else if chars[i] == '*' || chars[i] == '_' {
            let delim = chars[i];
            if i + 1 < len && chars[i + 1] != delim && chars[i + 1] != ' ' && chars[i + 1] != '\n' {
                if !buf.is_empty() {
                    tokens.push(Token::Plain(buf.clone()));
                    buf.clear();
                }
                i += 1;
                let mut inner = String::new();
                while i < len && chars[i] != delim {
                    inner.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    i += 1;
                }
                tokens.push(Token::Italic(inner));
            } else {
                buf.push(chars[i]);
                i += 1;
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }

    if !buf.is_empty() {
        tokens.push(Token::Plain(buf));
    }

    tokens
}

/// Flatten tokens into ratatui Spans with the given base style.
fn tokens_to_spans(
    tokens: &[Token],
    base: Style,
    palette: &ratatui_themes::ThemePalette,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for token in tokens {
        match token {
            Token::Plain(s) => {
                spans.push(Span::styled(s.clone(), base));
            }
            Token::Code(s) => {
                spans.push(Span::styled(
                    s.clone(),
                    base.bg(Color::DarkGray).add_modifier(Modifier::DIM),
                ));
            }
            Token::Bold(s) => {
                spans.push(Span::styled(s.clone(), base.add_modifier(Modifier::BOLD)));
            }
            Token::Italic(s) => {
                spans.push(Span::styled(s.clone(), base.add_modifier(Modifier::DIM)));
            }
            Token::FencedBlock(lang, lines) => {
                let lang_label = if !lang.is_empty() && lang != "text" {
                    format!("[{}]", lang.to_lowercase())
                } else {
                    String::new()
                };
                if !lang_label.is_empty() {
                    spans.push(Span::styled(
                        lang_label,
                        Style::default().fg(palette.accent),
                    ));
                }
                spans.extend(lines.iter().map(|l| {
                    Span::styled(
                        l.clone(),
                        Style::default()
                            .fg(palette.success)
                            .add_modifier(Modifier::DIM),
                    )
                }));
            }
        }
    }
    spans
}

/// A single renderable line with optional role color for the header.
#[derive(Debug, Clone)]
pub struct StyledLine {
    /// The visual line content.
    pub content: Line<'static>,
    /// Optional role-specific color for decoration.
    pub role_color: Option<Color>,
}

/// Structured render result for a single message.
/// The conversation widget uses this to render bordered blocks.
#[derive(Debug, Clone)]
pub struct MessageRenderEntry {
    /// The role of this message.
    pub role: MessageRole,
    /// Whether this is a tool result (compact style).
    pub is_tool_result: bool,
    /// Lines of body content (excluding header).
    pub body_lines: Vec<StyledLine>,
    /// Tool call info to render inline.
    pub tool_calls: Vec<String>,
}

impl MessageRenderEntry {
    /// Add a blank trailing line after the message.
    pub fn with_trailing_blank(mut self) -> Self {
        self.body_lines.push(StyledLine {
            content: Line::raw(""),
            role_color: None,
        });
        self
    }
}

/// Render message body text into styled lines.
fn render_body_lines(text: &str, palette: &ratatui_themes::ThemePalette) -> Vec<StyledLine> {
    let body_style = Style::default().fg(palette.info);
    let mut result = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim_start();

        // Blockquote
        if trimmed.starts_with('>') {
            let after = trimmed.strip_prefix('>').unwrap_or(trimmed).trim_start();
            let tokens = tokenize(after);
            let mut line_spans = vec![Span::styled(
                "┃ ",
                Style::default()
                    .fg(palette.info)
                    .add_modifier(Modifier::DIM),
            )];
            line_spans.extend(tokens_to_spans(
                &tokens,
                body_style.add_modifier(Modifier::DIM),
                palette,
            ));
            result.push(StyledLine {
                content: Line::from(line_spans),
                role_color: None,
            });
        }
        // Heading
        else if let Some(heading_content) = trimmed.strip_prefix('#').map(|s| s.trim_start()) {
            let heading_tokens = tokenize(heading_content);
            let mut line_spans = vec![Span::styled("▸ ", Style::default().fg(palette.accent))];
            line_spans.extend(tokens_to_spans(
                &heading_tokens,
                body_style.add_modifier(Modifier::BOLD).fg(palette.accent),
                palette,
            ));
            result.push(StyledLine {
                content: Line::from(line_spans),
                role_color: None,
            });
        } else {
            let tokens = tokenize(trimmed);
            result.push(StyledLine {
                content: Line::from(tokens_to_spans(&tokens, body_style, palette)),
                role_color: None,
            });
        }
    }

    result
}

/// Render a single message into a structured `MessageRenderEntry`.
pub fn render_message_entry(
    msg: &Message,
    palette: &ratatui_themes::ThemePalette,
) -> MessageRenderEntry {
    let info = role_info_for_role(&msg.role, palette);
    let color = info.border_color;
    let icon = info.icon;

    let text = match &msg.content {
        MessageContent::Text(t) => t.clone(),
        _ => String::new(),
    };

    let mut body_lines = if text.is_empty() {
        vec![StyledLine {
            content: Line::raw("(empty)"),
            role_color: None,
        }]
    } else {
        render_body_lines(&text, palette)
    };

    // Tool call indicators
    let mut tool_calls = Vec::new();
    if let Some(tcs) = &msg.tool_calls {
        for tc in tcs {
            let info = if tc.arguments.is_null() {
                tc.tool_name.clone()
            } else {
                format!(
                    "{}({})",
                    tc.tool_name,
                    tc.arguments
                        .to_string()
                        .chars()
                        .take(30)
                        .collect::<String>()
                )
            };
            tool_calls.push(info.clone());
        }
    }

    // Prepend tool call lines to body
    if !tool_calls.is_empty() {
        let mut tool_lines = Vec::new();
        for tc in &tool_calls {
            tool_lines.push(StyledLine {
                content: Line::from(Span::styled(
                    format!("   {} {}", icon, tc),
                    Style::default().fg(color).add_modifier(Modifier::DIM),
                )),
                role_color: None,
            });
        }
        tool_lines.extend(body_lines);
        body_lines = tool_lines;
    }

    MessageRenderEntry {
        role: msg.role.clone(),
        is_tool_result: msg.role == MessageRole::Tool,
        body_lines,
        tool_calls,
    }
    .with_trailing_blank()
}

/// Message renderer with runtime theme support and inline markdown rendering.
pub struct MessageRenderer {
    current_palette: ratatui_themes::ThemePalette,
    current_theme: ThemeName,
}

impl MessageRenderer {
    pub fn new() -> Self {
        let default_theme = ThemeName::default();
        Self {
            current_palette: default_theme.palette(),
            current_theme: default_theme,
        }
    }

    pub fn set_theme(&mut self, theme: ThemeName) {
        self.current_theme = theme;
        self.current_palette = self.current_theme.palette();
    }

    /// Set the theme from a string (e.g. "nord", "catppuccin-mocha").
    /// Falls back to Dracula on unknown names.
    pub fn set_theme_from_str(&mut self, s: &str) {
        if let Ok(name) = s.parse::<ThemeName>() {
            self.set_theme(name);
        }
    }

    pub fn current_theme(&self) -> ThemeName {
        self.current_theme
    }

    /// Return a reference to the current theme's color palette.
    ///
    /// This is the single source of truth for all semantic colors used
    /// across the TUI — borders, headers, body text, tool results, etc.
    pub fn current_palette(&self) -> &ratatui_themes::ThemePalette {
        &self.current_palette
    }

    /// Legacy API: render a message into flat `Line` slices (for backward compat).
    pub fn render(&self, msg: &Message) -> Vec<Line<'static>> {
        let entry = self.render_entry(msg);
        entry.body_lines.into_iter().map(|sl| sl.content).collect()
    }

    /// Render a message into a structured `MessageRenderEntry`.
    pub fn render_entry(&self, msg: &Message) -> MessageRenderEntry {
        render_message_entry(msg, self.current_palette())
    }
}

impl Default for MessageRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::Message;

    /// Given: plain markdown text with bold and italic
    /// When: tokenize is called
    /// Then: tokens correctly split into Plain, Bold, Italic segments
    #[test]
    fn test_tokenize_bold_italic() {
        let tokens = tokenize("Hello **bold** and *italic* world");
        assert_eq!(tokens.len(), 5);
        match &tokens[1] {
            Token::Bold(s) => assert_eq!(s, "bold"),
            _ => panic!("Expected Bold token"),
        }
        match &tokens[3] {
            Token::Italic(s) => assert_eq!(s, "italic"),
            _ => panic!("Expected Italic token"),
        }
    }

    /// Given: inline code snippet
    /// When: tokenize is called
    /// Then: Code token is produced
    #[test]
    fn test_tokenize_inline_code() {
        let tokens = tokenize("use `foo` bar");
        assert_eq!(tokens.len(), 3);
        match &tokens[1] {
            Token::Code(s) => assert_eq!(s, "foo"),
            _ => panic!("Expected Code token"),
        }
    }

    /// Given: a plain text message
    /// When: render_entry is called
    /// Then: output has correct role and non-empty body
    #[test]
    fn test_render_entry_has_role_and_body() {
        let renderer = MessageRenderer::new();
        let msg = Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text("Hello world".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        };
        let entry = renderer.render_entry(&msg);
        assert_eq!(entry.role, MessageRole::Assistant);
        assert!(!entry.body_lines.is_empty());
        assert!(!entry.is_tool_result);
    }

    /// Given: a tool result message
    /// When: render_entry is called
    /// Then: is_tool_result is true
    #[test]
    fn test_render_entry_tool_result() {
        let renderer = MessageRenderer::new();
        let msg = Message {
            role: MessageRole::Tool,
            content: MessageContent::Text("file read: 42 lines".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        };
        let entry = renderer.render_entry(&msg);
        assert!(entry.is_tool_result);
    }
}
