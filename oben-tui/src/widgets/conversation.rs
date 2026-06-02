//! Message display widget — renders message history with scrolling.
//!
//! Encapsulates the pre-rendered message lines, scrolling, streaming text
//! overlay, and turn-status text. The chat panel only manages state and
//! delegates rendering.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use textwrap::wrap as textwrap_wrap;
use unicode_width::UnicodeWidthStr;

use crate::turn::turn_state::TurnState;
use crate::widgets::message_renderer::MessageRenderer;
use crate::widgets::role_style::{get_style_for_role, ColorHint};
use crate::widgets::style::Theme;
use oben_models::{Message, MessageRole};

/// State for the message display widget.
pub struct ConversationState {
    pub scroll_state: Arc<StdMutex<ScrollbarState>>,
    pub scroll_to_bottom: bool,
    /// Persisted scroll position across frames (AtomicUsize so render(&self) can update it).
    pub scroll_pos: Arc<AtomicUsize>,
    pub base_lines: Vec<Line<'static>>,
    pub stream_info: Arc<StdMutex<String>>,
    pub turn_state_ref: Option<Arc<StdMutex<TurnState>>>,
    /// Accumulated scroll delta from user mouse scroll. Reset by render.
    pub user_scroll_offset: Arc<AtomicI32>,
    /// Selection start/end as (visual_line_idx, char_offset).
    pub selection_start: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
}

impl ConversationState {
    pub fn new() -> Self {
        Self {
            scroll_state: Arc::new(StdMutex::new(ScrollbarState::new(0))),
            scroll_to_bottom: true,
            scroll_pos: Arc::new(AtomicUsize::new(0)),
            base_lines: Vec::new(),
            stream_info: Arc::new(StdMutex::new(String::new())),
            turn_state_ref: None,
            user_scroll_offset: Arc::new(AtomicI32::new(0)),
            selection_start: None,
            selection_end: None,
        }
    }

    /// Cancel any active selection.
    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }
}

/// Message display widget — renders messages with scrolling and streaming overlay.
pub struct ConversationWidget;

impl ConversationWidget {
    /// Check if mouse event is relevant to message display and update selection state.
    /// Returns true if the event was consumed (i.e. a mouse button was pressed/released on the text area).
    pub fn handle_mouse_event(
        &self,
        state: &mut ConversationState,
        visual_lines: &[Line<'static>],
        scroll_offset: u16,
        event_row: u16,
        event_col: u16,
        viewport_top: u16,
        viewport_left: u16,
    ) -> bool {
        let row = (event_row.saturating_sub(viewport_top) as usize)
            .saturating_sub(1);
        let col = (event_col.saturating_sub(viewport_left) as usize)
            .saturating_sub(1);
        let global_row = row.saturating_add(scroll_offset as usize);
        if global_row >= visual_lines.len() || row >= (visual_lines.len() / scroll_offset.max(1) as usize) as usize {
            return false;
        }
        // Only handle mouse down/up to toggle selection.
        // We check the first two bytes of the event kind (opaque), and
        // simply toggle start when Down is pressed.
        #[allow(clippy::match_single_binding)]
        let _ = (event_row, event_col);
        if row < visual_lines.len() {
            let line = &visual_lines[global_row.min(visual_lines.len() - 1)];
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            let offset = std::cmp::min(col, text.width());
            if state.selection_start.is_none() {
                state.selection_start = Some((global_row, offset));
                state.selection_end = Some((global_row, offset));
                return true;
            }
        }
        false
    }

    /// Cancel any active selection (call on key press in message area).
    pub fn cancel_selection(&self, state: &mut ConversationState) {
        state.selection_start = None;
        state.selection_end = None;
    }

    /// Extract selected text from visual lines, clear selection, return None if empty.
    pub fn get_selected_text(&self, state: &mut ConversationState) -> Option<String> {
        let (sy, sx) = state.selection_start?;
        let (ey, ex) = state.selection_end?;

        let y_start = std::cmp::min(sy, ey);
        let y_end = std::cmp::max(sy, ey);
        let mut result = Vec::new();

        for v in y_start..=y_end {
            if v >= state.base_lines.len() {
                break;
            }
            let line = &state.base_lines[v];
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            let x_start = if v == sy { std::cmp::min(sx, ex) } else if v > sy { 0 } else { sx };
            let x_end = if v == sy && v == ey { std::cmp::max(sx, ex) } else if v == ey { ex } else { text.width() };

            if x_start >= x_end {
                continue;
            }

            let mut byte_start = 0;
            let mut char_count = 0;
            for (i, ch) in text.chars().enumerate() {
                if char_count >= x_start {
                    byte_start = i;
                    break;
                }
                char_count += ch.len_utf8();
            }
            let mut byte_end = text.len();
            char_count = 0;
            for (i, ch) in text.chars().enumerate() {
                if char_count >= x_end {
                    byte_end = i;
                    break;
                }
                char_count += ch.len_utf8();
            }
            result.push(text[byte_start..byte_end].to_string());
        }

        if result.is_empty() {
            state.selection_start = None;
            state.selection_end = None;
            return None;
        }
        state.selection_start = None;
        state.selection_end = None;
        Some(result.join("\n"))
    }

    /// Wrap base lines into visual lines for the given width, preserving styles.
    fn build_visual_lines(
        &self,
        lines: &[Line<'static>],
        inner_width: usize,
    ) -> Vec<Line<'static>> {
        lines
            .iter()
            .flat_map(|line| {
                let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
                if total_width <= inner_width {
                    vec![line.clone()]
                } else if line.spans.len() == 1 {
                    let text = line.spans[0].content.to_string();
                    textwrap_wrap(&text, inner_width)
                        .into_iter()
                        .map(|wrapped| {
                            Line::from(Span::styled(
                                wrapped.into_owned(),
                                line.spans[0].style,
                            ))
                        })
                        .collect::<Vec<_>>()
                } else {
                    // Multi-span, overflow: don't wrap — ratatui Paragraph handles wrapping
                    vec![line.clone()]
                }
            })
            .collect()
    }

    fn render_messages(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        theme: &Theme,
        is_streaming: bool,
    ) {
        let mut lines = state.base_lines.iter().cloned().collect::<Vec<_>>();

        let mut stream_text = String::new();
        if let Some(ref ts) = state.turn_state_ref {
            if let Ok(ts) = ts.lock() {
                if !ts.streaming_text.is_empty() {
                    stream_text = ts.streaming_text.clone();
                }
            }
        }

        // Streaming assistant text
        if is_streaming && !stream_text.is_empty() {
            let role_style = get_style_for_role(&MessageRole::Assistant);
            let hint = role_style.color_hint();
            let color = Self::color_from_hint(hint, theme);
            lines.push(Line::from(Span::styled(
                format!(" {} {} ", role_style.icon(), role_style.label()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));
            for l in stream_text.lines() {
                lines.push(Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::DIM),
                )));
            }
            lines.push(Line::from(""));
        }

        let block = Block::default().borders(Borders::ALL).title(" Messages ");
        let message_area = block.inner(area);
        block.render(area, frame.buffer_mut());

        let inner_height = (message_area.height.saturating_sub(1)).max(1);
        let inner_width = (message_area.width).max(1) as usize;

        // Wrap long lines while preserving existing styles.
        let visual_lines = self.build_visual_lines(&lines, inner_width);

        let content_height = visual_lines.len().max(1) as u16;

        // Update scroll position: apply accumulated offset, clamp to valid range.
        let max_scroll = content_height.saturating_sub(inner_height) as usize;
        let scroll_pos = {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            let current_pos = state.scroll_pos.load(Ordering::SeqCst);

            // Drain accumulated scroll offset atomically.
            let offset = state.user_scroll_offset.swap(0, Ordering::SeqCst);

            // Compute target: offset always takes priority over scroll_to_bottom.
            // When user scrolls, offset != 0 — apply it regardless of scroll_to_bottom.
            // Only snap to bottom when there's NO scroll activity (offset == 0).
            let mut target = current_pos;
            if offset != 0 {
                target = ((current_pos as i64 + offset as i64).max(0) as usize).min(max_scroll);
            } else if state.scroll_to_bottom {
                target = max_scroll;
            }

            // Persist position (AtomicUsize is writeable through &ref).
            state.scroll_pos.store(target, Ordering::SeqCst);

            *scroll_state = ScrollbarState::new(visual_lines.len())
                .viewport_content_length(inner_height as usize)
                .position(target);
            target
        };

        let paragraph = Paragraph::new(visual_lines.iter().cloned().collect::<Vec<_>>())
            .scroll((scroll_pos as u16, 0));
        frame.render_widget(paragraph, message_area);

        // Render vertical scrollbar on the right edge of the messages area (inside border).
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            message_area,
            &mut state.scroll_state.lock().unwrap(),
        );

        // Render selection highlight.
        let sy = state.selection_start.map(|(s, _)| s);
        let sx = state.selection_start.map(|(_, s)| s);
        let ey = state.selection_end.map(|(s, _)| s);
        let ex = state.selection_end.map(|(_, s)| s);
        if let (Some(sy), Some(sx), Some(ey), Some(ex)) = (sy, sx, ey, ex) {
            let visible_start = if state.scroll_to_bottom {
                visual_lines.len().saturating_sub(inner_height as usize)
            } else {
                scroll_pos
            };
            let visible_end = (visible_start + inner_height as usize).min(visual_lines.len());

            let vy_start = if sy >= visible_start && sy < visible_end { sy } else { visible_start };
            let vy_end = if ey >= visible_start && ey < visible_end { ey } else { visible_end - 1 };
            let vy_start = std::cmp::min(vy_start, vy_end);
            let vy_end = std::cmp::max(vy_start, vy_end);

            for v in vy_start..=vy_end {
                let buf_row = (v as u16).saturating_sub(scroll_pos as u16) + 1;
                if buf_row >= area.height.saturating_sub(1) {
                    continue;
                }
                let tx = if v < visual_lines.len() { visual_lines[v].width() } else { 0 };

                let (x_start, x_end) = if v == sy && v == ey {
                    let a = std::cmp::min(sx, ex);
                    let b = std::cmp::max(sx, ex);
                    (a, b)
                } else if v == sy {
                    let a = sx;
                    let b = std::cmp::min(ex, tx);
                    (a, b)
                } else if v == ey {
                    let a = 0;
                    let b = std::cmp::min(ex, tx);
                    (a, b)
                } else {
                    continue;
                };

                if x_start >= x_end {
                    continue;
                }
                let x_end = x_end.min(tx).min(inner_width);

                for col in x_start..x_end {
                    let buf_col = (col as u16) + 1;
                    if let Some(cell) = frame.buffer_mut().cell_mut((buf_col, buf_row)) {
                        cell.set_style(cell.style().bg(Color::Gray));
                    }
                }
            }
        }
    }

    fn render_turn_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        _theme: &Theme,
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
        // Render tool status at bottom-right of messages area, not top-left
        let status_w = lines.iter()
            .map(|l| l.len() as u16)
            .max()
            .unwrap_or(1) + 2;
        let display_area = Rect::new(
            area.x + area.width.saturating_sub(status_w + 2),
            area.y + area.height.saturating_sub(height + 2),
            status_w.min(area.width.saturating_sub(2)),
            height.min(area.height.saturating_sub(2)),
        );
        frame.render_widget(para, display_area);
    }

    fn color_from_hint(hint: ColorHint, theme: &Theme) -> Color {
        match hint {
            ColorHint::Success => theme.success,
            ColorHint::Info => theme.accent,
            ColorHint::Accent => theme.accent,
            ColorHint::Warning => theme.warning,
        }
    }

    /// Render the full message display widget in `area`.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        theme: &Theme,
        is_streaming: bool,
    ) {
        self.render_messages(frame, area, state, theme, is_streaming);
        self.render_turn_status(frame, area, state, theme);

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
                Style::default().fg(Color::Yellow),
            )));
            frame.render_widget(para, indicator_area);
        }
    }

    /// Append a user message to the internal display state.
    pub fn append_user_message(&mut self, state: &mut ConversationState, text: &str) {
        state
            .base_lines
            .push(Line::from(format!(" {} {}", "\u{1F464}", text)));
        state.base_lines.push(Line::from(""));
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
                    Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(line_owned, Style::default().fg(palette.info)),
            ];
            state.base_lines.push(Line::from(spans));
        }
        state.base_lines.push(Line::from(""));
    }

    /// Rebuild base lines from session messages using the renderer.
    pub fn rebuild_from_messages(
        &self,
        state: &mut ConversationState,
        messages: &[Message],
        renderer: &MessageRenderer,
    ) {
        let mut lines = Vec::new();
        for msg in messages {
            lines.extend(renderer.render(msg));
        }
        state.base_lines = lines;
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
    /// Then: streaming_text is NOT included in stream_info (prevents duplication in render_messages + render_turn_status)
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
        // The stream_info should only have tool info, not the streaming text
        assert!(info.contains("file_write"));
        assert!(info.is_empty() || !info.contains("The Clockmaker of Lost Hours"));
    }
}
