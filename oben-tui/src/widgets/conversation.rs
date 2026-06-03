//! Message display widget — renders message history with scrolling.
//!
//! Encapsulates the pre-rendered message lines, scrolling, streaming text
//! overlay, and turn-status text. Messages are rendered as bordered blocks
//! (like the Hermes Agent reference), with each message wrapped in a rounded-border
//! Block whose title shows the role label and icon.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use textwrap::wrap as textwrap_wrap;
use unicode_width::UnicodeWidthStr;

use crate::turn::turn_state::TurnState;
use crate::widgets::message_renderer::{MessageRenderEntry, MessageRenderer};
use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageRole};

/// Block rendering strategy for a message entry.
enum BlockType<'a> {
    /// Regular message: full border + role title + role-specific color.
    Message(&'a MessageRole),
    /// Tool result: indented muted rounded box (no role title).
    ToolResult,
}

/// State for the message display widget.
pub struct ConversationState {
    pub scroll_state: Arc<StdMutex<ScrollbarState>>,
    pub scroll_to_bottom: bool,
    /// Persisted scroll position across frames (AtomicUsize so render(&self) can update it).
    pub scroll_pos: Arc<AtomicUsize>,
    pub stream_info: Arc<StdMutex<String>>,
    pub turn_state_ref: Option<Arc<StdMutex<TurnState>>>,
    /// Accumulated scroll delta from user mouse scroll. Reset by render.
    pub user_scroll_offset: Arc<AtomicI32>,
    /// Selection start/end as (visual_line_idx, char_offset).
    pub selection_start: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
    /// Per-message structured entries for bordered-block rendering.
    pub message_entries: Arc<StdMutex<Vec<MessageRenderEntry>>>,
}

impl ConversationState {
    pub fn new() -> Self {
        Self {
            scroll_state: Arc::new(StdMutex::new(ScrollbarState::new(0))),
            scroll_to_bottom: true,
            scroll_pos: Arc::new(AtomicUsize::new(0)),
            stream_info: Arc::new(StdMutex::new(String::new())),
            turn_state_ref: None,
            user_scroll_offset: Arc::new(AtomicI32::new(0)),
            selection_start: None,
            selection_end: None,
            message_entries: Arc::new(StdMutex::new(Vec::new()))
        }
    }

    /// Clear selection state.
    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }
}

/// Widget that renders message entries with bordered blocks.
pub struct ConversationWidget;

impl ConversationWidget {
    /// Determine the border/title style for a given role.
    fn role_border_style(
        &self,
        role: &MessageRole,
        palette: &ratatui_themes::ThemePalette,
    ) -> Style {
        let info = role_info_for_role(role, palette);
        Style::default().fg(info.border_color)
    }

    fn role_title(&self, role: &MessageRole, palette: &ratatui_themes::ThemePalette) -> Line<'static> {
        let role_info = role_info_for_role(role, palette);
        Line::from(vec![
            Span::styled(
                format!(" {} ", role_info.icon),
                Style::default()
                    .fg(role_info.header_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                role_info.label.to_string(),
                Style::default()
                    .fg(role_info.header_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    }

    /// Wrap styled lines into wrapped lines for a given inner width.
    fn wrap_styled_lines_to_lines(
        &self,
        lines: &[Line<'static>],
        inner_width: usize,
    ) -> Vec<Line<'static>> {
        let mut result = Vec::new();
        for line in lines {
            let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
            if total_width <= inner_width {
                result.push(line.clone());
            } else if line.spans.len() == 1 {
                let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                result.extend(
                    textwrap_wrap(&text, inner_width)
                        .into_iter()
                        .map(|wrapped| {
                            Line::from(Span::styled(wrapped.into_owned(), line.spans[0].style))
                        }),
                );
            } else {
                // For lines with multiple styled spans, we wrap the plain text
                // and apply the first span's style to each wrapped line.
                let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                let first_style = line.spans.first().map(|s| s.style).unwrap_or_default();
                result.extend(
                    textwrap_wrap(&text, inner_width)
                        .into_iter()
                        .map(|wrapped| Line::from(Span::styled(wrapped.into_owned(), first_style))),
                );
            }
        }
        result
    }

    /// Build visual lines from styled lines for selection and legacy rendering.
    fn build_visual_lines(&self, lines: &[Line<'static>], inner_width: usize) -> Vec<Line<'static>> {
        lines
            .iter()
            .flat_map(|line| {
                let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
                if total_width <= inner_width {
                    vec![line.clone()]
                } else if line.spans.len() == 1 {
                    let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                    textwrap_wrap(&text, inner_width)
                        .into_iter()
                        .map(|wrapped| {
                            Line::from(Span::styled(wrapped.into_owned(), line.spans[0].style))
                        })
                        .collect::<Vec<_>>()
                } else {
                    let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                    let first_style = line.spans.first().map(|s| s.style).unwrap_or_default();
                    textwrap_wrap(&text, inner_width)
                        .into_iter()
                        .map(|wrapped| {
                            Line::from(Span::styled(wrapped.into_owned(), first_style))
                        })
                        .collect()
                }
            })
            .collect()
    }

    /// Get the selected text from the current state, if any.
    pub fn get_selected_text(&self, state: &mut ConversationState) -> Option<String> {
        let sy = state.selection_start.map(|(s, _)| s);
        let sx = state.selection_start.map(|(_, s)| s);
        let ey = state.selection_end.map(|(s, _)| s);
        let ex = state.selection_end.map(|(_, s)| s);

        if let (Some(sy), Some(sx), Some(ey), Some(ex)) = (sy, sx, ey, ex) {
            // Build flat lines from entries
            let entries = state.message_entries.lock().unwrap();
            let mut lines: Vec<Line<'static>> = Vec::new();
            for entry in entries.iter() {
                for sl in entry.body_lines.iter() {
                    lines.push(sl.content.clone());
                }
            }

            let vy_start = std::cmp::min(sy, ey);
            let vy_end = std::cmp::max(sy, ey);
            let mut result = String::new();

            for v in vy_start..=vy_end {
                if v >= lines.len() {
                    break;
                }
                let line = &lines[v];
                let line_text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                let chars: Vec<char> = line_text.chars().collect();

                if vy_start == vy_end {
                    let (x_start, x_end) = (std::cmp::min(sx, ex), std::cmp::max(sx, ex));
                    if x_start < chars.len() {
                        let sel: String = chars[x_start..std::cmp::min(x_end, chars.len())]
                            .iter()
                            .collect();
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&sel);
                    }
                } else if v == vy_start {
                    let start_x = std::cmp::min(sx, chars.len());
                    let sel: String = chars[start_x..].iter().collect();
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&sel);
                } else if v == vy_end {
                    let end_x = std::cmp::min(ex, chars.len());
                    let sel: String = chars[..end_x].iter().collect();
                    result.push('\n');
                    result.push_str(&sel);
                } else {
                    result.push('\n');
                    result.push_str(&line_text);
                }
            }

            if !result.is_empty() {
                return Some(result);
            }
        }
        None
    }

    /// Render message blocks with bordered styling.
    fn render_bordered_blocks(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
        entries: &[MessageRenderEntry],
    ) {
        // Outer border block (like "Messages" area)
        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.muted))
            .title(Line::from(vec![
                Span::styled(" ", Style::default().fg(palette.info)),
                Span::styled(
                    "Messages",
                    Style::default()
                        .fg(palette.info)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", Style::default().fg(palette.info)),
            ]));
        let msg_area = outer_block.inner(area);
        outer_block.render(area, frame.buffer_mut());

        // Calculate available space for messages (leave room for scrollbar)
        let inner_width = (msg_area.width as usize).saturating_sub(2);
        let inner_height = (msg_area.height as usize).saturating_sub(1);

        // Compute layout info for each entry: regular blocks need full border + title,
        // tool results get an indented muted box (like hermes-agent reference).
        let mut msg_wrapped_lines: Vec<Vec<Line<'static>>> = Vec::new();
        let mut msg_block_heights: Vec<u16> = Vec::new();
        let mut msg_block_type: Vec<BlockType<'_>> = Vec::new();

        for entry in entries {
            // Extract plain Line content from StyledLine entries for wrapping
            let plain_lines: Vec<Line<'static>> =
                entry.body_lines.iter().map(|sl| sl.content.clone()).collect();

            let wrapped = self.wrap_styled_lines_to_lines(
                &plain_lines,
                inner_width.saturating_sub(2), // -2 for block borders
            );

            if entry.is_tool_result {
                // Tool results: indented muted box (2 cols indent + 2 for border)
                let inner_width_tool = inner_width.saturating_sub(4); // 2 indent + 2 border
                let wrapped_tool = self.wrap_styled_lines_to_lines(
                    &plain_lines,
                    inner_width_tool.saturating_sub(2), // -2 for inner border
                );
                let block_height = wrapped_tool.len() as u16 + 2; // +2 for top+bottom border
                msg_wrapped_lines.push(wrapped_tool);
                msg_block_heights.push(block_height);
                msg_block_type.push(BlockType::ToolResult);
            } else {
                let block_height = wrapped.len() as u16 + 2; // +2 for top+bottom border
                msg_wrapped_lines.push(wrapped);
                msg_block_heights.push(block_height);
                msg_block_type.push(BlockType::Message(&entry.role));
            }
        }

        let total_height: u16 = msg_block_heights.iter().sum::<u16>()
            + entries.len().saturating_sub(1) as u16; // 1-row margin between messages

        // Calculate scrollable range in PIXEL HEIGHT (standard scroll: total - viewport)
        let scrollable_range: usize =
            (total_height as i64 - inner_height as i64).max(0) as usize;
        let is_scrollable = total_height > inner_height as u16;

        // Determine scroll offset (height-based, not block-count-based)
        let scroll_offset: usize = if state.scroll_to_bottom {
            // At bottom: offset = total_height - inner_height (show last N rows)
            // Clamp to total_height - min_block_height to prevent the skip loop
            // from skipping ALL blocks (which happens when scroll_offset >= total_height).
            let max_offset: usize = total_height
                .saturating_sub(msg_block_heights.iter().min().copied().unwrap_or(1)) as usize;
            scrollable_range.min(max_offset)
        } else {
            // Manual scroll: read current position
            state.scroll_pos.load(Ordering::SeqCst)
        };

        tracing::info!(
            "[render_bordered_blocks] entries={} inner_height={} total_height={} \
             scrollable_range={} scroll_offset={} scroll_to_bottom={} is_scrollable={}",
            entries.len(), inner_height, total_height,
            scrollable_range, scroll_offset, state.scroll_to_bottom, is_scrollable
        );

        // Update scrollbar state
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(scroll_offset);
        }

        // Render each message block, skipping those above the scroll offset
        let mut y = msg_area.top();
        let mut accumulated_height: u16 = 0;

        for (idx, &block_height) in msg_block_heights.iter().enumerate() {
            let block_total_height = block_height + 1; // +1 for margin

            // Skip blocks whose accumulated height is below the scroll offset
            if accumulated_height + block_total_height <= scroll_offset as u16 {
                accumulated_height += block_total_height;
                continue;
            }

            // Skip if block is below viewport
            if y >= msg_area.bottom() {
                break;
            }

            // Clamp block height to viewport
            let available_height = (msg_area.bottom() - y).min(block_height);
            let actual_height = available_height.max(1);
            let wrapped = &msg_wrapped_lines[idx];
            let block_type = &msg_block_type[idx];

            match block_type {
                BlockType::Message(role) => {
                    // Regular message: full border + role title + role-specific color
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(self.role_border_style(role, palette))
                        .title(self.role_title(role, palette));

                    let block_area =
                        Rect::new(msg_area.left(), y, msg_area.width, actual_height);

                    // Calculate body area BEFORE rendering (block is moved)
                    let body_area = block.inner(block_area);

                    // Render body lines inside the block first (background)
                    if actual_height > 2 && !wrapped.is_empty() {
                        let body_lines: Vec<Line> = wrapped
                            .iter()
                            .take(body_area.height as usize)
                            .cloned()
                            .collect();
                        if !body_lines.is_empty() {
                            let para = Paragraph::new(body_lines);
                            frame.render_widget(para, body_area);
                        }
                    }

                    // Render block borders/title on top
                    frame.render_widget(block, block_area);
                }
                BlockType::ToolResult => {
                    // Tool result: indented muted box (no role title, muted border)
                    let indent = 2; // 2 cols indent from left edge
                    let box_width =
                        (msg_area.width as usize).saturating_sub(indent * 2) as u16;
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(palette.muted))
                        .border_type(BorderType::Rounded);

                    let block_area = Rect::new(
                        msg_area.left() + indent as u16,
                        y,
                        box_width.min(
                            msg_area.width.saturating_sub((indent * 2) as u16),
                        ),
                        actual_height,
                    );

                    // Calculate body area BEFORE rendering (block is moved)
                    let body_area = block.inner(block_area);

                    // Render body lines with muted style inside the block
                    if actual_height > 2 && !wrapped.is_empty() {
                        let body_lines: Vec<Line> = wrapped
                            .iter()
                            .take(body_area.height as usize)
                            .cloned()
                            .map(|line| {
                                let muted_spans: Vec<Span> = line
                                    .spans
                                    .iter()
                                    .filter_map(|span| {
                                        // Preserve foreground color but apply muted dim modifier
                                        span.style
                                            .fg(palette.muted)
                                            .add_modifier(Modifier::DIM)
                                            .fg
                                            .map(|fg| {
                                                Span::styled(
                                                    span.content.clone(),
                                                    Style::default()
                                                        .fg(fg)
                                                        .add_modifier(Modifier::DIM),
                                                )
                                            })
                                            .or_else(|| {
                                                // If span had no explicit fg, use muted
                                                Some(Span::styled(
                                                    span.content.clone(),
                                                    Style::default()
                                                        .fg(palette.muted)
                                                        .add_modifier(Modifier::DIM),
                                                ))
                                            })
                                    })
                                    .collect();
                                Line::from(muted_spans)
                            })
                            .collect();
                        if !body_lines.is_empty() {
                            let para = Paragraph::new(body_lines);
                            frame.render_widget(para, body_area);
                        }
                    }

                    // Render block borders on top
                    frame.render_widget(block, block_area);
                }
            }

            // Advance to next message (add 1-row margin)
            y += actual_height + 1;
        }

        // If streaming, render streaming text as an assistant block at the end
        let mut total_height = if msg_block_heights.is_empty() {
            0u16
        } else {
            msg_block_heights.iter().sum::<u16>() + entries.len().saturating_sub(1) as u16
        };
        let mut streaming_rendered = false;
        if is_streaming {
            if let Some(ref ts) = state.turn_state_ref {
                if let Ok(ts) = ts.lock() {
                    if !ts.streaming_text.is_empty() {
                        let stream_lines: Vec<Line<'static>> = ts
                            .streaming_text
                            .lines()
                            .map(|l| {
                                Line::from(Span::styled(
                                    l.to_string(),
                                    Style::default().fg(palette.info).add_modifier(Modifier::DIM),
                                ))
                            })
                            .collect();

                        let wrapped = self.wrap_styled_lines_to_lines(
                            &stream_lines,
                            inner_width.saturating_sub(2),
                        );
                        let block_height = if wrapped.is_empty() { 1 } else { wrapped.len() as u16 };
                        total_height += block_height + 1;

                        // Render streaming block
                        let role_info =
                            role_info_for_role(&MessageRole::Assistant, palette);
                        let role_color = role_info.border_color;

                        let block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(
                                Style::default()
                                    .fg(role_color)
                                    .add_modifier(Modifier::BOLD),
                            )
                            .title(Line::from(vec![
                                Span::raw(role_info.icon),
                                Span::styled(
                                    role_info.label,
                                    Style::default()
                                        .fg(role_color)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ]));

                        let block_area = Rect::new(
                            msg_area.left(),
                            y,
                            msg_area.width,
                            (block_height + 1).max(1),
                        );
                        let body_area = block.inner(block_area);

                        if !wrapped.is_empty() {
                            frame.render_widget(Paragraph::new(wrapped), body_area);
                        }
                        frame.render_widget(block, block_area);
                        y += block_height + 1;
                        streaming_rendered = true;
                    }
                }
            }
        }

        if !streaming_rendered && is_streaming {
            total_height += 1; // placeholder for streaming
        }

        // Update scrollbar position based on mouse scroll events
        let offset = state.user_scroll_offset.swap(0, Ordering::SeqCst);
        let mut new_pos = state.scroll_pos.load(Ordering::SeqCst);
        
        tracing::info!(
            "[scroll_update] BEFORE: scroll_pos={} offset={} scrollable_range={} scroll_to_bottom={}",
            state.scroll_pos.load(Ordering::SeqCst), offset, scrollable_range, state.scroll_to_bottom
        );
        
        if offset != 0 {
            new_pos = ((new_pos as i64 + offset as i64).max(0) as usize)
                .min(scrollable_range);
        }
        
        // Only force to bottom when user hasn't scrolled away yet
        if state.scroll_to_bottom && new_pos >= scrollable_range {
            new_pos = scrollable_range;
        }
        
        let final_pos = new_pos;
        state.scroll_pos.store(new_pos, Ordering::SeqCst);
        
        tracing::info!(
            "[scroll_update] AFTER: new_pos={} (was {})",
            final_pos, state.scroll_pos.load(Ordering::SeqCst)
        );
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(new_pos);
        }
    }

    fn render_messages(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
    ) {
        let entries = state.message_entries.lock().unwrap();
        self.render_bordered_blocks(
            frame,
            area,
            state,
            palette,
            is_streaming,
            &entries,
        );
    }

    fn render_turn_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        _palette: &ratatui_themes::ThemePalette,
    ) {
        let mut stream_info = String::new();
        if let Ok(si) = state.stream_info.lock() {
            stream_info = si.clone();
        }
        if stream_info.is_empty() {
            return;
        }

        let lines: Vec<&str> = stream_info.lines().collect();
        let height = lines.len().min(3) as u16;
        if height == 0 {
            return;
        }

        let displayed_lines: Vec<Line> = lines.iter().take(3).map(|l| Line::from(*l)).collect();
        let para = Paragraph::new(displayed_lines);
        let status_w = lines.iter().map(|l| l.len() as u16).max().unwrap_or(1) + 2;
        let display_area = Rect::new(
            area.x + area.width.saturating_sub(status_w + 2),
            area.y + area.height.saturating_sub(height + 2),
            status_w.min(area.width.saturating_sub(2)),
            height.min(area.height.saturating_sub(2)),
        );
        frame.render_widget(para, display_area);
    }

    /// Render the full message display widget in `area`.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
    ) {
        self.render_messages(frame, area, state, palette, is_streaming);
        self.render_turn_status(frame, area, state, palette);

        // Draw streaming indicator in messages panel area
        if is_streaming {
            let indicator_text = " Streaming... ";
            let w = indicator_text.len() as u16 + 2;
            let indicator_area = Rect::new(
                area.right()
                    .saturating_sub(w + 2)
                    .min(area.width.saturating_sub(2)),
                area.y + 1,
                w,
                1,
            );
            let para = Paragraph::new(Line::from(Span::styled(
                indicator_text,
                Style::default().fg(palette.info),
            )));
            frame.render_widget(para, indicator_area);
        }
    }

    /// Append a user message to the internal display state.
    pub fn append_user_message(&mut self, state: &mut ConversationState, text: &str) {
        let body_lines = vec![crate::widgets::message_renderer::StyledLine {
            content: Line::from(text.to_string()),
            role_color: None,
        }];
        state.message_entries.lock().unwrap().push(
            crate::widgets::message_renderer::MessageRenderEntry {
                role: oben_models::MessageRole::User,
                body_lines,
                is_tool_result: false,
                tool_calls: Vec::new(),
            },
        );
    }

    /// Append an info/system message to the internal display state.
    /// Uses the System role style (gear icon, accent color).
    pub fn append_info_message(&mut self, state: &mut ConversationState, text: &str) {
        use ratatui_themes::ThemeName;
        let palette = ThemeName::default().palette();
        for line in text.lines() {
            let line_owned = line.to_string();
            let spans = vec![
                Span::styled(
                    "\u{2699} ",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(line_owned, Style::default().fg(palette.info)),
            ];
            let body_lines = vec![crate::widgets::message_renderer::StyledLine {
                content: Line::from(spans),
                role_color: None,
            }];
            state.message_entries.lock().unwrap().push(
                crate::widgets::message_renderer::MessageRenderEntry {
                    role: oben_models::MessageRole::System,
                    body_lines,
                    is_tool_result: false,
                    tool_calls: Vec::new(),
                },
            );
        }
    }

    /// Rebuild message entries from session messages using the renderer.
    pub fn rebuild_from_messages(
        &self,
        state: &mut ConversationState,
        messages: &[Message],
        renderer: &MessageRenderer,
    ) {
        let mut entries = Vec::new();
        for msg in messages {
            entries.push(renderer.render_entry(msg));
        }
        let mut entry_lock = state.message_entries.lock().unwrap();
        *entry_lock = entries;
    }

    /// Update stream_info from turn state into ConversationState.
    pub fn update_stream_info(&self, state: &mut ConversationState, turn_state: &TurnState) {
        let mut parts = Vec::new();

        let active = &turn_state.active_tools;
        if !active.is_empty() {
            let names: Vec<String> = active
                .iter()
                .take(2)
                .map(|t| {
                    format!(
                        " {}\u{1F527} {}",
                        t.name,
                        t.context.chars().take(30).collect::<String>()
                    )
                })
                .collect();
            parts.push(format!("\u{1F527} {}", names.join(", ")).to_string());
        }

        if !parts.is_empty() {
            let info = parts.join("\n");
            if let Ok(mut si) = state.stream_info.lock() {
                *si = info;
            }
        } else if let Ok(mut si) = state.stream_info.lock() {
            si.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::turn_state::{ActiveTool, TurnState};
    use std::time::Instant;

    /// Given: a TurnState with active tool calls AND streaming text
    /// When: update_stream_info is called
    /// Then: stream_info contains only tool info, NOT streaming text
    #[test]
    fn test_update_stream_info_excludes_streaming_text() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-1".into(),
            name: "file_read".into(),
            started_at: Instant::now(),
            context: "/Users/test/config.yaml".into(),
        });
        turn.streaming_text =
            "This is streaming text that should NOT appear in stream_info".into();

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(!info.contains("streaming text"));
        assert!(!info.is_empty());
        assert!(info.contains("file_read"));
    }

    /// Given: a TurnState with ONLY streaming text, no active tools
    /// When: update_stream_info is called
    /// Then: stream_info stays empty (no misleading tool status)
    #[test]
    fn test_update_stream_info_empty_with_no_tools() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.streaming_text = "Some streaming content".into();

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.is_empty());
    }

    /// Given: a TurnState with active tools
    /// When: update_stream_info is called
    /// Then: stream_info includes the tool name
    #[test]
    fn test_update_stream_info_includes_tool_name() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-2".into(),
            name: "search_files".into(),
            started_at: Instant::now(),
            context: "/some/path".into(),
        });

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.contains("search_files"));
    }

    /// Given: a TurnState with streaming text AND active tools
    /// When: update_stream_info is called
    /// Then: streaming_text is NOT included in stream_info
    #[test]
    fn test_streaming_text_not_duplicated_in_stream_info() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        let streaming_content = "The Clockmaker of Lost Hours";
        turn.streaming_text = streaming_content.into();
        turn.active_tools.push(ActiveTool {
            id: "call-write".into(),
            name: "file_write".into(),
            started_at: Instant::now(),
            context: "/Users/test/docs.md".into(),
        });

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.contains("file_write"));
        assert!(info.is_empty() || !info.contains("The Clockmaker of Lost Hours"));
    }
}
