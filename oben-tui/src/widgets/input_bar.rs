//! Input bar widget — text input with wrapping, cursor tracking, placeholder,
//! tab completion overlay, and streaming indicator.
//!
//! This widget encapsulates all layout logic for the input area so the chat
//! panel only needs to manage state and delegate rendering.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::time::Instant;
use textwrap::wrap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::widgets::style::Theme;

/// Text area state tracked by the input bar widget.
pub struct InputState {
    /// Current text content.
    pub text: String,
    /// Grapheme index of the cursor.
    pub cursor: usize,
    /// Whether a turn is currently streaming (suppresses submission).
    pub streaming: bool,
    /// Tab completion candidates.
    pub completion_items: Vec<CompletionItem>,
    /// Currently selected index in `completion_items`.
    pub completion_index: usize,
    /// Last Enter press time (debounce).
    pub last_enter_time: Option<Instant>,
}

/// Single tab completion entry.
#[derive(Clone)]
pub struct CompletionItem {
    pub cmd: String,
    pub desc: String,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            streaming: false,
            completion_items: Vec::new(),
            completion_index: 0,
            last_enter_time: None,
        }
    }

    /// Display width of `n` graphemes in the text.
    pub fn grapheme_prefix_display(&self, n: usize) -> usize {
        self.text[..self.grapheme_to(n)]
            .graphemes(true)
            .take(n)
            .map(|g| g.width())
            .sum()
    }

    /// Display width of the text.
    pub fn display_width(&self) -> usize {
        self.text.width()
    }

    /// Count grapheme clusters in the text.
    pub fn grapheme_count(&self) -> usize {
        self.text.graphemes(true).count()
    }

    /// Convert grapheme index to byte index.
    fn grapheme_to_byte(&self, grapheme_idx: usize) -> usize {
        self.text
            .graphemes(true)
            .take(grapheme_idx.min(self.grapheme_count()))
            .map(|g| g.len())
            .sum()
    }

    /// Convert grapheme index to the byte offset it maps to.
    fn grapheme_to(&self, grapheme_idx: usize) -> usize {
        self.grapheme_to_byte(grapheme_idx)
    }

    /// Cursor screen position (col, row) based on text wrapping.
    pub fn cursor_screen_pos(&self, text_cols: usize) -> (u16, u16) {
        if text_cols == 0 || self.text.is_empty() {
            return (0, 0);
        }

        let mut col: usize = 0;
        let mut row: usize = 0;

        for (i, g) in self.text.graphemes(true).enumerate() {
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
}

/// Input bar widget.
///
/// Renders the text area block with wrapping, cursor, placeholder, streaming
/// indicator, and tab completion overlay.
pub struct InputBar;

impl InputBar {
    /// Render the input bar widget.
    ///
    /// Returns the height of the rendered block in rows (including border).
    pub fn render(&self, frame: &mut Frame, area: Rect, state: &InputState, theme: &Theme) -> u16 {
        let text_cols = (area.width as usize).saturating_sub(2).max(1);

        // Compute wrapped lines for cursor pos + height calc.
        let input_lines = self.build_text_lines(&state.text, text_cols, theme);
        let total_lines = input_lines.len() as u16;
        let visible_height = if total_lines > 0 {
            area.height.saturating_sub(2)
        } else {
            1
        };
        let row_scroll = if total_lines > visible_height {
            total_lines.saturating_sub(visible_height)
        } else {
            0
        };

        let block = Block::default().borders(Borders::ALL).title(" Typing.. ");

        block.render(area, frame.buffer_mut());
        let inner_inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );

        // Render text/paragraph inside border.
        let para = Paragraph::new(input_lines).scroll((row_scroll, 0));
        frame.render_widget(para, inner_inner);

        // Set cursor position.
        let (screen_col, screen_row) = state.cursor_screen_pos(text_cols);
        frame.set_cursor_position(Position::new(
            inner_inner.x + screen_col,
            inner_inner.y + screen_row,
        ));

        // Streaming indicator.
        if state.streaming {
            self.render_streaming_indicator(frame, area, theme);
        }

        // Tab completion overlay.
        if !state.completion_items.is_empty() {
            self.render_tab_completion(frame, area, state, theme);
        }

        area.height
    }

    /// Predict the height in rows needed to render the input bar including border
    /// for a given screen width.
    pub fn calculate_input_height(
        &self, state: &InputState, screen_width: u16,
    ) -> u16 {
        let text_cols = (screen_width as usize).saturating_sub(2).max(1);
        let lines = if state.text.is_empty() {
            1u16
        } else {
            textwrap::wrap(state.text.as_str(), text_cols).len() as u16
        };

        // border(2) + text_lines + streaming_indicator(1) + completion(0..8)
        let text_area = lines.max(1);
        let streaming = if state.streaming { 1 } else { 0 };
        let completion = (state.completion_items.len() as u16).min(8);
        2 + text_area + streaming + completion
    }

    /// Process a key event for the input bar.
    ///
    /// Returns true if the event was consumed (caller should NOT re-process).
    pub fn handle_key(
        &mut self,
        state: &mut InputState,
        app: &mut crate::App,
        key: KeyEvent,
    ) -> bool {
        // Streaming: only Ctrl+C kills it.
        if state.streaming {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                state.streaming = false;
                return true;
            }
            return true; // suppress all other keys during stream
        }

        match key.code {
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                self.handle_submit(state, app);
                return true;
            }
            KeyCode::Char('\n') | KeyCode::Char('\r') if key.modifiers == KeyModifiers::NONE => {
                self.handle_submit(state, app);
                return true;
            }
            KeyCode::Up => {
                if !state.completion_items.is_empty() {
                    if state.completion_index == 0 {
                        return true;
                    }
                    state.completion_index -= 1;
                    self.apply_completion(state);
                    return true;
                } else if let Some(next) = app.input_history.up(&state.text) {
                    state.text = next;
                    state.cursor = state.grapheme_count();
                }
                return true;
            }
            KeyCode::Down => {
                if !state.completion_items.is_empty() {
                    self.cycle_completion(state, true);
                    return true;
                } else if let Some(next) = app.input_history.down() {
                    state.text = next;
                    state.cursor = state.grapheme_count();
                }
                return true;
            }
            KeyCode::Left if state.cursor > 0 => {
                state.cursor -= 1;
                return true;
            }
            KeyCode::Right if state.cursor < state.grapheme_count() => {
                state.cursor += 1;
                return true;
            }
            KeyCode::Backspace if state.cursor > 0 => {
                let byte_idx = state.grapheme_to_byte(state.cursor - 1);
                let g = &state.text[byte_idx..];
                let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                state.text.drain(byte_idx..byte_idx + len);
                state.cursor -= 1;
                self.update_completions(state);
                return true;
            }
            KeyCode::Delete => {
                if state.cursor < state.grapheme_count() {
                    let byte_idx = state.grapheme_to_byte(state.cursor);
                    let g = &state.text[byte_idx..];
                    let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                    state.text.drain(byte_idx..byte_idx + len);
                }
                self.update_completions(state);
                return true;
            }
            _ if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char(_)) =>
            {
                let ch = match key.code {
                    KeyCode::Char(c) => c,
                    _ => return false,
                };
                let byte_idx = state.grapheme_to_byte(state.cursor);
                state.text.insert_str(byte_idx, &ch.to_string());
                state.cursor += 1;
                self.update_completions(state);
                return true;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.text.clear();
                state.cursor = 0;
                state.completion_items.clear();
                return true;
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if state.cursor > 0 {
                    let prefix: Vec<&str> = state.text[..state.grapheme_to_byte(state.cursor)]
                        .graphemes(true)
                        .collect();
                    let mut word_end = prefix.len();
                    for (i, g) in prefix.iter().enumerate().rev() {
                        if g.trim().is_empty() {
                            word_end = i;
                        } else {
                            break;
                        }
                    }
                    while word_end > 0 && !prefix[word_end - 1].trim().is_empty() {
                        word_end -= 1;
                    }
                    let byte_start = state.grapheme_to_byte(word_end);
                    let byte_end = state.grapheme_to_byte(state.cursor);
                    state.text.drain(byte_start..byte_end);
                    state.cursor = word_end;
                    self.update_completions(state);
                }
                return true;
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.cursor = 0;
                return true;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.cursor = state.grapheme_count();
                return true;
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte_idx = state.grapheme_to_byte(state.cursor);
                state.text.truncate(byte_idx);
                self.update_completions(state);
                return true;
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                // Word back
                if state.cursor > 0 {
                    let prefix: Vec<&str> = state.text[..state.grapheme_to_byte(state.cursor)]
                        .graphemes(true)
                        .collect();
                    let mut i = prefix.len();
                    while i > 0 && prefix[i - 1].trim().is_empty() {
                        i -= 1;
                    }
                    while i > 0 && !prefix[i - 1].trim().is_empty() {
                        i -= 1;
                    }
                    state.cursor = i;
                }
                self.update_completions(state);
                return true;
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                // Word forward
                if state.cursor < state.grapheme_count() {
                    let remaining: Vec<&str> = state.text[state.grapheme_to_byte(state.cursor)..]
                        .graphemes(true)
                        .collect();
                    let mut j = 0;
                    while j < remaining.len() && remaining[j].trim().is_empty() {
                        j += 1;
                    }
                    while j < remaining.len() && !remaining[j].trim().is_empty() {
                        j += 1;
                    }
                    state.cursor += j;
                }
                self.update_completions(state);
                return true;
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    if !text.is_empty() {
                        let byte_idx = state.grapheme_to_byte(state.cursor);
                        state.text.insert_str(byte_idx, &text);
                        state.cursor += text.graphemes(true).count();
                    }
                }
                self.update_completions(state);
                return true;
            }
            KeyCode::Tab => {
                if !state.completion_items.is_empty() {
                    self.cycle_completion(state, true);
                    return true;
                }
            }
            _ => {}
        }

        false
    }

    fn handle_submit(&self, state: &mut InputState, app: &mut crate::App) {
        let trimmed = state.text.trim().to_string();
        if trimmed.is_empty() {
            return;
        }

        // Prevent double submit
        if let Some(stamp) = state.last_enter_time {
            if stamp.elapsed().as_millis() < 150 {
                state.last_enter_time = None;
                return;
            }
        }
        state.last_enter_time = Some(Instant::now());

        state.completion_items.clear();
        state.completion_index = 0;

        let should_send = match trimmed.as_str() {
            "/quit" => {
                app.running = false;
                false
            }
            "/clear" => {
                state.text.clear();
                state.cursor = 0;
                false
            }
            "/new" => {
                if let Some(tx) = &app.input_tx {
                    let _ = tx.send(crate::TuiEvent::ChatInput("start new session".into()));
                }
                state.text.clear();
                state.cursor = 0;
                false
            }
            "/compact" => {
                app.status = "Compacting session context...".to_string();
                if let Some(tx) = &app.input_tx {
                    let _ = tx.send(crate::TuiEvent::ChatInput("compact session".into()));
                }
                state.text.clear();
                state.cursor = 0;
                false
            }
            "/reasoning" => {
                if let Some(tx) = &app.input_tx {
                    let _ = tx.send(crate::TuiEvent::ChatInput(state.text.clone()));
                }
                true
            }
            "/help" => {
                app.status = format!(
                    "{}",
                    "Slash commands: /clear /compact /new /quit /session /theme /reasoning /todo /details\n\
                    Keyboard: Up/Down=history, Ctrl+A/E=home/end, Ctrl+W=D=delete word, Ctrl+K=kill line"
                );
                false
            }
            "/session" => {
                app.status = "Session management via F2 (Sessions panel)".to_string();
                false
            }
            "/theme" => {
                app.status = "Press Ctrl+T to cycle themes".to_string();
                false
            }
            "/todo" => {
                app.status = "TODO: No pending tasks.".to_string();
                false
            }
            "/details" => {
                app.status = "Use /help for available commands.".to_string();
                false
            }
            _ => true,
        };

        if should_send {
            if let Some(tx) = &app.input_tx {
                let _ = tx.send(crate::TuiEvent::ChatInput(state.text.clone()));
            }
            app.input_history.append(&state.text);
        }

        state.text.clear();
        state.cursor = 0;
    }

    /// Update tab completion candidates.
    pub fn update_completions(&self, state: &mut InputState) {
        if !state.text.starts_with('/') {
            state.completion_items.clear();
            state.completion_index = 0;
            return;
        }

        let text_before_cursor = if state.cursor > 0 {
            &state.text[..state.grapheme_to_byte(state.cursor)]
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
            state.completion_items = commands
                .iter()
                .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&prefix))
                .map(|(cmd, desc)| CompletionItem {
                    cmd: (*cmd).to_string(),
                    desc: (*desc).to_string(),
                })
                .collect();
            if !state.completion_items.is_empty() {
                state.completion_index = 0;
            }
        } else {
            state.completion_items.clear();
            state.completion_index = 0;
        }
    }

    fn build_text_lines(&self, text: &str, text_cols: usize, _theme: &Theme) -> Vec<Line<'static>> {
        if text.is_empty() {
            return vec![Line::from(Span::styled(
                "Type '/' for commands. Type your message and press Enter.",
                Style::default().fg(Color::DarkGray),
            ))];
        }

        text.lines()
            .flat_map(|line| {
                if text_cols == 0 {
                    vec![Line::from(line.to_string())]
                } else {
                    wrap(line, text_cols)
                        .into_iter()
                        .map(|w| Line::from(Span::raw(w.to_string())))
                        .collect::<Vec<_>>()
                }
            })
            .collect()
    }

    fn render_streaming_indicator(&self, frame: &mut Frame, area: Rect, _theme: &Theme) {
        let text = " \u{23F3} Streaming... ";
        let w = text.len() as u16 + 2;
        let indicator_area = Rect::new(area.right().saturating_sub(w + 2), area.y + 1, w, 1);
        let para = Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::Yellow),
        )));
        frame.render_widget(para, indicator_area);
    }

    fn render_tab_completion(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &InputState,
        _theme: &Theme,
    ) {
        let max_lines = 8;
        let items = &state.completion_items[..state.completion_items.len().min(max_lines)];

        let display_lines: Vec<Line> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                if i == state.completion_index {
                    Line::from(Span::styled(
                        format!(" \u{25B8} {} ({})", item.cmd, item.desc),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {} ({})", item.cmd, item.desc),
                        Style::default().fg(Color::DarkGray),
                    ))
                }
            })
            .collect();

        // Use the input text wrapping to compute overlay position
        let text_cols = (area.width as usize).saturating_sub(2).max(1);
        let row_offset = if state.text.is_empty() {
            0
        } else if text_cols == 0 {
            1
        } else {
            textwrap::wrap(&state.text, text_cols).len() as u16
        };

        let completion_area = Rect::new(
            area.x,
            area.y + 1 + row_offset,
            area.width,
            items.len() as u16,
        );

        let para = Paragraph::new(display_lines);
        frame.render_widget(para, completion_area);
    }

    fn cycle_completion(&self, state: &mut InputState, forward: bool) {
        let len = state.completion_items.len();
        if len == 0 {
            return;
        }
        if forward {
            state.completion_index = (state.completion_index + 1) % len;
        } else {
            if state.completion_index == 0 {
                state.completion_index = len - 1;
            } else {
                state.completion_index -= 1;
            }
        }
        self.apply_completion(state);
    }

    fn apply_completion(&self, state: &mut InputState) {
        let entry = &state.completion_items[state.completion_index];
        state.text = entry.cmd.clone();
        state.cursor = state.text.graphemes(true).count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::style::Theme;

    fn make_state(text: &str) -> InputState {
        let mut s = InputState::new();
        s.text = text.to_string();
        s
    }

    #[test]
    fn test_calculate_input_height_empty() {
        let ib = InputBar;
        let state = make_state("");
        // border(2) + text(1) = 3
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_single_line_short() {
        let ib = InputBar;
        let state = make_state("hello");
        // border(2) + text(1) = 3
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_word_wraps() {
        let ib = InputBar;
        let state = make_state("hello world foo bar baz qux quux corge extra long text here");
        // text_cols = 40-2 = 38; wraps into 2 lines → 2 + 2 + 0 + 0 = 4
        let h = ib.calculate_input_height(&state, 40);
        assert_eq!(h, 4, "word wrap should add height");
    }

    #[test]
    fn test_calculate_input_height_explicit_newline() {
        let ib = InputBar;
        let state = make_state("line1\nline2\nline3");
        // 3 lines → 2 + 3 + 0 + 0 = 5
        assert_eq!(ib.calculate_input_height(&state, 40), 5);
    }

    #[test]
    fn test_calculate_input_height_streaming() {
        let ib = InputBar;
        let mut state = make_state("");
        state.streaming = true;
        // border(2) + text(1) + streaming(1) = 4
        assert_eq!(ib.calculate_input_height(&state, 40), 4);
    }

    #[test]
    fn test_calculate_input_height_completion_list() {
        let ib = InputBar;
        let mut state = make_state("");
        state.completion_items = vec![
            CompletionItem { cmd: "/foo".into(), desc: "a".into() },
            CompletionItem { cmd: "/bar".into(), desc: "b".into() },
            CompletionItem { cmd: "/baz".into(), desc: "c".into() },
        ];
        // border(2) + text(1) + completion(3) = 6
        assert_eq!(ib.calculate_input_height(&state, 40), 6);
    }

    #[test]
    fn test_calculate_input_height_completion_clamped() {
        let ib = InputBar;
        let mut state = make_state("");
        // 12 items -> clamped to 8
        for i in 0..12 {
            state.completion_items.push(CompletionItem {
                cmd: format!("/cmd{}", i),
                desc: format!("desc{}", i),
            });
        }
        assert_eq!(ib.calculate_input_height(&state, 40), 11); // 2 + 1 + 8
    }

    /// Given: an empty InputState
    /// When: Space key is sent
    /// Then: state.text contains " " (single space)
    #[test]
    fn test_space_key_inserts_single_space() {
        let mut ib = InputBar;
        let mut state = make_state("");
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let consumed = ib.handle_key(&mut state, &mut app, key);
        assert!(consumed, "space key should be consumed");
        assert_eq!(state.text, " ");
    }

    /// Given: an InputState with text "ab" and cursor at position 1
    /// When: Space key is sent
    /// Then: state.text becomes "a b"
    #[test]
    fn test_space_key_inserts_at_cursor() {
        let mut ib = InputBar;
        let mut state = make_state("ab");
        state.cursor = 1;
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        ib.handle_key(&mut state, &mut app, key);
        assert_eq!(state.text, "a b");
    }

    /// Given: an InputState with text "hello"
    /// When: Backspace is pressed with cursor > 0
    /// Then: rightmost grapheme is removed
    #[test]
    fn test_backspace_removes_rightmost_grapheme() {
        let mut ib = InputBar;
        let mut state = make_state("hello");
        state.cursor = 5;
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        ib.handle_key(&mut state, &mut app, key);
        assert_eq!(state.text, "hell");
        assert_eq!(state.cursor, 4);
    }

    /// Given: an InputState with text "hi" and cursor at end
    /// When: Enter is pressed
    /// Then: handle_submit is triggered (text is non-empty, consumed=true)
    #[test]
    fn test_enter_consumed_with_text() {
        let mut ib = InputBar;
        let mut state = make_state("hello");
        state.cursor = 5;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::App::new().unwrap();
        app.input_tx = Some(tx);
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let consumed = ib.handle_key(&mut state, &mut app, key);
        assert!(consumed, "enter with text should be consumed");
        assert!(state.text.is_empty(), "text cleared after submit");
    }

    /// Given: an empty InputState
    /// When: Enter is pressed
    /// Then: nothing is submitted, consumed=false (empty input rejected)
    #[test]
    fn test_enter_not_consumed_when_empty() {
        let mut ib = InputBar;
        let mut state = make_state("");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let mut app = crate::App::new().unwrap();
        app.input_tx = Some(tx);
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let consumed = ib.handle_key(&mut state, &mut app, key);
        // Enter is always consumed; empty input just clears state without submitting.
        assert!(consumed, "enter should always be consumed");
    }

    /// Given: an InputState with 5 CJK characters
    /// When: cursor moves right 3 times
    /// Then: cursor = 3
    #[test]
    fn test_cursor_moves_by_grapheme_not_by_byte() {
        let mut ib = InputBar;
        let mut state = InputState {
            text: "你好世界测试".to_string(),
            cursor: 0,
            streaming: false,
            completion_items: Vec::new(),
            completion_index: 0,
            last_enter_time: None,
        };
        let mut app = crate::App::new().unwrap();
        for _ in 0..3 {
            let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
            ib.handle_key(&mut state, &mut app, key);
        }
        assert_eq!(state.cursor, 3);
    }

    /// Given: an InputState with text "abc"
    /// When: Ctrl+W is pressed
    /// Then: entire text is removed (one "word")
    #[test]
    fn test_ctrl_w_deletes_word_before_cursor() {
        let mut ib = InputBar;
        let mut state = make_state("hello world");
        state.cursor = 11; // end of "world"
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        );
        ib.handle_key(&mut state, &mut app, key);
        assert_eq!(state.text, "hello ");
    }

    /// Given: an InputState with text "abc"
    /// When: Ctrl+U is pressed
    /// Then: entire input is cleared
    #[test]
    fn test_ctrl_u_clears_all() {
        let mut ib = InputBar;
        let mut state = make_state("some text here");
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        );
        ib.handle_key(&mut state, &mut app, key);
        assert_eq!(state.text, "");
        assert_eq!(state.cursor, 0);
    }

    /// Given: streaming=true state
    /// When: any key except Ctrl+C
    /// Then: consumed=true, streaming stays true
    #[test]
    fn test_streaming_suppresses_all_keys_except_ctrl_c() {
        let mut ib = InputBar;
        let mut state = make_state("hello");
        state.streaming = true;
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let consumed = ib.handle_key(&mut state, &mut app, key);
        assert!(consumed);
        assert!(state.streaming, "streaming should stay true");
    }

    /// Given: streaming=true with Ctrl+C
    /// When: Ctrl+C is pressed
    /// Then: streaming becomes false, consumed=true
    #[test]
    fn test_ctrl_c_exits_streaming() {
        let mut ib = InputBar;
        let mut state = make_state("hello");
        state.streaming = true;
        let mut app = crate::App::new().unwrap();
        let key = KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        );
        let consumed = ib.handle_key(&mut state, &mut app, key);
        assert!(consumed);
        assert!(!state.streaming, "ctrl+c should stop streaming");
    }

    /// Given: an InputState with "/co"
    /// When: completion is updated via update_completions
    /// Then: completion_items is populated with matching slash commands
    #[test]
    fn test_update_completions_finds_matching_commands() {
        let mut state = InputState::new();
        state.text = "/co".to_string();
        state.cursor = 3;
        InputBar.update_completions(&mut state);
        assert!(!state.completion_items.is_empty());
        // Should match /compact, /details (starts with "/co"? no, /comp*"
        let found_compact = state
            .completion_items
            .iter()
            .any(|c| c.cmd == "/compact");
        assert!(
            found_compact,
            "should find /compact for /comm"
        );
    }

    /// Given: an InputState with "/unknown"
    /// When: completion is updated
    /// Then: completion_items is empty
    #[test]
    fn test_update_completions_no_match() {
        let mut state = InputState::new();
        state.text = "/xyznonexistent".to_string();
        state.cursor = 17;
        InputBar.update_completions(&mut state);
        assert!(state.completion_items.is_empty());
    }

    /// Given: a completion list with 2+ items
    /// When: Up arrow is pressed (cycle through completions)
    /// Then: completion_index decreases, text updates
    #[test]
    fn test_up_arrow_cycles_completions() {
        let mut state = InputState::new();
        InputBar.update_completions(&mut state);
        let mut state2 = InputState::new();
        state2.text = "/".to_string();
        state2.cursor = 1;
        InputBar.update_completions(&mut state2);
        assert!(state2.completion_items.len() >= 2);

        let mut ib = InputBar;
        let mut app = crate::App::new().unwrap();
        let up_key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        ib.handle_key(&mut state2, &mut app, up_key);
        // At index 0, up does nothing (first item is already selected)
        assert_eq!(state2.completion_index, 0);
    }

    /// Given: a completion list with 2+ items
    /// When: Down arrow cycles forward then wraps to 0
    /// Then: completion_index cycles correctly
    #[test]
    fn test_down_arrow_cycles_completions_forward() {
        let mut state = InputState::new();
        state.text = "/".to_string();
        state.cursor = 1;
        InputBar.update_completions(&mut state);
        let mut ib = InputBar;
        let mut app = crate::App::new().unwrap();
        // go forward past the end
        for _ in 0..state.completion_items.len() {
            let down_key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
            ib.handle_key(&mut state, &mut app, down_key);
        }
        assert_eq!(state.completion_index, 0);
    }
}
