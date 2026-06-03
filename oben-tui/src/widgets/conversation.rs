//! Message display widget — renders message history with scrolling.
//!
//! Encapsulates the pre-rendered message lines, scrolling, streaming text
//! overlay, and turn-status text. Messages are rendered as bordered blocks
//! (like the Hermes Agent reference), with each message wrapped in a rounded-border
//! Block whose title shows the role label and icon.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, ScrollbarState,
};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use crate::turn::turn_state::TurnState;
use crate::widgets::layout;
use crate::widgets::message_renderer::{MessageRenderEntry, MessageRenderer};
use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageRole};

/// Block rendering strategy for a message entry.
pub enum BlockType<'a> {
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
            message_entries: Arc::new(StdMutex::new(Vec::new())),
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

    fn role_title(
        &self,
        role: &MessageRole,
        palette: &ratatui_themes::ThemePalette,
    ) -> Line<'static> {
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
    ///
    /// Three-phase architecture:
    ///   Phase 1: Layout — compute heights, scroll offset, visible areas (pure data)
    ///   Phase 2: Render   — draw blocks using the pre-calculated areas
    ///   Phase 3: Scroll   — update scroll position from mouse delta
    fn render_bordered_blocks(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
        entries: &[MessageRenderEntry],
    ) {
        // ─── Outer frame ───────────────────────────────────────────────
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

        let inner_width = (msg_area.width as usize).saturating_sub(2);
        let inner_height = (msg_area.height as usize).saturating_sub(1);

        // ─── Phase 1: Layout calculation ───────────────────────────────
        // Wrap lines and compute heights — pure data, no rendering.

        // Accumulate (entry_index, block_type, wrapped_lines) for later rendering
        let mut layout_entries: Vec<(usize, BlockType<'_>, Vec<Line<'static>>)> = Vec::new();
        for entry in entries {
            let plain_lines: Vec<Line<'static>> = entry
                .body_lines
                .iter()
                .map(|sl| sl.content.clone())
                .collect();

            let block_type = if entry.is_tool_result {
                BlockType::ToolResult
            } else {
                BlockType::Message(&entry.role)
            };

            // Wrap using the layout module's function (consistent height estimation)
            let wrapped =
                layout::wrap_styled_lines_to_lines(&plain_lines, inner_width.saturating_sub(2));

            layout_entries.push((layout_entries.len(), block_type, wrapped));
        }

        // Compute block heights using the layout module's estimator
        let block_heights: Vec<u16> = layout_entries
            .iter()
            .map(|(_, _block_type, wrapped)| {
                // Estimate height from wrapped line count + border
                // Match exactly what the render loop produces
                let body_height = wrapped.len().max(1) as u16;
                body_height + layout::BODY_HEIGHT_ADJUSTER
            })
            .collect();

        let total_height = layout::calc_total_height(&block_heights);

        // Compute scroll offset using the layout module
        let scroll_offset = layout::compute_scroll_offset(
            total_height,
            inner_height as u16,
            (total_height as i64 - inner_height as i64).max(0) as usize,
            state.scroll_to_bottom,
            &block_heights,
            state.scroll_pos.load(Ordering::SeqCst),
        );

        // Update scrollbar state
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(scroll_offset);
        }

        // Calculate visible areas using the layout module
        let visible_areas = layout::calc_visible_areas(
            msg_area.top(),
            msg_area.bottom(),
            msg_area.left(),
            msg_area.width,
            scroll_offset,
            &block_heights,
        );

        // ─── Phase 2: Render visible blocks ────────────────────────────
        for (idx, block_rect) in visible_areas {
            let (_entry_idx, block_type, wrapped) = &layout_entries[idx];

            // body_area = block.inner(block_area) — calculated at render time
            // using the actual Block struct (handles title, borders correctly)
            let is_tool_result = matches!(block_type, BlockType::ToolResult);

            // Build the block (borders + title)
            let block = if is_tool_result {
                let indent = layout::TOOL_INDENT;
                let _box_width = (msg_area.width as usize).saturating_sub((indent * 2) as usize) as u16;
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.muted))
                    .border_type(BorderType::Rounded)
            } else {
                let role = match block_type {
                    BlockType::Message(r) => r,
                    BlockType::ToolResult => unreachable!(),
                };
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.role_border_style(role, palette))
                    .title(self.role_title(role, palette))
            };

            // Render body (Paragraph) before block (borders on top)
            let body_area = block.inner(block_rect);
            if block_rect.height > layout::BODY_HEIGHT_ADJUSTER && !wrapped.is_empty() {
                let body_lines: Vec<Line> = wrapped
                    .iter()
                    .take(body_area.height as usize)
                    .cloned()
                    .map(|line| {
                        if is_tool_result {
                            // Tool result: muted style
                            let muted_spans: Vec<Span> = line
                                .spans
                                .iter()
                                .filter_map(|span| {
                                    span.style
                                        .fg(palette.muted)
                                        .add_modifier(Modifier::DIM)
                                        .fg
                                        .map(|fg| {
                                            Span::styled(
                                                span.content.clone(),
                                                Style::default().fg(fg).add_modifier(Modifier::DIM),
                                            )
                                        })
                                        .or_else(|| {
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
                        } else {
                            line
                        }
                    })
                    .collect();

                if !body_lines.is_empty() {
                    frame.render_widget(Paragraph::new(body_lines), body_area);
                }
            }

            frame.render_widget(block, block_rect);
        }

        // ─── Phase 2.5: Streaming block (rendered after regular entries) ─
        let mut total_height = total_height;
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
                                    Style::default()
                                        .fg(palette.info)
                                        .add_modifier(Modifier::DIM),
                                ))
                            })
                            .collect();

                        let wrapped = layout::wrap_styled_lines_to_lines(
                            &stream_lines,
                            inner_width.saturating_sub(2),
                        );
                        let block_height = if wrapped.is_empty() {
                            1
                        } else {
                            wrapped.len() as u16
                        };
                        total_height += block_height + 1;

                        // Render streaming block after regular entries
                        let role_info = role_info_for_role(&MessageRole::Assistant, palette);
                        let role_color = role_info.border_color;

                        // Calculate y position: below all regular entries
                        let streaming_y: u16 = {
                            let mut y = msg_area.top();
                            let mut i = 0;
                            while i < entries.len() {
                                y += layout::estimate_block_height(
                                    &entries[i].body_lines,
                                    &layout_entries[i].1,
                                    inner_width.saturating_sub(2),
                                ) + layout::INTER_BLOCK_MARGIN;
                                i += 1;
                            }
                            y
                        };

                        let block_area = Rect::new(
                            msg_area.left(),
                            streaming_y,
                            msg_area.width,
                            (block_height + 1).max(1),
                        );
                        let block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(
                                Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                            )
                            .title(Line::from(vec![
                                Span::raw(role_info.icon),
                                Span::styled(
                                    role_info.label,
                                    Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                                ),
                            ]));
                        let body_area = block.inner(block_area);

                        if !wrapped.is_empty() {
                            frame.render_widget(Paragraph::new(wrapped), body_area);
                        }
                        frame.render_widget(block, block_area);
                        streaming_rendered = true;
                    }
                }
            }
        }

        if !streaming_rendered && is_streaming {
            total_height += 1; // placeholder for streaming
        }

        // ─── Phase 3: Scroll position update (mouse wheel) ─────────────
        let offset = state.user_scroll_offset.swap(0, Ordering::SeqCst);
        let mut new_pos = state.scroll_pos.load(Ordering::SeqCst);

        tracing::info!(
            "[scroll_update] BEFORE: scroll_pos={} offset={} scrollable_range={} scroll_to_bottom={}",
            state.scroll_pos.load(Ordering::SeqCst), offset, (total_height as i64 - inner_height as i64).max(0), state.scroll_to_bottom
        );

        if offset != 0 {
            new_pos = ((new_pos as i64 + offset as i64).max(0) as usize)
                .min((total_height as i64 - inner_height as i64).max(0) as usize);
        }

        // Only force to bottom when user hasn't scrolled away yet
        let scrollable_range = (total_height as i64 - inner_height as i64).max(0) as usize;
        if state.scroll_to_bottom && new_pos >= scrollable_range {
            new_pos = scrollable_range;
        }

        let final_pos = new_pos;
        state.scroll_pos.store(new_pos, Ordering::SeqCst);

        tracing::info!(
            "[scroll_update] AFTER: new_pos={} (was {})",
            final_pos,
            state.scroll_pos.load(Ordering::SeqCst)
        );
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(new_pos);
        }
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
        let entries = state.message_entries.lock().unwrap();
        self.render_bordered_blocks(frame, area, state, palette, is_streaming, &entries);
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
        turn.streaming_text = "This is streaming text that should NOT appear in stream_info".into();

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
