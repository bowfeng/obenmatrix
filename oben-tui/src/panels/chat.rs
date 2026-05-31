//! Chat panel — message history, streaming, input bar, tool call display.

use super::Panel;
use crate::{App, TuiEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use tracing;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};
use textwrap::wrap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use tui_scrollview::{ScrollView, ScrollViewState};

use oben_models::Message;
use std::sync::Mutex;
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
    pub scroll_state: Mutex<ScrollViewState>,
    pub view_mode: ChatViewMode,
    pub streaming: bool,
    pub session_id: Option<String>,
    pub streaming_text: String,
    pub last_enter_time: Option<Instant>,
    pub tool_trail: Vec<ToolTrailLine>,
    pub tab_completion_items: Vec<String>,
    pub tab_completion_index: usize,
    pub tab_completion_original: String,
    pub stream_info: String,
    pub turn_state_ref: Option<std::sync::Arc<std::sync::Mutex<crate::turn::event::TurnState>>>,
}

impl ChatPanel {
    /** Display width of input (1 CJK char = 1 column). */
    pub fn display_width(&self) -> usize {
        self.input.width()
    }

    /** Count grapheme clusters in input. */
    pub fn grapheme_count(&self) -> usize {
        self.input.graphemes(true).count()
    }

    // Convert grapheme index (cursor) to terminal screen position (col, row)
    // matching how the text is rendered (fold-based wrapping).
    pub fn cursor_screen_pos(&self, text_cols: usize) -> (u16, u16) {
        if text_cols == 0 || self.input.is_empty() {
            return (0, 0);
        }
        
        let mut col: usize = 0;
        let mut row: usize = 0;
        
        for (i, g) in self.input.graphemes(true).enumerate() {
            let g_width = g.width();
            
            if g == "\n" {
                col = 0;
                row += 1;
            } else if i < self.cursor {
                if col + g_width > text_cols {
                    row += 1;
                    col = g_width;
                } else {
                    col += g_width;
                }
            }
        }
        
        (col as u16, row as u16)
    }

    /** Display width of `n` graphemes in input. */
    pub fn grapheme_prefix_display(&self, n: usize) -> usize {
        self.input.graphemes(true)
            .take(n)
            .map(|g| g.width())
            .sum()
    }

    /** Convert grapheme index (cursor) to byte index for string slicing. */
    fn grapheme_to_byte(&self, grapheme_idx: usize) -> usize {
        self.input
            .graphemes(true)
            .take(grapheme_idx.min(self.grapheme_count()))
            .map(|g| g.len())
            .sum()
    }

    pub fn new(session_id: Option<String>, messages: Option<Vec<Message>>) -> Self {
        let chat_messages = messages
            .map(|msgs| msgs.iter().map(to_chat_msg).collect())
            .unwrap_or_default();
        Self {
            messages: chat_messages,
            input: String::new(),
            cursor: 0,
            scroll: 0,
            scroll_state: Mutex::new(ScrollViewState::default()),
            view_mode: ChatViewMode::History,
            streaming: false,
            session_id,
            streaming_text: String::new(),
            last_enter_time: None,
            tool_trail: Vec::new(),
            tab_completion_items: Vec::new(),
            tab_completion_index: 0,
            tab_completion_original: String::new(),
            stream_info: String::new(),
            turn_state_ref: None,
        }
    }

    /// Update stream_info from turn state
    pub fn update_from_turn_state(&mut self, turn_state: &crate::turn::event::TurnState) {
        tracing::info!("[ChatPanel::update_from_turn_state] streaming_text.len={} active_tools={} phase={:?}", 
            turn_state.streaming_text.len(), turn_state.active_tools.len(), turn_state.phase);
        let mut parts = Vec::new();
        
        // Show active tool info
        let active = &turn_state.active_tools;
        if !active.is_empty() {
            let names: Vec<String> = active.iter()
                .take(2)
                .map(|t| format!("{} ({})", t.name, t.context.chars().take(30).collect::<String>()))
                .collect();
            parts.push(format!("🔧 {}", names.join(", ")).to_string());
        }
        
        // Show streaming text preview
        if !turn_state.streaming_text.is_empty() {
            let preview = if turn_state.streaming_text.len() > 100 {
                let s: String = turn_state.streaming_text.chars().take(100).collect();
                format!("{}...", s)
            } else {
                turn_state.streaming_text.clone()
            };
            parts.push(format!("💬 {}", preview));
        }
        
        if !parts.is_empty() {
            self.stream_info = parts.join("\n");
        } else {
            self.stream_info.clear();
        }
    }

    fn handle_submit(&mut self, app: &mut App) {
        let trimmed = self.input.trim();
        tracing::info!("ChatPanel::submit - input='{}', has_input_tx={}", trimmed, app.input_tx.is_some());

        if trimmed.is_empty() {
            tracing::info!("ChatPanel::submit - empty input, returning");
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

        // Clear tab completion on any submission
        self.tab_completion_items.clear();
        self.tab_completion_index = 0;
        self.tab_completion_original = String::new();

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
            tracing::info!("ChatPanel::submit - sending ChatInput: '{}'", self.input);
            let _ = tx.send(TuiEvent::ChatInput(self.input.clone()));
        } else {
            tracing::info!("ChatPanel::submit - WARNING: input_tx is None, NOT sending");
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
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let mut stream_text = String::new();
        if let Some(ref ts) = self.turn_state_ref {
            if let Ok(ts) = ts.lock() {
                if !ts.streaming_text.is_empty() {
                    stream_text = ts.streaming_text.clone();
                }
            }
        }
        tracing::info!("[ChatPanel::draw] stream_text.len={} messages.len={}", stream_text.len(), self.messages.len());

        let trail_count = if self.tool_trail.is_empty() { 0u16 } else { (self.tool_trail.len() + 1).min(7) as u16 };

        // Calculate input bar height based on wrapped text lines
        let input_area_width: u16 = if trail_count > 0 {
            area.width.saturating_sub(trail_count + 1)
        } else {
            area.width.saturating_sub(1)
        };
        let text_cols = (input_area_width as usize).saturating_sub(2).max(1);
        
        let full_wrapped_lines = if text_cols > 0 {
            wrap(self.input.as_str(), text_cols).len()
        } else {
            1
        };

        let wrapped_lines = full_wrapped_lines.min(50);
        
        let indicator_height: u16 = if self.streaming { 1 } else { 0 };
        let completion_height: u16 = if !self.tab_completion_items.is_empty() {
            self.tab_completion_items.len().min(8) as u16
        } else {
            0
        };
        
        let bottom_area_height: u16 = 2 + wrapped_lines as u16 + indicator_height + completion_height;
        let bottom_area_height = bottom_area_height.max(3);
        
        let chat_height = area.height.saturating_sub(bottom_area_height);
        let chat_height = chat_height.min(area.height);

        let chunks = Layout::vertical([
            Constraint::Length(chat_height),
            Constraint::Length(bottom_area_height),
        ])
        .split(area);
        
        let chat_area = chunks[0];
        let trail_area = if trail_count > 0 {
            Rect::new(chat_area.x, chat_area.y + chat_area.height - trail_count as u16, trail_count, trail_count)
        } else {
            Rect::new(0, 0, 0, 0)
        };

        let input_area = chunks[1];

        draw_messages(frame, self, chat_area, &stream_text);
        if !self.tool_trail.is_empty() && trail_count > 0 {
            draw_tool_trail(frame, self, trail_area);
        }
        let body_relative_y = chunks[0].y;
        let body_relative_height = chunks[0].height;
        draw_turn_status(frame, &self.stream_info, Rect::new(chunks[0].x, body_relative_y, chunks[0].width, body_relative_height));

        // Calculate cursor position
        let text_cols_actual = (input_area.width as usize).saturating_sub(2).max(1);
        let (screen_col, screen_row) = self.cursor_screen_pos(text_cols_actual);

        // Build input text lines using same folding logic as cursor_screen_pos
        let input_text_lines: Vec<Line> = if !self.input.is_empty() {
            self.input.as_str().graphemes(true).fold(Vec::new(), |mut lines, g| {
                if lines.is_empty() {
                    lines.push(String::from(g));
                } else {
                    let last = lines.last_mut().unwrap();
                    let current_line_width: usize = last.graphemes(true).map(|c| c.width()).sum();
                    if current_line_width + g.width() > text_cols_actual {
                        lines.push(String::from(g));
                    } else {
                        last.push_str(g);
                    }
                }
                lines
            }).into_iter().map(Line::from).collect()
        } else {
            vec![Line::from(Span::styled(
                "Type '/' to see available slash commands. Type your message and press Enter to send.",
                Style::default().fg(Color::DarkGray),
            ))]
        };
        
        let total_lines = input_text_lines.len() as u16;
        let visible_height = if total_lines > 0 { input_area.height.saturating_sub(2) } else { 1 };
        let row_scroll = if total_lines > visible_height {
            total_lines.saturating_sub(visible_height)
        } else {
            0
        };

        let input_para = Paragraph::new(input_text_lines)
            .scroll((row_scroll, 0))
            .block(Block::default().borders(Borders::ALL).title(" Typing.. "));
        frame.render_widget(input_para, input_area);

        frame.set_cursor_position(Position::new(
            input_area.x + 1 + screen_col,
            input_area.y + 1 + screen_row,
        ));

        // Draw streaming indicator
        if self.streaming {
            let indicator_text = " ⏳ Streaming... ".to_string();
            let indicator_span = Span::styled(
                indicator_text.clone(),
                Style::default().fg(Color::Yellow),
            );
            let indicator_para = Paragraph::new(Line::from(indicator_span));
            let indicator_area = Rect::new(
                chat_area.right() - indicator_text.len() as u16 - 2,
                chat_area.y + 1,
                indicator_text.len() as u16 + 2,
                1,
            );
            frame.render_widget(indicator_para, indicator_area);
        }

        // Draw tab completion overlay below input text
        if !self.tab_completion_items.is_empty() {
            let completion_text: Vec<Line> = self.tab_completion_items.iter().enumerate().map(|(i, entry)| {
                let (cmd, desc) = entry.split_once(" — ").unwrap_or((&entry[..], ""));
                if i == self.tab_completion_index {
                    Line::from(Span::styled(
                        format!(" ▸ {} ({})", cmd, desc),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {} ({})", cmd, desc),
                        Style::default().fg(Color::DarkGray),
                    ))
                }
            }).collect();

            let max_lines = 8;
            let display_lines = completion_text.iter().take(max_lines).cloned().collect::<Vec<_>>();
            let completion_para = Paragraph::new(display_lines);
            let completion_area = Rect::new(
                input_area.x,
                input_area.y + 1 + textwrap::wrap(&self.input, text_cols_actual).len() as u16,
                input_area.width,
                if completion_text.len() > max_lines { max_lines as u16 } else { completion_text.len() as u16 },
            );
            frame.render_widget(completion_para, completion_area);
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
                if !self.tab_completion_items.is_empty() {
                    if self.tab_completion_index == 0 {
                        return;
                    } else {
                        self.tab_completion_index -= 1;
                        self.apply_tab_completion();
                    }
                } else if let Some(new_text) = app.input_history.up(&self.input) {
                    self.input = new_text;
                    self.cursor = self.grapheme_count();
                } else {
                    self.scroll_state.lock().unwrap().scroll_down();
                }
            }
            KeyCode::Down => {
                if !self.tab_completion_items.is_empty() {
                    self.cycle_tab(true);
                } else if let Some(new_text) = app.input_history.down() {
                    self.input = new_text;
                    self.cursor = self.grapheme_count();
                } else {
                    self.scroll_state.lock().unwrap().scroll_up();
                }
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                self.handle_submit(app);
            }
            KeyCode::Left => {
                if self.cursor > 0 { self.cursor -= 1; }
            }
            KeyCode::Right => {
                if self.cursor < self.grapheme_count() {
                    self.cursor += 1;
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let byte_idx = self.grapheme_to_byte(self.cursor - 1);
                    let g = &self.input[byte_idx..];
                    let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                    self.input.drain(byte_idx..byte_idx + len);
                    self.cursor -= 1;
                }
                self.update_completions();
            }
            KeyCode::Delete => {
                if self.cursor < self.grapheme_count() {
                    let byte_idx = self.grapheme_to_byte(self.cursor);
                    let g = &self.input[byte_idx..];
                    let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                    self.input.drain(byte_idx..byte_idx + len);
                }
                self.update_completions();
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => {
                let byte_idx = self.grapheme_to_byte(self.cursor);
                self.input.insert(byte_idx, c);
                self.cursor += 1;
                self.last_enter_time = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor = 0;
                self.tab_completion_items.clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor > 0 {
                    let prefix_graphemes: Vec<&str> = self.input[..self.grapheme_to_byte(self.cursor)].graphemes(true).collect();
                    let mut word_end = prefix_graphemes.len();
                    for (i, g) in prefix_graphemes.iter().enumerate().rev() {
                        if g.trim().is_empty() { word_end = i; } else { break; }
                    }
                    while word_end > 0 && !prefix_graphemes[word_end - 1].trim().is_empty() {
                        word_end -= 1;
                    }
                    let byte_start = self.grapheme_to_byte(word_end);
                    let byte_end = self.grapheme_to_byte(self.cursor);
                    self.input.drain(byte_start..byte_end);
                    self.cursor = word_end;
                    self.last_enter_time = None;
                    self.update_completions();
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.grapheme_count();
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte_idx = self.grapheme_to_byte(self.cursor);
                self.input.truncate(byte_idx);
                self.last_enter_time = None;
                self.update_completions();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor > 0 {
                    let prefix_graphemes: Vec<&str> = self.input[..self.grapheme_to_byte(self.cursor)].graphemes(true).collect();
                    let mut word_end = prefix_graphemes.len();
                    for (i, g) in prefix_graphemes.iter().enumerate().rev() {
                        if g.trim().is_empty() { word_end = i; } else { break; }
                    }
                    while word_end > 0 && !prefix_graphemes[word_end - 1].trim().is_empty() {
                        word_end -= 1;
                    }
                    let byte_start = self.grapheme_to_byte(word_end);
                    let byte_end = self.grapheme_to_byte(self.cursor);
                    self.input.drain(byte_start..byte_end);
                    self.cursor = word_end;
                    self.last_enter_time = None;
                    self.update_completions();
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.grapheme_count();
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte_idx = self.grapheme_to_byte(self.cursor);
                self.input.truncate(byte_idx);
                self.last_enter_time = None;
                self.update_completions();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.cursor < self.grapheme_count() {
                    let remaining_graphemes: Vec<&str> = self.input[self.grapheme_to_byte(self.cursor)..].graphemes(true).collect();
                    let mut word_end = 0;
                    // Skip leading whitespace
                    for g in &remaining_graphemes {
                        if !g.trim().is_empty() { break; }
                        word_end += 1;
                    }
                    // Skip non-whitespace (word)
                    for g in &remaining_graphemes[word_end..] {
                        if g.trim().is_empty() { break; }
                        word_end += 1;
                    }
                    let byte_end = self.grapheme_to_byte(self.cursor + word_end);
                    self.input.drain(self.grapheme_to_byte(self.cursor)..byte_end);
                    self.last_enter_time = None;
                    self.update_completions();
                }
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.cursor > 0 {
                    let prefix_graphemes: Vec<&str> = self.input[..self.grapheme_to_byte(self.cursor)].graphemes(true).collect();
                    let mut i = prefix_graphemes.len();
                    // Skip backwards over whitespace
                    while i > 0 && prefix_graphemes[i - 1].trim().is_empty() { i -= 1; }
                    // Skip backwards over non-whitespace
                    while i > 0 && !prefix_graphemes[i - 1].trim().is_empty() { i -= 1; }
                    self.cursor = i;
                    self.update_completions();
                }
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.cursor < self.grapheme_count() {
                    let remaining: Vec<&str> = self.input[self.grapheme_to_byte(self.cursor)..].graphemes(true).collect();
                    let mut i = 0;
                    // Skip leading whitespace
                    while i < remaining.len() && remaining[i].trim().is_empty() { i += 1; }
                    // Skip word
                    while i < remaining.len() && !remaining[i].trim().is_empty() { i += 1; }
                    self.cursor += i;
                    self.update_completions();
                }
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    if !text.is_empty() {
                        let byte_idx = self.grapheme_to_byte(self.cursor);
                        self.input.insert_str(byte_idx, &text);
                        self.cursor += text.graphemes(true).count();
                        self.last_enter_time = None;
                    }
                }
            }
            KeyCode::Tab => {
                if !self.tab_completion_items.is_empty() {
                    self.cycle_tab(true);
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
        self.update_completions();
    }

    /// Update tab completion candidates based on current input prefix.
    /// Only triggers when '/' is at the very start of the input.
    fn update_completions(&mut self) {
        if !self.input.starts_with('/') {
            self.tab_completion_items.clear();
            self.tab_completion_index = 0;
            self.tab_completion_original = String::new();
            return;
        }

        let text_before_cursor = if self.cursor > 0 {
            &self.input[..self.grapheme_to_byte(self.cursor)]
        } else {
            ""
        };
        let last_word = text_before_cursor.split_whitespace().last().unwrap_or("");

        if last_word.starts_with('/') {
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
                self.tab_completion_index = 0;
                self.tab_completion_original = String::new();
            }
        } else {
            self.tab_completion_items.clear();
            self.tab_completion_index = 0;
            self.tab_completion_original = String::new();
        }
    }

    /// Apply the current tab completion to input.
    fn apply_tab_completion(&mut self) {
        if self.tab_completion_items.is_empty() { return; }
        let entry = &self.tab_completion_items[self.tab_completion_index];
        let cmd = entry.split_whitespace().next().unwrap_or("");
        
        let replacement = if self.input[..self.grapheme_to_byte(self.cursor)].trim().is_empty() {
            format!("{}{}", cmd, &self.input[self.grapheme_to_byte(self.cursor)..])
        } else {
            let before_graphemes: Vec<&str> = self.input[..self.grapheme_to_byte(self.cursor)].graphemes(true).collect();
            let last_ws = before_graphemes.iter().rposition(|g| g.trim().is_empty());
            let start = match last_ws {
                Some(pos) => self.grapheme_to_byte(pos + 1),
                None => 0,
            };
            format!(
                "{}{}{}",
                &self.input[..start],
                cmd,
                &self.input[self.grapheme_to_byte(self.cursor)..]
            )
        };
        self.input = replacement.clone();
        self.cursor = self.input.graphemes(true).count();
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
    pub fn extract_tool_trail(&mut self, messages: &[Message]) {
        if messages.is_empty() {
            self.tool_trail.clear();
            return;
        }

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
        let mut pending: Vec<String> = Vec::new();

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
                let preview = if output.chars().count() > 60 {
                    let truncated: String = output.chars().take(60).collect();
                    format!("{}...", truncated)
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
                    let tool_name = if msg.tool_call_ids.first().is_some() {
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

        trail.reverse();
        if trail.len() > 5 {
            trail.drain(0..trail.len() - 5);
        }

        self.tool_trail = trail;
    }
}

fn draw_messages(frame: &mut Frame, panel: &ChatPanel, area: Rect, streaming_text: &str) {
    let mut lines: Vec<Line> = Vec::new();

    // Compute messages to render: during streaming, exclude the last assistant message
    // which is replaced by streaming_text
    let mut messages_to_render = &panel.messages[..];
    if panel.streaming && !streaming_text.is_empty() {
        // Find and exclude the last assistant message
        for (i, msg) in panel.messages.iter().enumerate().rev() {
            if msg.role == "Assistant" {
                messages_to_render = &panel.messages[..i];
                break;
            }
        }
    }

    for msg in messages_to_render {
        lines.push(Line::from(Span::styled(
            format!(" ── {} ── ", msg.role),
            Style::default().fg(match msg.role.as_str() {
                "User" => Color::Green,
                "Assistant" => Color::Blue,
                "System" => Color::Magenta,
                "Tool" => Color::Yellow,
                _ => Color::Gray,
            }).add_modifier(Modifier::BOLD),
        )));
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

    // Streaming assistant text replaces the last assistant message
    if panel.streaming && !streaming_text.is_empty() {
        lines.push(Line::from(Span::styled(
            " ── Assistant ── ",
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        )));
        lines.extend(streaming_text.lines().map(Line::from));
        lines.push(Line::from(""));
    }

    // Render into ScrollView — full content, ScrollView handles viewing area
    let total_lines = lines.len();
    let content_size = ratatui::layout::Size::new(area.width, total_lines.max(1) as u16);
    let mut scroll_view = ScrollView::new(content_size);
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Messages "));
    scroll_view.render_widget(para, ratatui::layout::Rect::new(0, 0, content_size.width, content_size.height));

    let mut state = panel.scroll_state.lock().unwrap();
    state.set_offset(Position::new(0, panel.scroll as u16));
    frame.render_stateful_widget(scroll_view, area, &mut *state);
}

fn draw_tool_trail(frame: &mut Frame, panel: &ChatPanel, area: Rect) {
    if panel.tool_trail.is_empty() {
        return;
    }

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

    let _trail_height = (trail_lines.len() as u16).min(area.height);
    let trail_para = Paragraph::new(Text::from(trail_lines));
    frame.render_widget(trail_para, area);
}

fn draw_turn_status(frame: &mut Frame, stream_info: &str, body_area: Rect) {
    if stream_info.is_empty() {
        return;
    }

    let lines = stream_info.lines().collect::<Vec<_>>();
    let height = lines.len().min(3) as u16;
    if height == 0 {
        return;
    }

    let displayed_lines: Vec<Line> = lines.iter().take(3).map(|l| Line::from(*l)).collect();
    let para = Paragraph::new(displayed_lines);
    let area = Rect::new(
        body_area.x,
        body_area.y,
        body_area.width,
        height,
    );
    frame.render_widget(para, area);
}
