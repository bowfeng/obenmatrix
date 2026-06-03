//! Chat panel — message history, streaming, input bar.

use super::{KeyAction, Panel};
use crate::turn::turn_state;
use crate::widgets::conversation::{ConversationState, ConversationWidget};
use crate::widgets::input_bar::{InputBarResult, InputBarWidget, InputState};
use crate::widgets::message_renderer::MessageRenderer;
use crossterm::event::KeyEvent;
use oben_models::Message;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use std::sync::atomic::Ordering;

/// Chat panel — message history, input bar, and streaming control.
pub struct ChatPanel {
    pub session_name: Option<String>,
    pub streaming: bool,
    pub input: InputState,
    pub message_state: ConversationState,
    pub message_count: usize,
    renderer: MessageRenderer,
    message_display: ConversationWidget,
}

impl ChatPanel {
    pub fn new(session_name: Option<String>, _messages: Option<Vec<Message>>) -> Self {
        let message_state = ConversationState::new();
        Self {
            session_name,
            streaming: false,
            input: InputState::new(),
            message_state,
            message_count: 0,
            renderer: MessageRenderer::new(),
            message_display: ConversationWidget,
        }
    }

    /// Create a chat panel with a specific theme from config.
    pub fn new_with_theme(
        session_name: Option<String>,
        _messages: Option<Vec<Message>>,
        theme: &str,
    ) -> Self {
        let mut panel = Self::new(session_name, _messages);
        panel.renderer.set_theme_from_str(theme);
        panel
    }

    /// Cycle to the next theme, returning the new theme name for persistence.
    pub fn cycle_theme(&mut self) -> String {
        let current = self.renderer.current_theme();
        let next = current.next();
        self.renderer.set_theme(next);
        next.slug().to_string()
    }

    /// Update message display state from session messages.
    pub fn update_from_messages(&mut self, messages: &[Message], session_name: Option<String>) {
        self.message_display.rebuild_from_messages(
            &mut self.message_state,
            messages,
            &self.renderer,
        );
        self.session_name = session_name;
        self.message_count = messages.len();
        self.streaming = false;
        self.message_state.scroll_to_bottom = true;
    }

    /// Set session data during panel activation.
    pub fn set_session_data(&mut self, session_name: Option<String>, messages: Vec<Message>) {
        if !messages.is_empty() || session_name.is_some() {
            self.update_from_messages(&messages, session_name);
        }
    }

    /// Append a user message to the display (shown immediately during a turn).
    pub fn append_user_message(&mut self, text: &str) {
        self.message_display
            .append_user_message(&mut self.message_state, text);
    }

    /// Append an info/system message to the display (used for slash commands like /help).
    pub fn append_info_message(&mut self, text: &str) {
        self.message_display
            .append_info_message(&mut self.message_state, text);
    }

    /// Update from turn state and sync stream_info in display.
    pub fn update_from_turn_state(&mut self, turn_state: &turn_state::TurnState) {
        self.message_display
            .update_stream_info(&mut self.message_state, turn_state);
    }

    /// Copy selection to system clipboard, then clear selection.
    pub fn copy_selection_to_clipboard(&mut self) {
        use crate::clipboard;
        if let Some(text) = self
            .message_display
            .get_selected_text(&mut self.message_state)
        {
            if !text.is_empty() && crate::clipboard::write_clipboard(&text) {
                self.message_state.clear_selection();
            }
        }
    }

    /// Set turn_state_ref so message display can read streaming text in real-time.
    pub fn set_turn_state_ref(
        &mut self,
        turn_state: std::sync::Arc<std::sync::Mutex<turn_state::TurnState>>,
    ) {
        self.message_state.turn_state_ref = Some(turn_state);
    }

    /// Clear all messages from the display and reset the message count.
    pub fn clear_display(&mut self) {
        self.message_state.message_entries.lock().unwrap().clear();
        self.message_state.scroll_to_bottom = true;
        self.message_count = 0;
        self.session_name = None;
    }

    /// Render the input bar widget.
    fn render_input_bar(&self, frame: &mut Frame, area: Rect, state: &InputState) {
        let palette = self.renderer.current_palette();
        InputBarWidget.render(frame, area, state, &palette);
    }
}

#[async_trait::async_trait]
impl Panel for ChatPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let input_height = InputBarWidget.calculate_input_height(&self.input, area.width);
        let chunks =
            Layout::vertical([Constraint::Min(0), Constraint::Length(input_height)]).split(area);

        let palette = self.renderer.current_palette();

        // Message display widget (pass streaming state from ChatPanel)
        self.message_display.render(
            frame,
            chunks[0],
            &self.message_state,
            &palette,
            self.streaming,
        );

        // Input bar widget
        self.render_input_bar(frame, chunks[1], &self.input);
    }

    async fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        let result = InputBarWidget.handle_key(&mut self.input, key);
        match result {
            InputBarResult::Consumed => KeyAction::None,
            InputBarResult::PassedThrough => KeyAction::None,
            InputBarResult::ChatInput(text) => KeyAction::ChatInput(text),
            InputBarResult::SlashCommand { cmd_name, extra } => match cmd_name.as_str() {
                "clear" => KeyAction::Clear,
                "new" => KeyAction::New,
                "compact" => KeyAction::Compact,
                "quit" => KeyAction::Quit,
                "reasoning" => KeyAction::Reasoning,
                "theme" => KeyAction::Theme,
                _ => KeyAction::Command { cmd_name, extra },
            },
        }
    }

    fn handle_mouse(&mut self, event: &crossterm::event::MouseEvent) -> bool {
        use crossterm::event::MouseEventKind;
        let scroll_step: i32 = 3;
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.message_state.scroll_to_bottom = false;
                let old = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                self.message_state
                    .user_scroll_offset
                    .fetch_add(scroll_step, Ordering::SeqCst);
                let new = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                tracing::info!(
                    "[mouse] ScrollDown: old_offset={} new_offset={} scroll_to_bottom=false",
                    old,
                    new
                );
                true
            }
            MouseEventKind::ScrollUp => {
                self.message_state.scroll_to_bottom = false;
                let old = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                self.message_state
                    .user_scroll_offset
                    .fetch_sub(scroll_step, Ordering::SeqCst);
                let new = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                tracing::info!(
                    "[mouse] ScrollUp: old_offset={} new_offset={} scroll_to_bottom=false",
                    old,
                    new
                );
                true
            }
            _ => false,
        }
    }
}
