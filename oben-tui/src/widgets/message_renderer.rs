//! Message renderer — renders `oben_models::Message` into structured display data.

use pulldown_cmark::{Parser, Event, Tag, TagEnd, CodeBlockKind, HeadingLevel};
use ratatui::prelude::*;
use ratatui_themes::ThemeName;

use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageContent, MessagePart, MessageRole};

/// A single renderable line with optional role color for the header.
#[derive(Debug, Clone)]
pub struct StyledLine {
    pub content: Line<'static>,
    pub role_color: Option<Color>,
}

/// Structured render result for a single message.
#[derive(Debug, Clone)]
pub struct MessageRenderEntry {
    pub role: MessageRole,
    pub is_tool_result: bool,
    pub body_lines: Vec<StyledLine>,
    pub tool_calls: Vec<String>,
    pub reasoning: Option<String>,
}

impl MessageRenderEntry {
    pub fn with_trailing_blank(mut self) -> Self {
        self.body_lines.push(StyledLine {
            content: Line::raw(""),
            role_color: None,
        });
        self
    }
}

/// Render markdown text into styled lines using pulldown-cmark.
pub fn render_body_lines(text: &str, palette: &ratatui_themes::ThemePalette) -> Vec<StyledLine> {
    let parser = Parser::new(text);
    let mut lines: Vec<StyledLine> = Vec::new();
    let mut current_line_text = String::new();
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut code_lang = String::new();
    let mut in_code_block = false;

    let body_style = Style::default().fg(palette.info);
    let code_style = Style::default().fg(palette.success).add_modifier(Modifier::DIM);
    let heading_style = Style::default().fg(palette.accent).add_modifier(Modifier::BOLD);

    for event in parser {
        match event {
            Event::Text(text) => {
                if in_code_block {
                    if !text.is_empty() {
                        code_block_lines.push(text.to_string());
                    }
                } else {
                    current_line_text.push_str(&text.to_string());
                }
            }
            Event::Code(code) => {
                if in_code_block {
                    code_block_lines.push(code.to_string());
                } else {
                    current_line_text.push('`');
                    current_line_text.push_str(&code);
                    current_line_text.push('`');
                }
            }
            Event::SoftBreak => {
                current_line_text.push(' ');
            }
            Event::HardBreak => {
                current_line_text.push(' ');
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !current_line_text.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            current_line_text.clone(),
                            body_style,
                        )),
                        role_color: None,
                    });
                    current_line_text.clear();
                }
            }
            Event::Start(Tag::Heading { level: HeadingLevel::H1, .. }) => {
                lines.push(StyledLine {
                    content: Line::from(vec![
                        Span::styled("▸ ", Style::default().fg(palette.accent)),
                        Span::styled(current_line_text.clone(), heading_style),
                    ]),
                    role_color: None,
                });
                current_line_text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang_str = code_lang.trim().to_string();
                let lang_label = if !lang_str.is_empty() && lang_str != "text" {
                    format!("[{}] ", lang_str.to_lowercase())
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

                for code_line in &code_block_lines {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            code_line.clone(),
                            code_style,
                        )),
                        role_color: None,
                    });
                }
                code_block_lines.clear();
                in_code_block = false;
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                code_lang = lang.trim().to_string();
                in_code_block = true;
                code_block_lines.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                in_code_block = true;
                code_block_lines.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                if !current_line_text.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(vec![
                            Span::styled("┃ ", Style::default().fg(palette.info).add_modifier(Modifier::DIM)),
                            Span::styled(current_line_text.clone(), body_style.add_modifier(Modifier::DIM)),
                        ]),
                        role_color: None,
                    });
                    current_line_text.clear();
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {}
            Event::Start(Tag::Item) => {
                lines.push(StyledLine {
                    content: Line::from(vec![
                        Span::styled("• ", Style::default().fg(palette.info)),
                        Span::styled(current_line_text.clone(), body_style),
                    ]),
                    role_color: None,
                });
                current_line_text.clear();
            }
            Event::Start(Tag::List(_)) => {}
            Event::End(TagEnd::List(_)) => {}
            Event::Start(Tag::Table(_)) => {
                if !current_line_text.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(
                            current_line_text.clone(),
                            body_style,
                        )),
                        role_color: None,
                    });
                    current_line_text.clear();
                }
            }
            _ => {}
        }
    }

    if !current_line_text.is_empty() {
        lines.push(StyledLine {
            content: Line::from(Span::styled(
                current_line_text,
                body_style,
            )),
            role_color: None,
        });
    }

    lines
}

/// Check if a URL is a base64 data URL (starts with "data:").
fn is_data_url(url: &str) -> bool {
    url.starts_with("data:")
}

/// Generate a display placeholder for an image URL.
fn image_placeholder(url: &str, detail: Option<&str>) -> String {
    if is_data_url(url) {
        let mime_hint = url
            .split_once(':')
            .and_then(|(_, rest)| rest.split_once(';'))
            .map(|(first, _)| first.trim())
            .unwrap_or("");
        let detail_hint = detail.unwrap_or("");
        let mime_or_detail = if !mime_hint.is_empty() {
            mime_hint
        } else if !detail_hint.is_empty() {
            detail_hint
        } else {
            "image"
        };
        format!("\u{1F3F7}\u{FE0F} {}", mime_or_detail)
    } else {
        if let Some(d) = detail {
            format!("\u{1F3F7}\u{FE0F} {} <{}>", d, url)
        } else {
            format!("\u{1F3F7}\u{FE0F} <{}>", url)
        }
    }
}

/// Render a single message into a structured `MessageRenderEntry`.
pub fn render_message_entry(
    msg: &Message,
    palette: &ratatui_themes::ThemePalette,
) -> MessageRenderEntry {
    let info = role_info_for_role(&msg.role, palette);
    let color = info.border_color;
    let icon = info.icon;

    let raw_content: String = match &msg.content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Image { url, .. } => format!("[image: {}]", url.chars().take(40).collect::<String>()),
        MessageContent::Parts(parts) => parts.iter()
            .map(|p| match p {
                MessagePart::Text(t) => t.clone(),
                MessagePart::Image { url, .. } => format!("[image: {}]", url.chars().take(40).collect::<String>()),
            })
            .collect::<Vec<_>>()
            .join(" "),
    };
    tracing::info!(
        "[render_entry] role={:?} raw_content_len={} preview={}",
        msg.role, raw_content.len(),
        raw_content.chars().take(120).collect::<String>()
    );

    let mut has_images = false;
    let mut combined_parts: Vec<String> = Vec::new();

    match &msg.content {
        MessageContent::Text(t) => {
            if !t.is_empty() {
                combined_parts.push(t.clone());
            }
        }
        MessageContent::Image { url, detail } => {
            has_images = true;
            combined_parts.push(image_placeholder(url, detail.as_deref()));
        }
        MessageContent::Parts(parts) => {
            for part in parts {
                match part {
                    MessagePart::Text(t) => {
                        if !t.is_empty() {
                            combined_parts.push(t.clone());
                        }
                    }
                    MessagePart::Image { url, detail } => {
                        has_images = true;
                        combined_parts.push(image_placeholder(url, detail.as_deref()));
                    }
                }
            }
        }
    }

    let text = if combined_parts.is_empty() {
        String::new()
    } else {
        if has_images && combined_parts.len() > 1 {
            combined_parts.join(" ")
        } else {
            combined_parts.join("")
        }
    };

    let mut body_lines = if text.is_empty() {
        vec![]
    } else {
        render_body_lines(&text, palette)
    };

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
        reasoning: msg.reasoning.clone(),
    }
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

    pub fn set_theme_from_str(&mut self, s: &str) {
        if let Ok(name) = s.parse::<ThemeName>() {
            self.set_theme(name);
        }
    }

    pub fn current_theme(&self) -> ThemeName {
        self.current_theme
    }

    pub fn current_palette(&self) -> &ratatui_themes::ThemePalette {
        &self.current_palette
    }

    pub fn render(&self, msg: &Message) -> Vec<Line<'static>> {
        let entry = self.render_entry(msg);
        entry.body_lines.into_iter().map(|sl| sl.content).collect()
    }

    pub fn render_entry(&self, msg: &Message) -> MessageRenderEntry {
        render_message_entry(msg, self.current_palette())
    }

    pub fn render_entries(&self, msg: &Message) -> Vec<MessageRenderEntry> {
        let main = render_message_entry(msg, self.current_palette());
        let Some(refining) = main.reasoning.clone() else { return vec![main]; };
        if refining.is_empty() { return vec![main]; }

        let reasoning_color = if msg.role == MessageRole::Assistant {
            self.current_palette().muted
        } else {
            self.current_palette().info
        };

        let reasoning_lines: Vec<StyledLine> = refining
            .lines()
            .map(|line| StyledLine {
                content: Line::styled(
                    line.to_string(),
                    Style::default()
                        .fg(reasoning_color)
                        .add_modifier(Modifier::DIM),
                ),
                role_color: None,
            })
            .collect();

        let reasoning_entry = MessageRenderEntry {
            role: msg.role.clone(),
            is_tool_result: false,
            body_lines: reasoning_lines,
            tool_calls: vec![],
            reasoning: Some(refining),
        };

        vec![reasoning_entry, main]
    }
}

impl Default for MessageRenderer {
    fn default() -> Self {
        Self::new()
    }
}
