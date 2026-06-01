//! Message display widget — renders message history with scrolling.
//!
//! Encapsulates the pre-rendered message lines, scrolling, streaming text
//! overlay, and turn-status text. The chat panel only manages state and
//! delegates rendering.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::{Arc, Mutex as StdMutex};
use textwrap::wrap as textwrap_wrap;
use unicode_width::UnicodeWidthStr;
use tui_scrollview::{ScrollView, ScrollViewState};

use crate::turn::event::TurnState;
use crate::widgets::message_renderer::MessageRenderer;
use crate::widgets::role_style::{get_style_for_role, ColorHint};
use crate::widgets::style::Theme;
use oben_models::{Message, MessageRole};

/// State for the message display widget.
pub struct MessageDisplayState {
    pub scroll_state: Arc<StdMutex<ScrollViewState>>,
    pub scroll_to_bottom: bool,
    pub base_lines: Vec<Line<'static>>,
    pub stream_info: Arc<StdMutex<String>>,
    pub turn_state_ref: Option<Arc<StdMutex<TurnState>>>,
}

impl MessageDisplayState {
    pub fn new() -> Self {
        Self {
            scroll_state: Arc::new(StdMutex::new(ScrollViewState::default())),
            scroll_to_bottom: true,
            base_lines: Vec::new(),
            stream_info: Arc::new(StdMutex::new(String::new())),
            turn_state_ref: None,
        }
    }
}

/// Message display widget — renders messages with scrolling and streaming overlay.
pub struct MessageDisplay;

impl MessageDisplay {
    fn render_messages(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &MessageDisplayState,
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
        block.render(area, frame.buffer_mut());

        let inner_height = (area.height.saturating_sub(2)).max(1);
        let inner_width = (area.width.saturating_sub(2)).max(1) as usize;

        // Scroll layout: wrap long lines while preserving existing styles.
        let mut visual_lines: Vec<Line<'static>> = Vec::new();
        for line in &lines {
            if line.spans.len() != 1 {
                visual_lines.push(line.clone());
                continue;
            }
            let text = line.spans[0].content.to_string();
            if text.width() <= inner_width {
                visual_lines.push(line.clone());
            } else {
                for wrapped in textwrap_wrap(&text, inner_width) {
                    visual_lines.push(Line::from(Span::styled(
                        wrapped.into_owned(),
                        line.spans[0].style,
                    )));
                }
            }
        }

        let total_lines = visual_lines.len();
        let content_height = (total_lines.max(1)) as u16;
        let mut scroll_view = ScrollView::new(Size::new(inner_width as u16, content_height));

        scroll_view.render_widget(
            Paragraph::new(visual_lines).block(Block::default().borders(Borders::NONE)),
            scroll_view.area(),
        );

        let viewport_area = Rect::new(area.x + 1, area.y + 1, inner_width as u16, inner_height);

        let mut scrollable_view = state.scroll_state.lock().unwrap();
        let max_offset = content_height.saturating_sub(inner_height.max(1));
        if state.scroll_to_bottom || scrollable_view.offset().y >= max_offset {
            scrollable_view.set_offset(Position::new(0, max_offset.max(1)));
        }
        // Force horizontal offset to 0
        let y = scrollable_view.offset().y;
        scrollable_view.set_offset(Position::new(0, y));
        scroll_view.render(viewport_area, frame.buffer_mut(), &mut scrollable_view);
    }

    fn render_turn_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &MessageDisplayState,
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
        state: &MessageDisplayState,
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
    pub fn append_user_message(&mut self, state: &mut MessageDisplayState, text: &str) {
        state
            .base_lines
            .push(Line::from(format!(" {} {}", "\u{1F464}", text)));
        state.base_lines.push(Line::from(""));
    }

    /// Rebuild base lines from session messages using the renderer.
    pub fn rebuild_from_messages(
        &self,
        state: &mut MessageDisplayState,
        messages: &[Message],
        renderer: &MessageRenderer,
    ) {
        let mut lines = Vec::new();
        for msg in messages {
            lines.extend(renderer.render(msg));
        }
        state.base_lines = lines;
    }

    /// Update stream_info from turn state into MessageDisplayState.
    pub fn update_stream_info(&self, state: &mut MessageDisplayState, turn_state: &TurnState) {
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
    use crate::turn::event::{ActiveTool, TurnState};
    use std::time::Instant;

    /// Given: a TurnState with active tool calls AND streaming text
    /// When: update_stream_info is called
    /// Then: stream_info contains only tool info, NOT streaming text
    #[test]
    fn test_update_stream_info_excludes_streaming_text() {
        let mut state = MessageDisplayState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-1".into(),
            name: "file_read".into(),
            started_at: Instant::now(),
            context: "/Users/test/config.yaml".into(),
        });
        turn.streaming_text = "This is streaming text that should NOT appear in stream_info".into();

        MessageDisplay.update_stream_info(&mut state, &turn);

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
        let mut state = MessageDisplayState::new();
        let mut turn = TurnState::new();
        turn.streaming_text = "Some streaming content".into();

        MessageDisplay.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.is_empty());
    }

    /// Given: a TurnState with active tools
    /// When: update_stream_info is called
    /// Then: stream_info includes the tool name
    #[test]
    fn test_update_stream_info_includes_tool_name() {
        let mut state = MessageDisplayState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-2".into(),
            name: "search_files".into(),
            started_at: Instant::now(),
            context: "/some/path".into(),
        });

        MessageDisplay.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.contains("search_files"));
    }

    /// Given: a TurnState with streaming text AND active tools
    /// When: update_stream_info is called
    /// Then: streaming_text is NOT included in stream_info (prevents duplication in render_messages + render_turn_status)
    #[test]
    fn test_streaming_text_not_duplicated_in_stream_info() {
        let mut state = MessageDisplayState::new();
        let mut turn = TurnState::new();
        let streaming_content = "The Clockmaker of Lost Hours";
        turn.streaming_text = streaming_content.into();
        turn.active_tools.push(ActiveTool {
            id: "call-write".into(),
            name: "file_write".into(),
            started_at: Instant::now(),
            context: "/Users/test/docs.md".into(),
        });

        MessageDisplay.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        // The stream_info should only have tool info, not the streaming text
        assert!(info.contains("file_write"));
        assert!(info.is_empty() || !info.contains("The Clockmaker of Lost Hours"));
    }
}
