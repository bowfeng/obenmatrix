//! Chat panel — message history, streaming, input bar, tool call display.

use super::Panel;
use crate::{App, TuiEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState, ScrollbarOrientation};
use oben_models::Message;

pub enum ChatViewMode {
    History,
    ToolOutput(usize),
    Streaming,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub text: String,
    pub has_tool_calls: bool,
    pub tool_calls: Vec<String>,
    pub tool_results: Vec<(String, String)>,
}

pub struct ChatPanel {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub scroll: usize,
    pub max_scroll: usize,
    pub view_mode: ChatViewMode,
    pub streaming: bool,
    pub session_id: Option<String>,
    pub streaming_text: String,
}

impl ChatPanel {
    pub fn new(session_id: Option<String>, messages: Option<Vec<Message>>) -> Self {
        let chat_messages = messages
            .map(|msgs| msgs.iter().map(to_chat_msg).collect())
            .unwrap_or_default();
        Self {
            messages: chat_messages,
            input: String::new(),
            cursor: 0,
            scroll: 0,
            max_scroll: 0,
            view_mode: ChatViewMode::History,
            streaming: false,
            session_id,
            streaming_text: String::new(),
        }
    }
}

fn role_to_str(role: &oben_models::MessageRole) -> &'static str {
    match role {
        oben_models::MessageRole::System => "System",
        oben_models::MessageRole::User => "User",
        oben_models::MessageRole::Assistant => "Assistant",
        oben_models::MessageRole::Tool => "Tool",
    }
}

fn to_chat_msg(msg: &Message) -> ChatMessage {
    let text = msg.content.to_text();
    let has_tool_calls = msg.tool_calls.is_some() && !msg.tool_calls.as_ref().unwrap().is_empty();
    let tool_calls: Vec<String> = msg
        .tool_calls
        .as_ref()
        .map(|calls| calls.iter().map(|tc| tc.tool_name.clone()).collect())
        .unwrap_or_default();
    let is_tool = msg.role == oben_models::MessageRole::Tool;
    let tool_results: Vec<(String, String)> = if is_tool {
        vec![("tool".to_string(), text.clone())]
    } else {
        vec![]
    };
    ChatMessage {
        role: role_to_str(&msg.role).to_string(),
        text,
        has_tool_calls,
        tool_calls,
        tool_results,
    }
}

impl Panel for ChatPanel {
    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);
        draw_messages(frame, self, chunks[0]);

        let input_text = format!("> {}", &self.input[self.cursor..]);
        let input_para = Paragraph::new(Text::from(input_text.as_str()))
            .block(Block::default().borders(Borders::ALL).title(" Input "));
        frame.render_widget(input_para, chunks[1]);

        let cursor_x = 2 + unicode_width::UnicodeWidthStr::width(&self.input[..self.cursor]) as u16;
        frame.set_cursor_position(Position::new(chunks[1].x + cursor_x, chunks[1].y + 1));

        if self.streaming {
            let indicator_text = " ⏳ Streaming... ".to_string();
            let indicator_span = Span::styled(
                indicator_text.clone(),
                Style::default().fg(Color::Yellow),
            );
            let indicator_para = Paragraph::new(Line::from(indicator_span));
            let indicator_area = Rect::new(
                chunks[0].right() - indicator_text.len() as u16 - 2,
                chunks[0].y + 1,
                indicator_text.len() as u16 + 2,
                1,
            );
            frame.render_widget(indicator_para, indicator_area);
        }
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) {
        if self.streaming {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.streaming = false;
                self.view_mode = ChatViewMode::History;
            }
            return;
        }

        match key.code {
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                if !self.input.is_empty() {
                    // Send input to async main loop via channel
                    if let Some(tx) = &app.input_tx {
                        let _ = tx.send(TuiEvent::ChatInput(self.input.clone()));
                    }
                    self.input.clear();
                    self.cursor = 0;
                }
            }
            KeyCode::Left => { if self.cursor > 0 { self.cursor -= 1; } }
            KeyCode::Right => { if self.cursor < self.input.len() { self.cursor += 1; } }
            KeyCode::Backspace => {
                if self.cursor > 0 { self.input.remove(self.cursor - 1); self.cursor -= 1; }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() { self.input.remove(self.cursor); }
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor = 0;
            }
            _ => {}
        }
    }
}

fn draw_messages(frame: &mut Frame, panel: &ChatPanel, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &panel.messages {
        let role_color = match msg.role.as_str() {
            "User" => Color::Green,
            "Assistant" => Color::Blue,
            "System" => Color::Magenta,
            "Tool" => Color::Yellow,
            _ => Color::Gray,
        };
        let role_line = Line::from(Span::styled(
            format!(" ── {} ── ", msg.role),
            Style::default().fg(role_color).add_modifier(Modifier::BOLD),
        ));
        lines.push(role_line);

        for line in msg.text.split('\n') {
            lines.push(Line::from(Span::raw(line.to_string())));
        }

        if msg.has_tool_calls && !msg.tool_calls.is_empty() {
            for tc in &msg.tool_calls {
                lines.push(Line::from(format!("   🔧 {}", tc)));
            }
            for (tool_name, output) in &msg.tool_results {
                let preview = if output.len() > 50 {
                    format!("{}...", &output[..50])
                } else {
                    output.clone()
                };
                lines.push(Line::from(format!("   ✅ {} → {}", tool_name, preview)));
            }
        }
        lines.push(Line::from(""));
    }

    let total_lines = lines.len();
    let para = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" Messages "));
    frame.render_widget(para, area);

    if total_lines > area.height as usize {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut state = ScrollbarState::new(total_lines).position(panel.scroll);
        frame.render_stateful_widget(scrollbar, area, &mut state);
    }
}
