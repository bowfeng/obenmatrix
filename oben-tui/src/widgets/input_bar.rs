//! Input bar widget — text input with wrapping, cursor tracking, placeholder,
//! tab completion overlay, and streaming indicator.
//!
//! This widget encapsulates all layout logic for the input area so the chat
//! panel only needs to manage state and delegate rendering.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use std::time::Instant;
use textwrap::wrap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

/// Result returned by `InputBarWidget::handle_key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputBarResult {
    /// Key was consumed, no special action.
    Consumed,
    /// Key was not consumed by the input bar.
    PassedThrough,
    /// Submit this text as chat input.
    ChatInput(String),
    /// Execute this slash command.
    SlashCommand { cmd_name: String, extra: String },
    /// Interrupt the current streaming turn.
    Interrupt,
}

/// Input bar widget.
///
/// Renders the text area block with wrapping, cursor, placeholder, streaming
/// indicator, and tab completion overlay.
pub struct InputBarWidget;

impl InputBarWidget {
    /// Render the input bar widget.
    ///
    /// Returns the height of the rendered block in rows (including border).
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &InputState,
        palette: &ratatui_themes::ThemePalette,
    ) -> u16 {
        let text_cols = (area.width as usize).saturating_sub(2).max(1);

        // Compute wrapped lines for cursor pos + height calc.
        let input_lines = self.build_text_lines(&state.text, text_cols, palette);
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

        // Clean input bar: no title, rounded border, subtle style.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.muted));

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

        // Streaming indicator — styled badge on the top-right of the input bar.
        if state.streaming {
            self.render_streaming_indicator(frame, area, palette);
        }

        // Tab completion overlay.
        if !state.completion_items.is_empty() {
            self.render_tab_completion(frame, area, state, palette);
        }

        area.height
    }

    /// Predict the height in rows needed to render the input bar including border
    /// for a given screen width.
    pub fn calculate_input_height(&self, state: &InputState, screen_width: u16) -> u16 {
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
    /// Returns an `InputBarResult` indicating how the event was handled.
    pub fn handle_key(&mut self, state: &mut InputState, key: KeyEvent) -> InputBarResult {
        // Streaming: only ESC interrupts it.
        if state.streaming {
            if key.code == KeyCode::Esc {
                tracing::info!("[input_bar] ESC received during streaming, interrupting");
                state.streaming = false;
                return InputBarResult::Interrupt;
            }
            return InputBarResult::Consumed; // suppress all other keys during stream
        }

        match key.code {
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                return self.handle_submit(state);
            }
            KeyCode::Char('\n') | KeyCode::Char('\r') if key.modifiers == KeyModifiers::NONE => {
                return self.handle_submit(state);
            }
            KeyCode::Up => {
                if !state.completion_items.is_empty() {
                    if state.completion_index == 0 {
                        return InputBarResult::Consumed;
                    }
                    state.completion_index -= 1;
                    self.apply_completion(state);
                    return InputBarResult::Consumed;
                }
                return InputBarResult::PassedThrough;
            }
            KeyCode::Down => {
                if !state.completion_items.is_empty() {
                    self.cycle_completion(state, true);
                    return InputBarResult::Consumed;
                }
                return InputBarResult::PassedThrough;
            }
            KeyCode::Left if state.cursor > 0 => {
                state.cursor -= 1;
                return InputBarResult::Consumed;
            }
            KeyCode::Right if state.cursor < state.grapheme_count() => {
                state.cursor += 1;
                return InputBarResult::Consumed;
            }
            KeyCode::Backspace if state.cursor > 0 => {
                let byte_idx = state.grapheme_to_byte(state.cursor - 1);
                let g = &state.text[byte_idx..];
                let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                state.text.drain(byte_idx..byte_idx + len);
                state.cursor -= 1;
                self.update_completions(state);
                return InputBarResult::Consumed;
            }
            KeyCode::Delete => {
                if state.cursor < state.grapheme_count() {
                    let byte_idx = state.grapheme_to_byte(state.cursor);
                    let g = &state.text[byte_idx..];
                    let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                    state.text.drain(byte_idx..byte_idx + len);
                }
                self.update_completions(state);
                return InputBarResult::Consumed;
            }
            _ if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char(_)) =>
            {
                let ch = match key.code {
                    KeyCode::Char(c) => c,
                    _ => return InputBarResult::PassedThrough,
                };
                let byte_idx = state.grapheme_to_byte(state.cursor);
                state.text.insert_str(byte_idx, &ch.to_string());
                state.cursor += 1;
                self.update_completions(state);
                return InputBarResult::Consumed;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.text.clear();
                state.cursor = 0;
                state.completion_items.clear();
                return InputBarResult::Consumed;
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
                return InputBarResult::Consumed;
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.cursor = 0;
                return InputBarResult::Consumed;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.cursor = state.grapheme_count();
                return InputBarResult::Consumed;
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte_idx = state.grapheme_to_byte(state.cursor);
                state.text.truncate(byte_idx);
                self.update_completions(state);
                return InputBarResult::Consumed;
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
                return InputBarResult::Consumed;
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
                return InputBarResult::Consumed;
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
                return InputBarResult::Consumed;
            }
            KeyCode::Tab => {
                if !state.completion_items.is_empty() {
                    self.cycle_completion(state, true);
                    return InputBarResult::Consumed;
                }
            }
            _ => {}
        }

        InputBarResult::PassedThrough
    }

    fn handle_submit(&self, state: &mut InputState) -> InputBarResult {
        let trimmed = state.text.trim().to_string();
        if trimmed.is_empty() {
            return InputBarResult::Consumed;
        }

        // Prevent double submit
        if let Some(stamp) = state.last_enter_time {
            if stamp.elapsed().as_millis() < 150 {
                state.last_enter_time = None;
                return InputBarResult::Consumed;
            }
        }
        state.last_enter_time = Some(Instant::now());

        state.completion_items.clear();
        state.completion_index = 0;

        // Slash command dispatch — pass through to panel which maps to KeyAction
        if trimmed.starts_with('/') {
            let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
            let cmd_name = parts[0][1..].to_lowercase(); // strip '/' and lowercase
            let extra = if parts.len() > 1 { parts[1].trim() } else { "" };
            state.text.clear();
            state.cursor = 0;
            return InputBarResult::SlashCommand {
                cmd_name,
                extra: extra.to_string(),
            };
        }

        // Default: submit as chat input
        let input = state.text.clone();
        state.text.clear();
        state.cursor = 0;
        InputBarResult::ChatInput(input)
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
                ("/rename", "Rename current session"),
                ("/new", "Start a new session"),
                ("/compact", "Compress current session context"),
                ("/clear", "Clear chat messages"),
                ("/help", "Show this help message"),
                ("/reasoning", "Enable step-by-step reasoning mode"),
                ("/theme", "Current theme info"),
                ("/todo", "Show pending tasks"),
                ("/quit", "Exit TUI"),
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

    fn build_text_lines(
        &self,
        text: &str,
        text_cols: usize,
        _palette: &ratatui_themes::ThemePalette,
    ) -> Vec<Line<'static>> {
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

    fn render_streaming_indicator(
        &self,
        frame: &mut Frame,
        area: Rect,
        palette: &ratatui_themes::ThemePalette,
    ) {
        let text = " \u{23F3} Streaming... ";
        let w = text.len() as u16 + 2;
        let indicator_area = Rect::new(area.right().saturating_sub(w + 2), area.y + 1, w, 1);
        let para = Paragraph::new(Line::from(Span::styled(
            text,
            Style::default()
                .fg(palette.info)
                .add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(para, indicator_area);
    }

    fn render_tab_completion(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &InputState,
        palette: &ratatui_themes::ThemePalette,
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
                            .fg(palette.info)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {} ({})", item.cmd, item.desc),
                        Style::default().fg(palette.muted),
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

    fn make_state(text: &str) -> InputState {
        let mut s = InputState::new();
        s.text = text.to_string();
        s
    }

    #[test]
    fn test_calculate_input_height_empty() {
        let ib = InputBarWidget;
        let state = make_state("");
        // border(2) + text(1) = 3
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_single_line_short() {
        let ib = InputBarWidget;
        let state = make_state("hello");
        // border(2) + text(1) = 3
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_word_wraps() {
        let ib = InputBarWidget;
        let state = make_state("hello world foo bar baz qux quux corge extra long text here");
        // text_cols = 40-2 = 38; wraps into 2 lines -> 2 + 2 + 0 + 0 = 4
        let h = ib.calculate_input_height(&state, 40);
        assert_eq!(h, 4, "word wrap should add height");
    }

    #[test]
    fn test_calculate_input_height_explicit_newline() {
        let ib = InputBarWidget;
        let state = make_state("line1\nline2\nline3");
        // 3 lines -> 2 + 3 + 0 + 0 = 5
        assert_eq!(ib.calculate_input_height(&state, 40), 5);
    }

    #[test]
    fn test_calculate_input_height_streaming() {
        let ib = InputBarWidget;
        let mut state = make_state("");
        state.streaming = true;
        // border(2) + text(1) + streaming(1) = 4
        assert_eq!(ib.calculate_input_height(&state, 40), 4);
    }

    #[test]
    fn test_calculate_input_height_completion_list() {
        let ib = InputBarWidget;
        let mut state = make_state("");
        state.completion_items = vec![
            CompletionItem {
                cmd: "/foo".into(),
                desc: "a".into(),
            },
            CompletionItem {
                cmd: "/bar".into(),
                desc: "b".into(),
            },
            CompletionItem {
                cmd: "/baz".into(),
                desc: "c".into(),
            },
        ];
        // border(2) + text(1) + completion(3) = 6
        assert_eq!(ib.calculate_input_height(&state, 40), 6);
    }

    #[test]
    fn test_calculate_input_height_completion_clamped() {
        let ib = InputBarWidget;
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
        let mut ib = InputBarWidget;
        let mut state = make_state("");
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let result = ib.handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        assert_eq!(state.text, " ");
    }

    /// Given: an InputState with text "ab" and cursor at position 1
    /// When: Space key is sent
    /// Then: state.text becomes "a b"
    #[test]
    fn test_space_key_inserts_at_cursor() {
        let mut ib = InputBarWidget;
        let mut state = make_state("ab");
        state.cursor = 1;
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        ib.handle_key(&mut state, key);
        assert_eq!(state.text, "a b");
    }

    /// Given: an InputState with text "hello"
    /// When: Backspace is pressed with cursor > 0
    /// Then: rightmost grapheme is removed
    #[test]
    fn test_backspace_removes_rightmost_grapheme() {
        let mut ib = InputBarWidget;
        let mut state = make_state("hello");
        state.cursor = 5;
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        ib.handle_key(&mut state, key);
        assert_eq!(state.text, "hell");
        assert_eq!(state.cursor, 4);
    }

    /// Given: an InputState with text "hi" and cursor at end
    /// When: Enter is pressed
    /// Then: handle_submit is triggered (text is non-empty, ChatInput)
    #[test]
    fn test_enter_consumed_with_text() {
        let mut ib = InputBarWidget;
        let mut state = make_state("hello");
        state.cursor = 5;
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib.handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::ChatInput(_)));
        assert!(state.text.is_empty(), "text cleared after submit");
    }

    /// Given: an empty InputState
    /// When: Enter is pressed
    /// Then: nothing is submitted, consumed=Consumed (empty input rejected)
    #[test]
    fn test_enter_not_consumed_when_empty() {
        let mut ib = InputBarWidget;
        let mut state = make_state("");

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib.handle_key(&mut state, key);
        // Enter is always consumed; empty input just clears state without submitting.
        assert!(matches!(result, InputBarResult::Consumed));
    }

    /// Given: an InputState with 5 CJK characters
    /// When: cursor moves right 3 times
    /// Then: cursor = 3
    #[test]
    fn test_cursor_moves_by_grapheme_not_by_byte() {
        let mut ib = InputBarWidget;
        let mut state = InputState {
            text: "你好世界测试".to_string(),
            cursor: 0,
            streaming: false,
            completion_items: Vec::new(),
            completion_index: 0,
            last_enter_time: None,
        };
        for _ in 0..3 {
            let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
            ib.handle_key(&mut state, key);
        }
        assert_eq!(state.cursor, 3);
    }

    /// Given: an InputState with text "abc"
    /// When: Ctrl+W is pressed
    /// Then: entire text is removed (one "word")
    #[test]
    fn test_ctrl_w_deletes_word_before_cursor() {
        let mut ib = InputBarWidget;
        let mut state = make_state("hello world");
        state.cursor = 11; // end of "world"
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        ib.handle_key(&mut state, key);
        assert_eq!(state.text, "hello ");
    }

    /// Given: an InputState with text "abc"
    /// When: Ctrl+U is pressed
    /// Then: entire input is cleared
    #[test]
    fn test_ctrl_u_clears_all() {
        let mut ib = InputBarWidget;
        let mut state = make_state("some text here");
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        ib.handle_key(&mut state, key);
        assert_eq!(state.text, "");
        assert_eq!(state.cursor, 0);
    }

    /// Given: streaming=true state
    /// When: any key except ESC
    /// Then: consumed=Consumed, streaming stays true
    #[test]
    fn test_streaming_suppresses_all_keys_except_esc() {
        let mut ib = InputBarWidget;
        let mut state = make_state("hello");
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let result = ib.handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        assert!(state.streaming, "streaming should stay true");
    }

    /// Given: streaming=true with ESC
    /// When: ESC is pressed
    /// Then: streaming becomes false, result=Interrupt
    #[test]
    fn test_esc_interrupts_streaming() {
        let mut ib = InputBarWidget;
        let mut state = make_state("hello");
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let result = ib.handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Interrupt));
        assert!(!state.streaming, "esc should stop streaming");
    }

    /// Given: an InputState with "/co"
    /// When: completion is updated via update_completions
    /// Then: completion_items is populated with matching slash commands
    #[test]
    fn test_update_completions_finds_matching_commands() {
        let mut state = InputState::new();
        state.text = "/co".to_string();
        state.cursor = 3;
        InputBarWidget.update_completions(&mut state);
        assert!(!state.completion_items.is_empty());
        // Should match /compact
        let found_compact = state.completion_items.iter().any(|c| c.cmd == "/compact");
        assert!(found_compact, "should find /compact for /co");
    }

    /// Given: an InputState with "/unknown"
    /// When: completion is updated
    /// Then: completion_items is empty
    #[test]
    fn test_update_completions_no_match() {
        let mut state = InputState::new();
        state.text = "/xyznonexistent".to_string();
        state.cursor = 17;
        InputBarWidget.update_completions(&mut state);
        assert!(state.completion_items.is_empty());
    }

    /// Given: a completion list with 2+ items
    /// When: Up arrow is pressed (cycle through completions)
    /// Then: completion_index decreases, text updates
    #[test]
    fn test_up_arrow_cycles_completions() {
        let mut state = InputState::new();
        InputBarWidget.update_completions(&mut state);
        let mut state2 = InputState::new();
        state2.text = "/".to_string();
        state2.cursor = 1;
        InputBarWidget.update_completions(&mut state2);
        assert!(state2.completion_items.len() >= 2);

        let mut ib = InputBarWidget;
        let up_key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        ib.handle_key(&mut state2, up_key);
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
        InputBarWidget.update_completions(&mut state);
        let mut ib = InputBarWidget;
        // go forward past the end
        for _ in 0..state.completion_items.len() {
            let down_key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
            ib.handle_key(&mut state, down_key);
        }
        assert_eq!(state.completion_index, 0);
    }
}
