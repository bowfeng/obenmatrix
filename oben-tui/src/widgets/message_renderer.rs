//! Message renderer — renders `oben_models::Message` into `Line` slices.
//!
//! Supports inline markdown (**bold**, *italic*, `code`) and fenced code blocks.

use std::sync::Mutex;

use ratatui::prelude::*;
use ratatui_themes::ThemeName;

use crate::widgets::role_style::{get_style_for_role, ColorHint};
use oben_models::{Message, MessageContent};

#[derive(Debug, Clone, Copy)]
enum ColorField {
    Success,
    Info,
    Accent,
    Warning,
}

impl ColorField {
    fn to_color(self, palette: &ratatui_themes::ThemePalette) -> Color {
        match self {
            ColorField::Success => palette.success,
            ColorField::Info => palette.info,
            ColorField::Accent => palette.accent,
            ColorField::Warning => palette.warning,
        }
    }
}

impl ColorHint {
    fn to_field(self) -> ColorField {
        match self {
            ColorHint::Success => ColorField::Success,
            ColorHint::Info => ColorField::Info,
            ColorHint::Accent => ColorField::Accent,
            ColorHint::Warning => ColorField::Warning,
        }
    }
}

/// State machine token types produced by the inline markdown lexer.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// Plain text (no markdown inside)
    Text(String),
    /// Inline code `code`
    Code(String),
    /// Bold **text**
    Bold(String),
    /// Italic *text* or _text_
    Italic(String),
    /// Fenced code block ```lang ... ```
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
                tokens.push(Token::Text(buf.clone()));
                buf.clear();
            }
            // Collect until closing ```
            let mut code_lines = Vec::new();
            i += 3;
            let mut lang_buf = String::new();
            // Read language line
            while i < len && chars[i] != '\n' {
                lang_buf.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip \n
            }
            // Read code until closing ```
            while i + 2 < len {
                if chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
                    i += 3;
                    break;
                }
                code_lines.push(chars[i].to_string());
                // Keep newlines
                i += 1;
            }
            if i < len {
                i += 1; // skip any trailing char
            }
            let language = lang_buf.trim().to_string();
            tokens.push(Token::FencedBlock(language, code_lines));
        }
        // Bold
        else if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !buf.is_empty() {
                tokens.push(Token::Text(buf.clone()));
                buf.clear();
            }
            i += 2;
            let mut inner = String::new();
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing **
            }
            tokens.push(Token::Bold(inner));
        }
        // Inline code
        else if chars[i] == '`' {
            if !buf.is_empty() {
                tokens.push(Token::Text(buf.clone()));
                buf.clear();
            }
            i += 1;
            let mut inner = String::new();
            while i < len && chars[i] != '`' {
                inner.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing `
            }
            tokens.push(Token::Code(inner));
        }
        // Italic
        else if chars[i] == '*' || chars[i] == '_' {
            let delim = chars[i];
            if i + 1 < len
                && chars[i + 1] != delim
                && chars[i + 1] != ' '
                && chars[i + 1] != '\n'
            {
                if !buf.is_empty() {
                    tokens.push(Token::Text(buf.clone()));
                    buf.clear();
                }
                i += 1;
                let mut inner = String::new();
                while i < len && chars[i] != delim {
                    inner.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    i += 1; // skip closing
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
        tokens.push(Token::Text(buf));
    }

    tokens
}

/// Flatten tokens into ratatui Spans.
fn tokens_to_spans(tokens: &[Token], base: Style, palette: &ratatui_themes::ThemePalette) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for token in tokens {
        match token {
            Token::Text(s) => {
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
                spans.push(Span::styled(
                    s.clone(),
                    base.add_modifier(Modifier::DIM),
                ));
            }
            Token::FencedBlock(lang, lines) => {
                let lang_label = if !lang.is_empty() && lang != "text" {
                    format!(" {}", lang.to_lowercase())
                } else {
                    String::new()
                };
                if !lang_label.is_empty() {
                    spans.push(Span::styled(
                        format!("[{}]", lang_label),
                        Style::default().fg(palette.accent),
                    ));
                }
                spans.extend(lines.iter().map(|l| {
                    Span::styled(l.clone(), Style::default().fg(palette.success).add_modifier(Modifier::DIM))
                }));
            }
        }
    }
    spans
}

/// Message renderer with runtime theme support and inline markdown rendering.
pub struct MessageRenderer {
    current_palette: Mutex<ratatui_themes::ThemePalette>,
    current_theme: Mutex<ThemeName>,
}

impl MessageRenderer {
    pub fn new() -> Self {
        let default_theme = ThemeName::default();
        Self {
            current_palette: Mutex::new(default_theme.palette()),
            current_theme: Mutex::new(default_theme),
        }
    }

    pub fn set_theme(&self, theme: ThemeName) {
        *self.current_theme.lock().unwrap() = theme;
        *self.current_palette.lock().unwrap() = theme.palette();
    }

    pub fn current_theme(&self) -> ThemeName {
        *self.current_theme.lock().unwrap()
    }

    pub fn render(&self, msg: &Message) -> Vec<Line<'static>> {
        let palette = self.current_palette.lock().unwrap();
        let style = get_style_for_role(&msg.role);
        let color_field = style.color_hint().to_field();

        let mut lines = Vec::new();

        // Header bar
        let header = format!("── {} {} ──", style.icon(), style.label());
        lines.push(Line::from(Span::styled(
            header,
            Style::default()
                .fg(color_field.to_color(&palette))
                .add_modifier(Modifier::BOLD),
        )));

        // Markdown body
        let text = match &msg.content {
            MessageContent::Text(t) => t.clone(),
            _ => String::new(),
        };

        if !text.is_empty() {
            let base_style = Style::default().fg(palette.info);

            // Tokenize line-by-line (preserves code blocks as single tokens)
            for raw_line in text.lines() {
                let trimmed = raw_line.trim_start();
                let is_blockquote = trimmed.starts_with('>');

                let line_style = if is_blockquote {
                    base_style.add_modifier(Modifier::DIM)
                } else {
                    base_style
                };

                if is_blockquote {
                    let after = trimmed.strip_prefix('>').unwrap_or(trimmed).trim_start();
                    let tokens = tokenize(after);
                    let mut result = vec![Span::styled(
                        "┃ ",
                        Style::default().fg(palette.info).add_modifier(Modifier::DIM),
                    )];
                    result.extend(tokens_to_spans(&tokens, line_style, &palette));
                    lines.push(Line::from(result));
                } else {
                    let tokens = tokenize(trimmed);
                    // Heading detection (simple # prefix)
                    if let Some(heading_content) = trimmed.strip_prefix('#').map(|s| s.trim_start()) {
                        let tokens2 = tokenize(heading_content);
                        let mut result = vec![Span::styled("▸ ", Style::default().fg(palette.accent))];
                        result.extend(tokens_to_spans(&tokens2, base_style.add_modifier(Modifier::BOLD).fg(palette.accent), &palette));
                        lines.push(Line::from(result));
                    } else {
                        let spans = tokens_to_spans(&tokens, line_style, &palette);
                        lines.push(Line::from(spans));
                    }
                }
            }
        }

        // Tool call indicators
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                let tool_info = if tc.arguments.is_null() {
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
                lines.push(Line::from(format!("   {}", tool_info)));
            }
        }

        lines.push(Line::from(""));
        lines
    }
}

impl Default for MessageRenderer {
    fn default() -> Self {
        Self::new()
    }
}
