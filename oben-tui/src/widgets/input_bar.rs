//! Input bar widget — text input with wrapping, cursor tracking, placeholder,
//! tab completion overlay, streaming indicator, and message queue.
//!
//! This widget encapsulates all layout logic for the input area so the chat
//! panel only needs to manage state and delegate rendering.
//!
//! ### Message Queue (Hermes-style)
//!
//! Enter during streaming turns appends the composer text as a **new queued
//! message** to `input_queue::Vec<String>`, instead of submitting it
//! immediately.  Ctrl+Enter during streaming **drains the head** of the queue
//! and emits it as a chat submission.  The queue is rendered below the input
//! area as a compact numbered list.
//!
//! This matches the Hermes agent `busy_input_mode: queue` reference behaviour
//! (see `hermes_cli/config.py` + `ui-tui/src/components/queuedMessages.tsx`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use std::time::Instant;
use textwrap::wrap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// ── Constants ─────────────────────────────────────────────────────────────

/// Maximum number of messages in the queue before oldest is dropped.
const MAX_QUEUE_SIZE: usize = 50;

/// Number of queue entries visible at once (Hermes: `QUEUE_WINDOW = 3`).
const VISIBLE_QUEUE_ENTRIES: usize = 3;

/// Maximum visible length of a queued message preview.
const QUEUE_PREVIEW_WIDTH: usize = 50;

// ── Input state ───────────────────────────────────────────────────────────

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
    /// Message queue (Hermes-style): Enter during streaming appends,
    /// Ctrl+Enter drains the head and submits immediately.
    pub input_queue: Vec<String>,
    /// Count of steer messages already appended to chat for this turn.
    /// Only 0 or 1 — increments when steer appends to chat.
    pub pending_steer_count: u32,
}

/// Single tab completion entry.
#[derive(Clone)]
pub struct CompletionItem {
    pub cmd: String,
    pub desc: String,
}

// ── Public API ────────────────────────────────────────────────────────────

impl InputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            streaming: false,
            completion_items: Vec::new(),
            completion_index: 0,
            last_enter_time: None,
            input_queue: Vec::new(),
            pending_steer_count: 0,
        }
    }

    /// Enqueue a message.  Drops the oldest item if the queue is full
    /// (Hermes has no hard cap, but a 50-message ceiling prevents runaway
    /// memory in long-lived sessions).
    pub fn enqueue_msg(&mut self, msg: String) {
        if msg.trim().is_empty() {
            return;
        }
        if self.input_queue.len() >= MAX_QUEUE_SIZE {
            self.input_queue.remove(0);
        }
        self.input_queue.push(msg);
    }

    /// Dequeue (pop head) the next queued message.
    pub fn dequeue_msg(&mut self) -> Option<String> {
        if self.input_queue.is_empty() {
            return None;
        }
        Some(self.input_queue.remove(0))
    }

    /// Clear the entire queue.
    pub fn clear_queue(&mut self) {
        self.input_queue.clear();
    }

    /// Check whether the queue has pending messages.
    pub fn queue_has_items(&self) -> bool {
        !self.input_queue.is_empty()
    }

    /// Number of messages in the queue.
    pub fn queue_len(&self) -> usize {
        self.input_queue.len()
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

// ── Result type ───────────────────────────────────────────────────────────

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
    /// Inject a message into the next tool call without interrupting
    /// (mirrors `/steer` from the Hermes CLI).
    Steer(String),
}

// ── Widget ────────────────────────────────────────────────────────────────

/// Input bar widget.
///
/// Renders the text area block with wrapping, cursor, placeholder, streaming
/// indicator, queue indicator, and tab completion overlay.
pub struct InputBarWidget;

impl InputBarWidget {
    // ── Rendering ─────────────────────────────────────────────────────

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
        let mut y_offset = 0u16;
        if state.streaming {
            y_offset += self.render_streaming_indicator(frame, area, y_offset, palette);
        }

        // Queue indicator — show queued input count during streaming.
        if !state.input_queue.is_empty() {
            y_offset += self.render_queue_indicator(frame, area, y_offset, state, palette);
        }

        // Tab completion overlay.
        if !state.completion_items.is_empty() {
            self.render_tab_completion(frame, area, state, palette);
        }

        area.height
    }

    /// Predict the height in rows needed to render the input bar including
    /// border for a given screen width.
    pub fn calculate_input_height(&self, state: &InputState, screen_width: u16) -> u16 {
        let text_cols = (screen_width as usize).saturating_sub(2).max(1);
        let lines = if state.text.is_empty() {
            1u16
        } else {
            textwrap::wrap(state.text.as_str(), text_cols).len() as u16
        };

        // border(2) + text_lines + streaming(1) + queue(N) + completion(0..8)
        let text_area = lines.max(1);
        let streaming = if state.streaming { 1 } else { 0 };
        let queue = if state.input_queue.is_empty() {
            0
        } else {
            let visible = state.input_queue.len().min(VISIBLE_QUEUE_ENTRIES);
            // 1 header line + visible items
            1 + visible as u16
        };
        let completion = (state.completion_items.len() as u16).min(8);
        2 + text_area + streaming + queue + completion
    }

    // ── Key handling ──────────────────────────────────────────────────

    /// Process a key event for the input bar.
    ///
    /// Returns an `InputBarResult` indicating how the event was handled.
    pub fn handle_key(&mut self, state: &mut InputState, key: KeyEvent) -> InputBarResult {
        // ── Streaming mode (Hermes `busy_input_mode: queue`) ───────────
        if state.streaming {
            if key.code == KeyCode::Esc {
                tracing::info!("[input_bar] ESC received during streaming, interrupting");
                state.streaming = false;
                state.input_queue.clear();
                state.pending_steer_count = 0;
                return InputBarResult::Interrupt;
            }

            // Ctrl+Enter → submit current input as a steer message (inject into next tool call).
            // Each press is independent — chat layer handles deduplication.
            if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
                let text = std::mem::take(&mut state.text).trim().to_string();
                state.cursor = 0;
                state.completion_items.clear();
                state.completion_index = 0;
                if !text.is_empty() {
                    return InputBarResult::Steer(text);
                }
                return InputBarResult::Consumed;
            }

            // Enter → append current text to queue.
            if key.code == KeyCode::Enter
                || key.code == KeyCode::Char('\n')
                || key.code == KeyCode::Char('\r')
            {
                // Clone the text first (borrow checker workaround), then take.
                let queue_entry = if !state.text.trim().is_empty() {
                    Some(std::mem::take(&mut state.text).clone())
                } else {
                    None
                };
                if let Some(msg) = queue_entry {
                    state.enqueue_msg(msg);
                }
                // Always clear composition area (Hermes-style).
                state.text.clear();
                state.cursor = 0;
                state.completion_items.clear();
                state.completion_index = 0;
                // Moving on to queued message — reset steer dedup.
                state.pending_steer_count = 0;
                return InputBarResult::Consumed;
            }

            // Tab: cycle completion if available.
            if key.code == KeyCode::Tab && !state.completion_items.is_empty() {
                self.cycle_completion(state, true);
                return InputBarResult::Consumed;
            }

            // Arrow keys during completion.
            if key.code == KeyCode::Up {
                if !state.completion_items.is_empty() && state.completion_index == 0 {
                    return InputBarResult::Consumed;
                }
                return InputBarResult::PassedThrough;
            }
            if key.code == KeyCode::Down {
                if !state.completion_items.is_empty() {
                    self.cycle_completion(state, true);
                    return InputBarResult::Consumed;
                }
                return InputBarResult::PassedThrough;
            }

            // Block movement/editing keys during streaming: consume but
            // apply them so the user can edit the input area freely.
            if matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Backspace | KeyCode::Delete) {
                self.apply_text_edit(state, key);
                return InputBarResult::Consumed;
            }

            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('u'))
            {
                state.text.clear();
                state.cursor = 0;
                state.completion_items.clear();
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('k'))
            {
                let byte_idx = state.grapheme_to_byte(state.cursor);
                state.text.truncate(byte_idx);
                state.completion_items.clear();
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('w'))
            {
                if state.cursor > 0 {
                    let prefix: Vec<&str> = state.text[..state.grapheme_to_byte(state.cursor)]
                        .graphemes(true)
                        .collect();
                    let mut word_end = prefix.len();
                    for (i, g) in prefix.iter().enumerate().rev() {
                        if g.trim().is_empty() { word_end = i; } else { break; }
                    }
                    while word_end > 0 && !prefix[word_end - 1].trim().is_empty() {
                        word_end -= 1;
                    }
                    let byte_start = state.grapheme_to_byte(word_end);
                    let byte_end = state.grapheme_to_byte(state.cursor);
                    state.text.drain(byte_start..byte_end);
                    state.cursor = word_end;
                    state.completion_items.clear();
                }
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('a'))
            {
                state.cursor = 0;
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('e'))
            {
                state.cursor = state.grapheme_count();
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('v')
            {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    if !text.is_empty() {
                        let byte_idx = state.grapheme_to_byte(state.cursor);
                        state.text.insert_str(byte_idx, &text);
                        state.cursor += text.graphemes(true).count();
                    }
                }
                state.completion_items.clear();
                return InputBarResult::Consumed;
            }
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char('b') | KeyCode::Char('f'))
            {
                self.apply_text_edit(state, key);
                return InputBarResult::Consumed;
            }

            // Printable characters → append to text area.
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char(_))
            {
                if let KeyCode::Char(c) = key.code {
                    let byte_idx = state.grapheme_to_byte(state.cursor);
                    state.text.insert_str(byte_idx, &c.to_string());
                    state.cursor += 1;
                    state.completion_items.clear();
                    return InputBarResult::Consumed;
                }
            }

            // All other keys consumed (no-op during streaming).
            return InputBarResult::Consumed;
        }

        // ── Idle mode ─────────────────────────────────────────────────

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
                        if g.trim().is_empty() { word_end = i; } else { break; }
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
                if state.cursor > 0 {
                    let prefix: Vec<&str> = state.text[..state.grapheme_to_byte(state.cursor)]
                        .graphemes(true)
                        .collect();
                    let mut i = prefix.len();
                    while i > 0 && prefix[i - 1].trim().is_empty() { i -= 1; }
                    while i > 0 && !prefix[i - 1].trim().is_empty() { i -= 1; }
                    state.cursor = i;
                }
                self.update_completions(state);
                return InputBarResult::Consumed;
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                if state.cursor < state.grapheme_count() {
                    let remaining: Vec<&str> = state.text[state.grapheme_to_byte(state.cursor)..]
                        .graphemes(true)
                        .collect();
                    let mut j = 0;
                    while j < remaining.len() && remaining[j].trim().is_empty() { j += 1; }
                    while j < remaining.len() && !remaining[j].trim().is_empty() { j += 1; }
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

    /// Submit the current text as chat input (slash command or plain).
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

    /// Apply text editing keys during streaming (Left/Right/Backspace/Bs/
    /// Alt+back/fwd).  This mirrors the non-streaming logic so the input
    /// area remains fully usable while the AI is still answering.
    fn apply_text_edit(&self, state: &mut InputState, key: KeyEvent) {
        match key.code {
            KeyCode::Left if state.cursor > 0 => {
                state.cursor -= 1;
            }
            KeyCode::Right if state.cursor < state.grapheme_count() => {
                state.cursor += 1;
            }
            KeyCode::Backspace if state.cursor > 0 => {
                let byte_idx = state.grapheme_to_byte(state.cursor - 1);
                let g = &state.text[byte_idx..];
                let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                state.text.drain(byte_idx..byte_idx + len);
                state.cursor -= 1;
            }
            KeyCode::Delete => {
                if state.cursor < state.grapheme_count() {
                    let byte_idx = state.grapheme_to_byte(state.cursor);
                    let g = &state.text[byte_idx..];
                    let len = g.graphemes(true).next().map(|x| x.len()).unwrap_or(3);
                    state.text.drain(byte_idx..byte_idx + len);
                }
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                if state.cursor > 0 {
                    let prefix: Vec<&str> = state.text[..state.grapheme_to_byte(state.cursor)]
                        .graphemes(true)
                        .collect();
                    let mut i = prefix.len();
                    while i > 0 && prefix[i - 1].trim().is_empty() { i -= 1; }
                    while i > 0 && !prefix[i - 1].trim().is_empty() { i -= 1; }
                    state.cursor = i;
                }
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                if state.cursor < state.grapheme_count() {
                    let remaining: Vec<&str> = state.text[state.grapheme_to_byte(state.cursor)..]
                        .graphemes(true)
                        .collect();
                    let mut j = 0;
                    while j < remaining.len() && remaining[j].trim().is_empty() { j += 1; }
                    while j < remaining.len() && !remaining[j].trim().is_empty() { j += 1; }
                    state.cursor += j;
                }
            }
            _ => {}
        }
    }

    // ── Rendering helpers ───────────────────────────────────────────

    fn render_streaming_indicator(
        &self,
        frame: &mut Frame,
        area: Rect,
        row_y: u16,
        palette: &ratatui_themes::ThemePalette,
    ) -> u16 {
        let text = " \u{23F3} Streaming... ";
        let w = text.len() as u16 + 2;
        let indicator_area = Rect::new(
            area.right().saturating_sub(w + 2),
            area.y + 1 + row_y,
            w,
            1,
        );
        let para = Paragraph::new(Line::from(Span::styled(
            text,
            Style::default()
                .fg(palette.info)
                .add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(para, indicator_area);
        1
    }

    /// Render queue indicator below the input bar.
    fn render_queue_indicator(
        &self,
        frame: &mut Frame,
        area: Rect,
        row_y: u16,
        state: &InputState,
        palette: &ratatui_themes::ThemePalette,
    ) -> u16 {
        let queue = &state.input_queue;
        if queue.is_empty() {
            return 0;
        }

        let n = queue.len();
        // "queued (N)" header + visible entries.
        let header = format!(" queued ({n})");
        let header_w = header.len() as u16 + 1;
        let top = area.y + 1 + row_y + 1; // below streaming indicator (row_y + 1)
        let header_start = area.x + 1;
        // If header wouldn't fit on one line, hide it (rare edge case).
        let can_show_header = area.width > header_w + 1;

        if can_show_header {
            let para = Paragraph::new(Line::from(Span::styled(
                header,
                Style::default().fg(palette.warning),
            )));
            let h_area = Rect::new(header_start, top, header_w + 1, 1);
            frame.render_widget(para, h_area);
        }

        let visible: &[String] = &queue[..n.min(VISIBLE_QUEUE_ENTRIES)];
        let item_rows = visible.len() as u16;

        // "queued (N)" takes 1 row if shown, 0 otherwise.
        let base = if can_show_header { 1 } else { 0 };

        for (i, msg) in visible.iter().enumerate() {
            let preview = {
                let chars: String = msg.chars().take(QUEUE_PREVIEW_WIDTH).collect();
                chars
            };
            let row_text = format!("   {}: {}", i + 1, preview);
            let para = Paragraph::new(Line::from(Span::styled(
                row_text,
                Style::default().fg(palette.muted),
            )));
            let y = top + base + i as u16;
            let w = area.width.saturating_sub(2);
            let item_area = Rect::new(area.x + 1, y, w.saturating_sub(1), 1);
            frame.render_widget(para, item_area);
        }

        // If there are hidden queues show an ellipsis.
        if n > VISIBLE_QUEUE_ENTRIES {
            let hidden = n - VISIBLE_QUEUE_ENTRIES;
            let more_text = format!("   ... and {} more", hidden);
            let para = Paragraph::new(Line::from(Span::styled(
                more_text,
                Style::default().fg(palette.muted),
            )));
            let y = top + base + item_rows;
            let w = area.width.saturating_sub(2);
            let item_area = Rect::new(area.x + 1, y, w.saturating_sub(1), 1);
            frame.render_widget(para, item_area);
            1 + base + item_rows
        } else {
            base + item_rows
        }
    }

    // ── Tab completion helpers ──────────────────────────────────────
    // (shared between streaming and idle mode)

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

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(text: &str) -> InputState {
        let mut s = InputState::new();
        s.text = text.to_string();
        s
    }

    fn ib() -> InputBarWidget {
        InputBarWidget
    }

    // ── Height / layout tests ───────────────────────────────────────

    #[test]
    fn test_calculate_input_height_empty() {
        let ib = ib();
        let state = make_state("");
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_single_line_short() {
        let ib = ib();
        let state = make_state("hello");
        assert_eq!(ib.calculate_input_height(&state, 40), 3);
    }

    #[test]
    fn test_calculate_input_height_word_wraps() {
        let ib = ib();
        let state = make_state(
            "hello world foo bar baz qux quux corge extra long text here",
        );
        let h = ib.calculate_input_height(&state, 40);
        assert_eq!(h, 4, "word wrap should add height");
    }

    #[test]
    fn test_calculate_input_height_explicit_newline() {
        let ib = ib();
        let state = make_state("line1\nline2\nline3");
        assert_eq!(ib.calculate_input_height(&state, 40), 5);
    }

    #[test]
    fn test_calculate_input_height_streaming() {
        let ib = ib();
        let mut state = make_state("");
        state.streaming = true;
        assert_eq!(ib.calculate_input_height(&state, 40), 4);
    }

    #[test]
    fn test_calculate_input_height_completion_list() {
        let ib = ib();
        let mut state = make_state("");
        state.completion_items = vec![
            CompletionItem { cmd: "/foo".into(), desc: "a".into() },
            CompletionItem { cmd: "/bar".into(), desc: "b".into() },
            CompletionItem { cmd: "/baz".into(), desc: "c".into() },
        ];
        assert_eq!(ib.calculate_input_height(&state, 40), 6);
    }

    #[test]
    fn test_calculate_input_height_completion_clamped() {
        let ib = ib();
        let mut state = make_state("");
        for i in 0..12 {
            state.completion_items.push(CompletionItem {
                cmd: format!("/cmd{}", i),
                desc: format!("desc{}", i),
            });
        }
        assert_eq!(ib.calculate_input_height(&state, 40), 11); // 2 + 1 + 8
    }

    #[test]
    fn test_calculate_input_height_with_queue() {
        let ib = ib();
        let mut state = make_state("");
        state.streaming = true;
        state.input_queue = vec!["a".into(), "b".into()];
        // streaming(1) + queue header(1) + 2 items = 4 extra
        assert_eq!(ib.calculate_input_height(&state, 40), 7);
    }

    #[test]
    fn test_calculate_input_height_queue_visible_clamped() {
        let ib = ib();
        let mut state = make_state("");
        state.streaming = true;
        for i in 0..20 {
            state.input_queue.push(format!("msg{}", i));
        }
        // streaming(1) + header(1) + VISIBLE_QUEUE_ENTRIES(3) = 5 extra
        assert_eq!(ib.calculate_input_height(&state, 40), 8);
    }

    // ── Normal-mode tests (unchanged) ──────────────────────────────

    #[test]
    fn test_space_key_inserts_single_space() {
        let mut state = make_state("");
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        assert_eq!(state.text, " ");
    }

    #[test]
    fn test_space_key_inserts_at_cursor() {
        let mut state = make_state("ab");
        state.cursor = 1;
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "a b");
    }

    #[test]
    fn test_backspace_removes_rightmost_grapheme() {
        let mut state = make_state("hello");
        state.cursor = 5;
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "hell");
        assert_eq!(state.cursor, 4);
    }

    #[test]
    fn test_enter_consumed_with_text() {
        let mut state = make_state("hello");
        state.cursor = 5;
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::ChatInput(_)));
        assert!(state.text.is_empty(), "text cleared after submit");
    }

    #[test]
    fn test_enter_not_consumed_when_empty() {
        let mut state = make_state("");
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
    }

    #[test]
    fn test_cursor_moves_by_grapheme_not_by_byte() {
        let mut state = InputState::new();
        state.text = "你好世界测试".to_string();
        state.cursor = 0;
        for _ in 0..3 {
            let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
            ib().handle_key(&mut state, key);
        }
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn test_ctrl_w_deletes_word_before_cursor() {
        let mut state = make_state("hello world");
        state.cursor = 11;
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "hello ");
    }

    #[test]
    fn test_ctrl_u_clears_all() {
        let mut state = make_state("some text here");
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "");
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_esc_interrupts_streaming() {
        let mut state = make_state("hello");
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Interrupt));
        assert!(!state.streaming);
    }

    #[test]
    fn test_esc_interrupt_clears_queue() {
        let mut state = make_state("");
        state.streaming = true;
        state.enqueue_msg("first".into());
        state.enqueue_msg("second".into());
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        ib().handle_key(&mut state, key);
        assert!(state.input_queue.is_empty());
    }

    #[test]
    fn test_streaming_suppress_text_typing_normal() {
        let mut state = make_state("");
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        // In streaming mode, text gets added to the composer, not to queue.
        assert_eq!(state.text, "a");
    }

    #[test]
    fn test_enter_during_streaming_appends_to_queue() {
        let mut state = make_state("queued message one");
        state.streaming = true;

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        assert_eq!(state.input_queue.len(), 1);
        assert_eq!(state.input_queue[0], "queued message one");
        assert!(state.text.is_empty());
    }

    #[test]
    fn test_multiple_ents_during_streaming_builds_queue() {
        let mut state = InputState::new();
        state.streaming = true;

        for msg in &["first", "second", "third"] {
            state.text = msg.to_string();
            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            ib().handle_key(&mut state, key);
        }

        assert_eq!(state.input_queue.len(), 3);
        assert_eq!(state.input_queue[0], "first");
        assert_eq!(state.input_queue[1], "second");
        assert_eq!(state.input_queue[2], "third");
        assert!(state.text.is_empty());
    }

    #[test]
    fn test_ctrl_enter_during_streaming_triggers_steer() {
        let mut state = make_state("steer instruction");
        state.streaming = true;
        state.enqueue_msg("queued msg".into());

        // Ctrl+Enter → current text input is sent as steer, queue is untouched.
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(
            result,
            InputBarResult::Steer(ref s) if s == "steer instruction"
        ));
        assert_eq!(state.input_queue, vec!["queued msg".to_string()]);
        assert_eq!(state.text, "");
    }

    #[test]
    fn test_ctrl_enter_clears_text_input() {
        let mut state = make_state("composer text");
        state.streaming = true;

        // Ctrl+Enter → current text is sent as steer, queue is untouched.
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        ib().handle_key(&mut state, key);
        // Queue is untouched — Ctrl+Enter sends current input as steer.
        assert_eq!(state.input_queue.len(), 0);
        assert_eq!(state.text, "");
    }

    #[test]
    fn test_ctrl_enter_empty_text_consumed() {
        let mut state = InputState::new();
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
    }

    #[test]
    fn test_enter_empty_during_streaming_noop() {
        let mut state = InputState::new();
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = ib().handle_key(&mut state, key);
        assert!(matches!(result, InputBarResult::Consumed));
        assert!(state.input_queue.is_empty());
    }

    #[test]
    fn test_streaming_ctrl_w_clears_word() {
        // Cursor at end — deletes the whole word "world"
        let mut state = InputState::new();
        state.text = "hello world".to_string();
        state.cursor = 11; // end of "world"
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "hello ");
    }

    #[test]
    fn test_streaming_ctrl_u_clears_text() {
        let mut state = make_state("some text here");
        state.streaming = true;
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        ib().handle_key(&mut state, key);
        assert_eq!(state.text, "");
    }

    #[test]
    fn test_update_completions_finds_matching_commands() {
        let mut state = InputState::new();
        state.text = "/co".to_string();
        state.cursor = 3;
        InputBarWidget.update_completions(&mut state);
        assert!(!state.completion_items.is_empty());
        let found = state.completion_items.iter().any(|c| c.cmd == "/compact");
        assert!(found);
    }

    #[test]
    fn test_update_completions_no_match() {
        let mut state = InputState::new();
        state.text = "/xyznonexistent".to_string();
        state.cursor = 17;
        InputBarWidget.update_completions(&mut state);
        assert!(state.completion_items.is_empty());
    }

    #[test]
    fn test_up_arrow_cycles_completions() {
        let mut state = InputState::new();
        InputBarWidget.update_completions(&mut state);
        let mut state2 = InputState::new();
        state2.text = "/".to_string();
        state2.cursor = 1;
        InputBarWidget.update_completions(&mut state2);
        assert!(state2.completion_items.len() >= 2);

        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        ib().handle_key(&mut state2, key);
        assert_eq!(state2.completion_index, 0);
    }

    #[test]
    fn test_down_arrow_cycles_completions_forward() {
        let mut state = InputState::new();
        state.text = "/".to_string();
        state.cursor = 1;
        InputBarWidget.update_completions(&mut state);
        for _ in 0..state.completion_items.len() {
            let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
            ib().handle_key(&mut state, key);
        }
        assert_eq!(state.completion_index, 0);
    }

    // ── Queue management tests ──────────────────────────────────────

    #[test]
    fn test_enqueue_empty_string_rejected() {
        let mut state = InputState::new();
        state.enqueue_msg("".into());
        state.enqueue_msg("   ".into());
        assert!(state.input_queue.is_empty());
    }

    #[test]
    fn test_enqueue_preserves_order() {
        let mut state = InputState::new();
        state.enqueue_msg("third".into());
        state.enqueue_msg("first".into());
        state.enqueue_msg("second".into());
        assert_eq!(state.input_queue, ["third", "first", "second"]);
    }

    #[test]
    fn test_dequeue_returns_head() {
        let mut state = InputState::new();
        state.enqueue_msg("a".into());
        state.enqueue_msg("b".into());
        assert_eq!(state.dequeue_msg(), Some("a".into()));
        assert_eq!(state.dequeue_msg(), Some("b".into()));
        assert!(state.dequeue_msg().is_none());
    }

    #[test]
    fn test_clear_queue() {
        let mut state = InputState::new();
        state.enqueue_msg("msg1".into());
        state.enqueue_msg("msg2".into());
        state.clear_queue();
        assert!(state.input_queue.is_empty());
    }

    #[test]
    fn test_queue_is_empty() {
        let mut state = InputState::new();
        assert!(!state.queue_has_items());
        state.enqueue_msg("a".into());
        assert!(state.queue_has_items());
        state.clear_queue();
        assert!(!state.queue_has_items());
    }

    #[test]
    fn test_queue_len() {
        let mut state = InputState::new();
        assert_eq!(state.queue_len(), 0);
        state.enqueue_msg("x".into());
        state.enqueue_msg("y".into());
        assert_eq!(state.queue_len(), 2);
    }

    #[test]
    fn test_queue_max_size_drops_oldest() {
        let mut state = InputState::new();
        for i in 0..MAX_QUEUE_SIZE + 5 {
            state.enqueue_msg(format!("msg{}", i));
        }
        assert_eq!(state.input_queue.len(), MAX_QUEUE_SIZE);
        assert!(state.input_queue[0].starts_with("msg5"));
        assert!(state.input_queue[MAX_QUEUE_SIZE - 1].starts_with("msg"));
    }
}
