//! Chat panel — message history, streaming, input bar, tool call display.

use super::Panel;
use crate::{App, TuiEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState, ScrollbarOrientation};
use unicode_width::UnicodeWidthStr;
use oben_models::Message;
use std::time::Instant;

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

pub enum ToolTrailStatus {
    Running(usize),
    Success,
    Error,
}

pub struct ToolTrailLine {
    pub status: ToolTrailStatus,
    pub tool_name: String,
    pub output_preview: String,
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
    pub last_enter_time: Option<Instant>,
    pub tool_trail: Vec<ToolTrailLine>,
    pub tab_completion_items: Vec<String>,
    pub tab_completion_index: usize,
    pub tab_completion_original: String,
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
            last_enter_time: None,
            tool_trail: Vec::new(),
            tab_completion_items: Vec::new(),
            tab_completion_index: 0,
            tab_completion_original: String::new(),
        }
    }

    fn handle_submit(&mut self, app: &mut App) {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return;
        }

        if self.input.len() > 64 * 1024 {
            app.status = "Input too large, max 64KB".to_string();
            return;
        }

        if let Some(stamp) = self.last_enter_time {
            if stamp.elapsed().as_millis() < 150 {
                self.last_enter_time = None;
                return;
            }
        }
        self.last_enter_time = Some(Instant::now());

        match trimmed {
            "/quit" => {
                app.running = false;
                return;
            }
            "/clear" => {
                self.messages.clear();
                self.input.clear();
                self.cursor = 0;
                return;
            }
            "/new" => {
                // Request a new session from the agent
                if let Some(tx) = &app.input_tx {
                    let _ = tx.send(TuiEvent::ChatInput("start new session".into()));
                }
                self.input.clear();
                self.cursor = 0;
                return;
            }
            "/details" => {
                app.status = "Details: Use /session view to see all commands and options.".to_string();
                return;
            }
            "/theme" => {
                app.status = "Theme: Currently using dark theme. Configuration via ~/.config/obenalien/config.yaml.".to_string();
                return;
            }
            "/reasoning" => {
                // Toggle reasoning: append explicit instruction
                let tx_ref = app.input_tx.clone();
                if let Some(tx) = &tx_ref {
                    let _ = tx.send(TuiEvent::ChatInput(format!("{}\n\n[reasoning mode: please explain your step-by-step reasoning before responding]", self.input)));
                }
                return;
            }
            "/compact" => {
                app.status = "Compacting session context...".to_string();
                if let Some(tx) = &app.input_tx {
                    let _ = tx.send(TuiEvent::ChatInput("compact session".into()));
                }
                self.input.clear();
                self.cursor = 0;
                return;
            }
            "/todo" => {
                app.status = "TODO: No pending tasks. Tools can set TODO items via task output.".to_string();
                return;
            }
            "/session" => {
                let mut info = "Active session management:".to_string();
                if let Some(ref sid) = self.session_id {
                    info.push_str(&format!("\n  ID: {}", sid));
                }
                info.push_str("\n  Commands: /new (new session), /compact (compress context), /switch or press F2 for sessions list");
                app.status = info;
                self.input.clear();
                self.cursor = 0;
                return;
            }
            "/help" => {
                let help = "Slash commands:\
                    \n  /help        Show this help message\
                    \n  /clear       Clear chat messages\
                    \n  /quit        Exit TUI\
                    \n  /new         Start a new session\
                    \n  /session     Show session info\
                    \n  /compact     Compress current session context\
                    \n  /todo        Show pending tasks\
                    \n  /reasoning   Enable step-by-step reasoning mode\
                    \n  /details     Show available commands\
                    \n  /theme       Current theme info\
                    \n\nKeyboard:\
                    \n  Up/Down    Navigate input history\
                    \n  Ctrl+A     Move cursor to start\
                    \n  Ctrl+E     Move cursor to end\
                    \n  Ctrl+W     Delete word before cursor\
                    \n  Ctrl+K     Delete from cursor to end\
                    \n  Ctrl+U     Clear entire input\
                    \n  Ctrl+V     Paste from system clipboard\
                    \n  Alt+D      Delete next word\
                    \n  Ctrl+C     Exit TUI\
                    \n  F1-F4     Switch panels (Chat/Sessions/Config/Setup)";
                app.status = help.to_string();
                self.input.clear();
                self.cursor = 0;
                return;
            }
            _ => {}
        }

        if let Some(tx) = &app.input_tx {
            let _ = tx.send(TuiEvent::ChatInput(self.input.clone()));
        }
        app.input_history.append(&self.input);
        self.input.clear();
        self.cursor = 0;
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
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);
        draw_messages(frame, self, chunks[0]);
        draw_tool_trail(frame, self, chunks[0]);

        let input_text = format!("> {}", &self.input[self.cursor..]);
        let input_para = Paragraph::new(Text::from(input_text.as_str()))
            .block(Block::default().borders(Borders::ALL).title(" Input (Ctrl+W:del word, Ctrl+A/E:home/end, Ctrl+K:del-line) "));
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
            KeyCode::Up => {
                if let Some(new_text) = app.input_history.up(&self.input) {
                    self.input = new_text;
                    self.cursor = self.input.len();
                }
            }
            KeyCode::Down => {
                if let Some(new_text) = app.input_history.down() {
                    self.input = new_text;
                    self.cursor = self.input.len();
                }
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                self.handle_submit(app);
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
                self.last_enter_time = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor = 0;
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor > 0 {
                    let prefix = &self.input[..self.cursor];
                    if let Some(word_start) = prefix
                        .char_indices()
                        .rev()
                        .find(|(_, c)| !c.is_whitespace() && !c.is_alphanumeric())
                        .map(|(i, _)| i + 1)
                        .or_else(|| {
                            prefix
                                .char_indices()
                                .rev()
                                .find(|(_, c)| c.is_whitespace())
                                .map(|(i, _)| i + 1)
                        }) {
                        self.input.drain(word_start..self.cursor);
                        self.cursor = word_start;
                    } else {
                        self.input.drain(0..self.cursor);
                        self.cursor = 0;
                    }
                    self.last_enter_time = None;
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.input.len();
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.truncate(self.cursor);
                self.last_enter_time = None;
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.cursor < self.input.len() {
                    let after = &self.input[self.cursor..];
                    let truncated = if let Some(sp) = after.find(|c: char| c.is_whitespace()) {
                        self.cursor + sp
                    } else {
                        self.input.len()
                    };
                    self.input.drain(self.cursor..truncated);
                    self.last_enter_time = None;
                }
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    if !text.is_empty() {
                        self.input.insert_str(self.cursor, &text);
                        self.cursor += text.len();
                        self.last_enter_time = None;
                    }
                }
            }
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::SHIFT) && key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+Shift+M: cycle backwards
                self.cycle_tab(false);
            }
            KeyCode::Esc => {
                // Escape: clear completion overlay and restore original input
                if !self.tab_completion_items.is_empty() && self.tab_completion_index == 0 {
                    self.input = self.tab_completion_original.clone();
                    self.cursor = self.input.len();
                    self.tab_completion_items.clear();
                    self.tab_completion_index = 0;
                }
            }
            _ => {}
        }

        // Update completions on any char input
        if matches!(key.code, KeyCode::Char(_)) && key.modifiers == KeyModifiers::NONE {
            self.update_completions();
        }
    }
}

impl ChatPanel {
    /// Handle bracket-paste escape sequences (ESC [ ? 2004 h / l).
    ///
    /// Terminals send `\x1b[?2004h` at paste start and `\x1b[?2004l` at paste
    /// end.  Any chars arriving after the start sequence are buffered as raw
    /// text instead of being processed key-by-key.
    pub fn handle_bracket_paste(&mut self, raw: &str) {
        self.input.push_str(raw);
        self.cursor = self.input.len();
        self.last_enter_time = None;
    }

    /// Update tab completion candidates based on current input prefix.
    fn update_completions(&mut self) {
        // Find the word before cursor (/command...)
        let text_before_cursor = if self.cursor > 0 {
            &self.input[..self.cursor]
        } else {
            ""
        };
        let last_word = text_before_cursor.split_whitespace().last().unwrap_or("");

        if last_word.starts_with('/') {
            // Filter slash commands that match the prefix
            let prefix = last_word.to_lowercase();
            let commands = [
                ("/clear", "Clear chat messages"),
                ("/compact", "Compress current session context"),
                ("/details", "Show available commands"),
                ("/help", "Show this help message"),
                ("/new", "Start a new session"),
                ("/quit", "Exit TUI"),
                ("/reasoning", "Enable step-by-step reasoning mode"),
                ("/session", "Show session info"),
                ("/theme", "Current theme info"),
                ("/todo", "Show pending tasks"),
            ];
            self.tab_completion_items = commands
                .iter()
                .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&prefix))
                .map(|(cmd, desc)| format!("{} — {}", cmd, desc))
                .collect();
            if !self.tab_completion_items.is_empty() {
                self.tab_completion_index = 0;
                self.tab_completion_original = self.input.clone();
            } else {
                self.tab_completion_items.clear();
            }
        } else {
            // Don't clear tab completion items when not in slash context,
            // just allow cycling
        }
    }

    /// Apply the current tab completion to input.
    fn apply_tab_completion(&mut self) {
        if self.tab_completion_items.is_empty() { return; }
        let entry = &self.tab_completion_items[self.tab_completion_index];
        // Extract command name from "cmd — description"
        let cmd = entry.split_whitespace().next().unwrap_or("");
        // Replace the current word (from last whitespace or start to cursor)
        let text_before = &self.input[..self.cursor];
        let last_ws = text_before.rfind(|c: char| c.is_whitespace()).unwrap_or(0);
        let replacement = if last_ws == 0 && last_ws < self.input.len() {
            // No whitespace found — replace from 0
            format!("{}{}", cmd, &self.input[self.cursor..])
        } else {
            let start = if last_ws == 0 { 0 } else { last_ws + 1 };
            format!(
                "{}{}{}",
                &self.input[..start],
                cmd,
                &self.input[self.cursor..]
            )
        };
        self.input = replacement.clone();
        self.cursor = replacement.len();
    }

    /// Cycle the completion index.
    fn cycle_tab(&mut self, forward: bool) {
        if self.tab_completion_items.is_empty() { return; }
        if forward {
            self.tab_completion_index =
                (self.tab_completion_index + 1) % self.tab_completion_items.len();
        } else {
            if self.tab_completion_index == 0 {
                self.tab_completion_index = self.tab_completion_items.len() - 1;
            } else {
                self.tab_completion_index -= 1;
            }
        }
        self.apply_tab_completion();
    }

    /// Extract tool trail from session messages.
    /// Finds assistant messages with tool_calls and their corresponding tool results.
    pub fn extract_tool_trail(&mut self, messages: &[Message]) {
        if messages.is_empty() {
            self.tool_trail.clear();
            return;
        }

        // Helper: get text content from a Message
        let text_of = |msg: &Message| -> String {
            match &msg.content {
                oben_models::MessageContent::Text(s) => s.clone(),
                oben_models::MessageContent::Image { .. } => String::new(),
                oben_models::MessageContent::Parts(parts) => {
                    parts.iter().filter_map(|p| {
                        if let oben_models::MessagePart::Text(t) = p {
                            Some(t.clone())
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>().join("\n")
                }
            }
        };

        let mut trail: Vec<ToolTrailLine> = Vec::new();
        let mut pending: Vec<String> = Vec::new(); // pending tool names

        for msg in messages {
            if msg.role == oben_models::MessageRole::Assistant {
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        pending.push(tc.tool_name.clone());
                    }
                }
            } else if msg.role == oben_models::MessageRole::Tool {
                let output = text_of(msg);
                let has_error = output.to_lowercase().contains("error") || output.to_lowercase().contains("failed");
                let preview = if output.len() > 60 {
                    format!("{}...", &output[..60])
                } else {
                    output.clone()
                };

                if let Some(pos) = pending.iter().position(|name| {
                    output.contains(name.as_str()) || output.contains(&name[..name.len().min(20)])
                }) {
                    trail.push(ToolTrailLine {
                        status: if has_error { ToolTrailStatus::Error } else { ToolTrailStatus::Success },
                        tool_name: pending[pos].clone(),
                        output_preview: preview,
                    });
                    pending.remove(pos);
                } else if let Some(first_pending) = pending.first() {
                    // Fallback: match by tool id (tool_call_ids)
                    let tool_name = if let Some(id) = msg.tool_call_ids.first() {
                        first_pending.clone()
                    } else {
                        "unknown".into()
                    };
                    trail.push(ToolTrailLine {
                        status: if has_error { ToolTrailStatus::Error } else { ToolTrailStatus::Success },
                        tool_name,
                        output_preview: preview,
                    });
                }
            }
        }

        // Reverse to chronological order
        trail.reverse();
        if trail.len() > 5 {
            trail.drain(0..trail.len() - 5);
        }

        self.tool_trail = trail;
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

fn draw_tool_trail(frame: &mut Frame, panel: &ChatPanel, area: Rect) {
    if panel.tool_trail.is_empty() {
        return;
    }

    // Count tool trail lines needed
    let mut trail_lines: Vec<Line> = vec![
        Line::from(Span::styled(
            " ── Tool Trail ── ",
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        )),
    ];

    for line in &panel.tool_trail {
        let spinner_char = match line.status {
            ToolTrailStatus::Running(idx) => {
                let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                spinners[idx % spinners.len()]
            }
            _ => '✓',
        };

        let fg = match line.status {
            ToolTrailStatus::Running(_) => Color::Yellow,
            ToolTrailStatus::Success => Color::Green,
            ToolTrailStatus::Error => Color::Red,
        };

        let display_text = if line.output_preview.is_empty() {
            format!(" {} {}", spinner_char, line.tool_name)
        } else {
            format!(" {} {} ({})", spinner_char, line.tool_name, line.output_preview)
        };

        trail_lines.push(Line::from(Span::styled(
            display_text,
            Style::default().fg(fg),
        )));
    }

    // Limit trail area to 5 lines max
    let trail_height = if trail_lines.len() > 6 { 6 } else { trail_lines.len() };
    let h = area.height.min(trail_lines.len() as u16) - if trail_lines.len() > 1 { 1 } else { 0 };
    let trail_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(h),
        area.width,
        trail_height as u16,
    );

    let trail_para = Paragraph::new(Text::from(trail_lines));
    frame.render_widget(trail_para, trail_area);
}
