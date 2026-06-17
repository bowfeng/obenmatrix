//! TuiCoordinator — manages a single conversation turn for the TUI event loop.
//!
//! Replaces the old `handle_chat_input()` + `tokio::spawn` pattern in `lib.rs`.
//! The coordinator owns the full turn lifecycle:
//!   1. Receives user input text
//!  2. Prepares ChatPanel for streaming
//!  3. Executes the turn via Agent
//!  4. Sends TurnCompletion back to the event loop via done_tx
//!
//! Interrupt handling is cooperative via `InterruptState`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use parking_lot::Mutex as PlMutex;
use tokio::sync::mpsc::UnboundedSender;
use oben_sessions::SessionManager;

use crate::app::TurnCompletion;
use crate::build_image_message;
use crate::panels::PanelId;
use crate::App;

/// Coordinates a single turn from user message to TurnCompletion send.
pub struct TuiCoordinator {
    /// Shared channel for sending completion result back to the event loop.
    done_tx: UnboundedSender<TurnCompletion>,
}

impl TuiCoordinator {
    pub fn new(done_tx: UnboundedSender<TurnCompletion>) -> Self {
        Self { done_tx }
    }

    /// Execute a single turn: receive input, build agent request, send result.
    ///
    /// This is the replacement for `handle_chat_input()` in `lib.rs`.
    /// It does NOT spawn a task — it returns a future that the caller
    /// spawns in `tokio::spawn`.
    ///
    /// All async mutations happen inside this function.
    pub async fn execute(
        &self,
        input: String,
        app: &mut App,
    ) {
        let Some(agent) = app.agent.as_ref().map(|a| Arc::clone(a)) else {
            app.status = "Agent not initialized".into();
            return;
        };

        if app.turn_handle.is_some() {
            app.status = "Already processing a turn. Please wait...".into();
            return;
        }

        let was_chat = app.active_panel == PanelId::Chat;

        // Turn Start
        {
            let mut ts = app.turn_state.lock();
            ts.on_turn_start();
        }

        // ChatPanel Preparation
        if was_chat {
            if let Some(chat) = app.get_chat_mut() {
                chat.streaming = true;
                chat.input.streaming = true;
                chat.message_state
                    .scroll_to_bottom
                    .store(true, Ordering::SeqCst);
                chat.append_user_message(&input);
                chat.input.text.clear();
                chat.input.cursor = 0;
            }
        }

        app.interrupt_state.reset_for_turn();
        let interrupt_clone = Arc::clone(&app.interrupt_state);

        // Record message count for orphan truncation on abort
        if was_chat {
            let g = agent.lock().await;
            let sm_arc = g.session_manager();
            let sm = sm_arc.lock().await;
            app.turn_message_count = g
                .context_window_manager()
                .session_id()
                .and_then(|sid| sm.session(&sid))
                .map(|s| s.messages.len())
                .unwrap_or(0);
        }

        // Build input message
        let input_msg = build_image_message(&input);
        let has_images = matches!(
            input_msg.content,
            oben_models::MessageContent::Image { .. }
                | oben_models::MessageContent::Parts(_)
        );

        // Run the turn
        let result = if has_images {
            agent
                .lock()
                .await
                .turn_with_message(input_msg, Some(Arc::clone(&interrupt_clone)))
                .await
        } else {
            agent
                .lock()
                .await
                .turn(&input, false, Some(Arc::clone(&interrupt_clone)))
                .await
        };

        // Fetch session data while holding lock
        let (session_name, messages) = {
            let guard = agent.lock().await;
            let sm_arc = guard.session_manager();
            let sm = sm_arc.lock().await;
            let sid = guard
                .context_window_manager()
                .session_id()
                .and_then(|sid| sm.session(&sid))
                .map(|s| s.name.clone());
            let msgs = guard
                .context_window_manager()
                .session_id()
                .and_then(|sid| sm.session(&sid))
                .map(|s| s.messages.clone())
                .unwrap_or_default();
            (sid, msgs)
        };

        // Finalize
        match &result {
            Ok(_) => {
                let mut ts = app.turn_state.lock();
                ts.on_completed("completed");
                let _ = self.done_tx.send(TurnCompletion {
                    success: true,
                    session_name,
                    messages,
                });
            }
            Err(e) => {
                let mut ts = app.turn_state.lock();
                ts.on_error(&format!("{}", e));
                let _ = self.done_tx.send(TurnCompletion {
                    success: false,
                    session_name: None,
                    messages: Vec::new(),
                });
            }
        }

        // Store input in history
        app.input_history.append(&input);
    }
}
