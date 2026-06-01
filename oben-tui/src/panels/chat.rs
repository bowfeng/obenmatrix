//! Chat panel — message history, streaming, input bar.

use super::Panel;
use crate::widgets::input_bar::{InputBar, InputState};
use crate::widgets::message_display::{MessageDisplay, MessageDisplayState};
use crate::widgets::message_renderer::MessageRenderer;
use crate::widgets::style::Theme;
use crate::App;
use crossterm::event::KeyEvent;
use oben_models::Message;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;

/// Chat panel — message history, input bar, and streaming control.
pub struct ChatPanel {
    pub session_id: Option<String>,
    pub streaming: bool,
    pub input: InputState,
    pub message_state: MessageDisplayState,
    pub message_count: usize,
    renderer: MessageRenderer,
    message_display: MessageDisplay,
}

impl ChatPanel {
    pub fn new(session_id: Option<String>, _messages: Option<Vec<Message>>) -> Self {
        let message_state = MessageDisplayState::new();
        Self {
            session_id,
            streaming: false,
            input: InputState::new(),
            message_state,
            message_count: 0,
            renderer: MessageRenderer::new(),
            message_display: MessageDisplay,
        }
    }

    /// Update message display state from session messages.
    pub fn update_from_messages(&mut self, messages: &[Message], session_name: Option<String>) {
        self.message_display.rebuild_from_messages(
            &mut self.message_state,
            messages,
            &self.renderer,
        );
        self.session_id = session_name;
        self.message_count = messages.len();
        self.streaming = false;
        self.message_state.scroll_to_bottom = true;
    }

    /// Append a user message to the display (shown immediately during a turn).
    pub fn append_user_message(&mut self, text: &str) {
        self.message_display
            .append_user_message(&mut self.message_state, text);
    }

    /// Update from turn state and sync stream_info in display.
    pub fn update_from_turn_state(&mut self, turn_state: &crate::turn::event::TurnState) {
        self.message_display
            .update_stream_info(&mut self.message_state, turn_state);
    }

    /// Copy selection to system clipboard, then clear selection.
    pub fn copy_selection_to_clipboard(&mut self) {
        use crate::clipboard;
        if let Some(text) = self.message_display.get_selected_text(&mut self.message_state) {
            if !text.is_empty() && clipboard::write_clipboard(&text) {
                self.message_state.clear_selection();
            }
        }
    }

    /// Set turn_state_ref so message display can read streaming text in real-time.
    pub fn set_turn_state_ref(
        &mut self,
        turn_state: std::sync::Arc<std::sync::Mutex<crate::turn::event::TurnState>>,
    ) {
        self.message_state.turn_state_ref = Some(turn_state);
    }

    /// Toggle theme by pressing Ctrl+T.
    pub fn cycle_theme(&mut self) {
        let current = self.renderer.current_theme();
        let next = current.next();
        self.renderer.set_theme(next);
    }

    /// Render the input bar widget.
    fn render_input_bar(&self, frame: &mut Frame, area: Rect, state: &InputState) {
        InputBar.render(frame, area, state, &Theme::default());
    }
}

impl Panel for ChatPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let input_height = InputBar.calculate_input_height(&self.input, area.width);
        let chunks = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(input_height),
        ])
        .split(area);

        // Message display widget (pass streaming state from ChatPanel)
        self.message_display
            .render(frame, chunks[0], &self.message_state, &Theme::default(), self.streaming);

        // Input bar widget
        self.render_input_bar(frame, chunks[1], &self.input);
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) {
        if InputBar.handle_key(&mut self.input, app, key) {
            return;
        }
    }
}
