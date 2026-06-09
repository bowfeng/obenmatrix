//! Chat panel — message history, streaming, input bar.

use super::{KeyAction, Panel};
use crate::event::EventBus;
use crate::turn::turn_state;
use crate::widgets::conversation::{ConversationState, ConversationWidget};
use crate::widgets::input_bar::{InputBarResult, InputBarWidget, InputState};
use crate::widgets::message_renderer::MessageRenderer;
use crossterm::event::KeyEvent;
use oben_models::Message;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Chat panel — message history, input bar, and streaming control.
pub struct ChatPanel {
    pub session_name: Option<String>,
    pub streaming: bool,
    pub input: InputState,
    pub message_state: ConversationState,
    pub message_count: usize,
    renderer: MessageRenderer,
    message_display: ConversationWidget,
    /// Shared event bus (used by auto-drain to emit queued messages when a
    /// turn settles).  Set via `set_event_bus()` during app init.
    event_bus: Option<Arc<EventBus>>,
    /// Channel to send drained messages back to the event loop.  Set via
    /// `set_input_sender()` during app init.
    input_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::TuiEvent>>,
    /// Previous turn phase — used to detect transitions into a settled state
    /// for queue auto-drain.  Only the *transition* into Completed/Error
    /// triggers a drain; steady-state draws do not fire again.
    prev_phase: turn_state::TurnPhase,
    /// Whether auto-drain has already fired for this completion.
    /// Prevents draining the next queued message when a new turn starts
    /// but the phase hasn't yet moved from Completed (e.g., while delta
    /// callbacks are still firing for the turn that just ended).
    drained_this_turn: bool,
}

impl ChatPanel {
    pub fn new(session_name: Option<String>) -> Self {
        let message_state = ConversationState::new();
        Self {
            session_name,
            streaming: false,
            input: InputState::new(),
            message_state,
            message_count: 0,
            renderer: MessageRenderer::new(),
            message_display: ConversationWidget,
            event_bus: None,
            input_tx: None,
            prev_phase: turn_state::TurnPhase::Idle,
            drained_this_turn: false,
        }
    }



    /// Create a chat panel with a specific theme from config.
    pub fn new_with_theme(
        session_name: Option<String>,
        theme: &str,
    ) -> Self {
        let mut panel = Self::new(session_name);
        panel.renderer.set_theme_from_str(theme);
        panel
    }

    /// Set the event bus for auto-drain.
    pub fn set_event_bus(&mut self, event_bus: Arc<EventBus>) {
        self.event_bus = Some(event_bus);
    }

    /// Set the input event sender for queue drain — drained messages are
    /// sent through here so they go through the normal event loop.
    pub fn set_input_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<crate::TuiEvent>) {
        self.input_tx = Some(tx);
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
            false,
        );
        self.session_name = session_name;
        self.message_count = messages.len();
        self.streaming = false;
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
    pub fn update_from_turn_state(&mut self, ts: &turn_state::TurnState) {
        self.message_display
            .update_stream_info(&mut self.message_state, ts);

        let prev = self.prev_phase.clone();
        let current = ts.phase.clone();

        let settled = matches!(
            current,
            turn_state::TurnPhase::Completed | turn_state::TurnPhase::Error(_)
        );
        let transitioning = !matches!(prev, turn_state::TurnPhase::Completed | turn_state::TurnPhase::Error(_));

        // Only drain when transitioned INTO settled AND agent is truly idle
        // (no pending delta callbacks from the just-finished turn) AND
        // we haven't already drained this completion.
        let agent_idle = !ts.is_active();

        let drain_trigger = settled && transitioning && agent_idle && !self.drained_this_turn;
        tracing::debug!(
            "[chat_panel] update_from_turn_state: settled={}, transitioning={}, agent_idle={}, drained_this_turn={}, prev_phase={:?}, current_phase={:?}, drain={}",
            settled, transitioning, agent_idle, self.drained_this_turn, prev, current, drain_trigger
        );

        if drain_trigger {
            tracing::debug!("[chat_panel] auto-drain trigger: queue_len={}", self.input.queue_len());
            if let Some(msg) = self.input.dequeue_msg() {
                let drain_time = Instant::now();
                tracing::info!("[chat_panel] auto-drain queued message: {}", msg);
                // Send the queued message through the event loop, not by
                // faking a full turn lifecycle.  Faking TurnStart +
                // StreamDelta + TurnCompleted in draw() races with the
                // real agent task that processes the next turn, which is
                // why the streaming was "interrupted".
                if let Some(ref tx) = &self.input_tx {
                    if tx.send(crate::TuiEvent::ChatInput(msg.clone())).is_err() {
                        tracing::warn!("[chat_panel] failed to send drained message");
                    }
                    // Force scroll to bottom after auto-drain so the next
                    // AI response is visible.  Even if the user had scrolled
                    // up while reading the previous response, the auto-drain
                    // is sending a new message which should be visible.
                    self.message_state.scroll_to_bottom.store(true, Ordering::SeqCst);
                    tracing::debug!(
                        "[chat_panel] drain: sent ChatInput in {:?}, queue_len_after={}, scroll_to_bottom=true",
                        drain_time.elapsed(),
                        self.input.queue_len()
                    );
                } else {
                    tracing::debug!("[chat_panel] no input_tx for drained message");
                }
            }
            self.drained_this_turn = true;
        } else if !settled || !transitioning {
            self.drained_this_turn = false;
        }

        // Always keep prev_phase in sync.
        self.prev_phase = current;
    }

    /// Copy selection to system clipboard, then clear selection.
    pub fn copy_selection_to_clipboard(&mut self) {
        tracing::debug!(
            "[selection] copy: start={:?} end={:?} body_width={} scroll_pos={}",
            self.message_state.selection_start,
            self.message_state.selection_end,
            self.message_state.body_width,
            self.message_state.scroll_pos.load(Ordering::SeqCst)
        );
        if let Some(text) = self
            .message_display
            .get_selected_text(&self.message_state)
        {
            tracing::debug!("[selection] copy: got {} chars", text.len());
            if !text.is_empty() && crate::clipboard::write_clipboard(&text) {
                self.message_state.clear_selection();
            }
        } else {
            tracing::debug!("[selection] copy: no text returned");
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
        self.message_state.scroll_to_bottom.store(true, Ordering::SeqCst);
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

        // Render text selection highlight (drawn on top of message blocks).
        if self.message_state.selection_start.is_some() {
            self.message_display.render_selection(
                frame,
                chunks[0],
                &self.message_state,
                &palette,
            );
        }

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
            InputBarResult::Interrupt => KeyAction::Interrupt,
            InputBarResult::Steer(text) => KeyAction::Steer(text),
        }
    }

    fn handle_mouse(&mut self, area: Rect, event: &crossterm::event::MouseEvent) -> Option<String> {
        use crossterm::event::MouseEventKind;
        use crossterm::event::MouseButton;

        let scroll_step: i32 = 3;
        let actual_body_width = area.width.saturating_sub(6);
        let content_x_offset = area.x.saturating_add(4);
        tracing::info!(
            "[selection] mouse event: row={} col={} kind={:?} msg_area_y={} content_x={} content_x_offset={} body_width={} old_body_width={}",
            event.row, event.column, event.kind, area.y,
            self.message_state.content_x,
            content_x_offset,
            actual_body_width,
            self.message_state.body_width
        );
        if actual_body_width > 0 {
            self.message_state.body_width = actual_body_width as usize;
        }
        self.message_state.wrap_width = actual_body_width as usize;
        self.message_state.content_x = content_x_offset;
        self.message_state.msg_area_y = area.y;
        self.message_state.content_y = area.y.saturating_add(2);

        match event.kind {
            MouseEventKind::ScrollDown => {
                self.message_state.scroll_to_bottom.store(false, Ordering::SeqCst);
                let old = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                self.message_state
                    .user_scroll_offset
                    .fetch_sub(scroll_step, Ordering::SeqCst);
                tracing::info!(
                    "[mouse] ScrollDown: old_offset={} new_offset={} scroll_to_bottom=false",
                    old,
                    self.message_state.user_scroll_offset.load(Ordering::SeqCst)
                );
                return None;
            }
            MouseEventKind::ScrollUp => {
                self.message_state.scroll_to_bottom.store(false, Ordering::SeqCst);
                let old = self.message_state.user_scroll_offset.load(Ordering::SeqCst);
                self.message_state
                    .user_scroll_offset
                    .fetch_add(scroll_step, Ordering::SeqCst);
                tracing::info!(
                    "[mouse] ScrollUp: old_offset={} new_offset={} scroll_to_bottom=false",
                    old,
                    self.message_state.user_scroll_offset.load(Ordering::SeqCst)
                );
                return None;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let row = event.row;
                let col = event.column;
                self.message_state.selection_start = Some((row, col));
                self.message_state.selection_end = None;
                tracing::debug!(
                    "[selection] MOUSE_DOWN: row={} col={} → selection=({},{})",
                    row, col, row, col
                );
                return None;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let row = event.row;
                let col = event.column;
                self.message_state.selection_end = Some((row, col));
                return None;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let row = event.row;
                let col = event.column;
                self.message_state.selection_end = Some((row, col));
                tracing::debug!(
                    "[mouse] LeftRelease: sel=(row={},col={}) content_y={} scroll_pos={}",
                    row, col, self.message_state.content_y,
                    self.message_state.scroll_pos.load(Ordering::SeqCst)
                );
                if self.message_state.selection_start.is_some() && self.message_state.selection_end.is_some() {
                    if let Some(text) = self.message_display.get_selected_text(&self.message_state) {
                        tracing::debug!("[mouse] LeftRelease: got {} chars", text.len());
                        if !text.is_empty() && crate::clipboard::write_clipboard(&text) {
                            tracing::info!("[mouse] auto-copied {} chars", text.len());
                            self.message_state.clear_selection();
                            return Some(text);
                        } else {
                            tracing::debug!("[mouse] LeftRelease: write_clipboard failed or empty");
                        }
                    } else {
                        tracing::debug!("[mouse] LeftRelease: get_selected_text returned None");
                    }
                }
                return None;
            }
            _ => None,
        }
    }
}
