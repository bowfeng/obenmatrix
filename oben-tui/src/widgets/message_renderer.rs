//! Message renderer — renders `oben_models::Message` into structured display data.

use pulldown_cmark::{Parser, Event, Tag, TagEnd, CodeBlockKind};
use ratatui::prelude::*;
use ratatui_themes::ThemeName;

use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageContent, MessagePart, MessageRole};

/// A text segment with associated styling.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub style: Style,
}

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
    pub title: Option<Line<'static>>,
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
    tracing::trace!("[render_md] INPUT text_len={} preview={}", text.len(), text.chars().take(200).collect::<String>());
    let parser = Parser::new(text);

    let body_style = Style::default().fg(palette.info);
    let code_style = Style::default().fg(palette.success).add_modifier(Modifier::DIM);
    let heading_style = Style::default().fg(palette.accent).add_modifier(Modifier::BOLD);

    let mut lines: Vec<StyledLine> = Vec::new();
    let mut in_code_block = false;
    
    // Style stack — tracks nested Strong/Emphasis
    let mut in_strong = false;
    let mut in_emphasis = false;

    // Segments — accumulated styled text
    let mut segments: Vec<Segment> = Vec::new();

    let mut pending_code_block_lang: Option<String> = None;
    let mut code_block_lines: Vec<String> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                tracing::trace!("[render_md] START CodeBlock lang=\"{}\"", lang.trim());
                pending_code_block_lang = Some(lang.trim().to_string());
                in_code_block = true;
                code_block_lines.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                in_code_block = true;
                code_block_lines.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(lang) = pending_code_block_lang {
                    let lang_str = lang.trim();
                    let label = if !lang_str.is_empty() && lang_str != "text" {
                        format!("[{}] ", lang_str.to_lowercase())
                    } else { String::new() };
                    if !label.is_empty() {
                        lines.push(StyledLine {
                            content: Line::from(Span::styled(label, Style::default().fg(palette.accent))),
                            role_color: None,
                        });
                    }
                }
                for code_line in &code_block_lines {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled(code_line.clone(), code_style)),
                        role_color: None,
                    });
                }
                code_block_lines.clear();
                pending_code_block_lang = None;
                in_code_block = false;
            }
            
            // ── Inline styling ──────────────
            Event::Start(Tag::Strong) => { in_strong = true; }
            Event::End(TagEnd::Strong) => { in_strong = false; }
            Event::Start(Tag::Emphasis) => { in_emphasis = true; }
            Event::End(TagEnd::Emphasis) => { in_emphasis = false; }

            // ── Headings ─────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                if !segments.is_empty() {
                    lines.push(flush_segments(&mut segments));
                }
                tracing::trace!("[render_md] START Heading level={:?}", level);
                lines.push(StyledLine {
                    content: Line::from(vec![
                        Span::styled("▸ ", Style::default().fg(palette.accent)),
                        Span::styled(segments_text(&segments), heading_style),
                    ]),
                    role_color: None,
                });
            }

            // ── Paragraphs ───────────────────────────────
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !segments.is_empty() {
                    lines.push(flush_segments(&mut segments));
                }
            }

            // ── Blockquotes ──────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                if !segments.is_empty() {
                    lines.push(flush_segments(&mut segments));
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {}

            // ── Lists ────────────────────────────────────
            Event::Start(Tag::Item) => {
                if !segments.is_empty() {
                    lines.push(StyledLine {
                        content: Line::from(vec![
                            Span::styled("• ", Style::default().fg(palette.info)),
                            Span::styled(segments_text(&segments), body_style),
                        ]),
                        role_color: None,
                    });
                    segments.clear();
                } else {
                    lines.push(StyledLine {
                        content: Line::from(Span::styled("• ", Style::default().fg(palette.info))),
                        role_color: None,
                    });
                }
            }
            Event::Start(Tag::List(_)) => {}
            Event::End(TagEnd::List(_)) => {}

            // ── Text content ─────────────────────────────
            Event::Text(text) => {
                if in_code_block {
                    code_block_lines.push(text.to_string());
                } else {
                    let mut style = body_style;
                    if in_strong { style = style.add_modifier(Modifier::BOLD); }
                    if in_emphasis { style = style.add_modifier(Modifier::ITALIC); }
                    segments.push(Segment { text: text.to_string(), style });
                }
            }

            // ── Inline code ─────────────────
            Event::Code(code) => {
                if in_code_block {
                    code_block_lines.push(code.to_string());
                } else {
                    segments.push(Segment { text: code.to_string(), style: code_style });
                }
            }

            // ── Breaks ───────────────────────────────────
            Event::SoftBreak => {
                if !in_code_block {
                    segments.push(Segment { text: " ".to_string(), style: body_style });
                }
            }
            Event::HardBreak => {
                if in_code_block {
                    code_block_lines.push("\n".to_string());
                } else if !segments.is_empty() {
                    lines.push(flush_segments(&mut segments));
                }
            }

            // ── Horizontal rule ──────────────────────────
            Event::Rule => {
                let rule = "\u{2500}".repeat(60);
                lines.push(StyledLine {
                    content: Line::from(Span::styled(rule, Style::default().fg(palette.info).add_modifier(Modifier::DIM))),
                    role_color: None,
                });
            }
            _ => {}
        }
    }

    // Flush remaining segments
    if !segments.is_empty() {
        lines.push(flush_segments(&mut segments));
    }

    for (i, line) in lines.iter().enumerate() {
        tracing::trace!("[render_md] line[{}] content=\"{}\"", i, line.content.spans.iter().map(|s| s.content.to_string()).collect::<String>());
    }
    tracing::trace!("[render_md] FINAL lines={}", lines.len());

    lines
}

fn segments_text(segments: &[Segment]) -> String {
    segments.iter().map(|s| s.text.as_str()).collect()
}

fn flush_segments(segments: &mut Vec<Segment>) -> StyledLine {
    let spans: Vec<Span<'static>> = segments.drain(..)
        .map(|s| Span::styled(s.text, s.style))
        .collect();
    StyledLine {
        content: Line::from(spans),
        role_color: None,
    }
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
    tracing::trace!(
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
        tracing::trace!("[render_md] text to parse len={} preview={}", text.len(), text.chars().take(200).collect::<String>());
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
        title: None,
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
            title: Some(Line::from(vec![
                Span::styled(
                    "  🤔 Thought",
                    Style::default()
                        .fg(self.current_palette().muted)
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                ),
            ])),
        };

        vec![reasoning_entry, main]
    }
}

impl Default for MessageRenderer {
    fn default() -> Self {
        Self::new()
    }
}
