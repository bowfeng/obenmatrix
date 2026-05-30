//! Turn controller — manages turn lifecycle, event distribution, and task dispatch.
//!
//! This is a deep module: it encapsulates the entire loop for event handling,
//! turn spawning, abortion, and result routing. The external TUI loop only needs to:
//! 1. Send events via `controller.event_tx()`
//! 2. Pump the loop via `controller.pump()` each frame
//! 3. Check for completion via `controller.poll_completion()`
//! 4. Cancel turn via `controller.cancel_current()`
//! 5. Supply agent via `controller.set_agent()`

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::AbortHandle;

use super::event::TurnState;
use crate::TuiEvent;

/// Agent turn result.
#[derive(Debug, PartialEq)]
pub enum TurnResult {
    Ok(String),
    Err(String),
    Cancelled,
}

/// Action to take after a turn completes.
pub enum RebuildAction {
    ChatPanel,
    None,
}

/// Completion returned by `poll_completion()` for the TUI event loop.
pub struct Completion {
    pub result: TurnResult,
    pub rebuild: RebuildAction,
}

/// Turn controller — manages turn lifecycle, event distribution, and task dispatch.
///
/// ## Turn flow
/// 1. App calls `set_agent(agent)` once after `init_chat`.
/// 2. Each `pump()` drains pending events, drains agent from slot, starts turn.
/// 3. When turn completes, agent is restored to the slot.
/// 4. App calls `poll_completion()` to get the result, then `take_agent()` to
///    retrieve the finished agent for panel rebuild.
pub struct TurnController {
    pub(crate) state: Arc<Mutex<TurnState>>,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
    event_tx: UnboundedSender<TuiEvent>,
    /// Agent slot — drained before turn, replenished after.
    agent: Arc<Mutex<Option<crate::Agent>>>,
    current_handle: Arc<Mutex<Option<AbortHandle>>>,
    last_completion: Arc<Mutex<Option<Completion>>>,
    /// Channel that sends (agent, result) back after turn completion.
    done_tx: tokio::sync::mpsc::UnboundedSender<(Option<crate::Agent>, TurnResult)>,
    done_rx: tokio::sync::mpsc::UnboundedReceiver<(Option<crate::Agent>, TurnResult)>,
}

impl TurnController {
    pub fn new() -> (Self, UnboundedSender<TuiEvent>) {
        let (event_tx, event_rx) = unbounded_channel();
        let (done_tx, done_rx) = unbounded_channel();

        let event_tx_send = event_tx.clone();

        let controller = Self {
            state: Arc::new(Mutex::new(TurnState::default())),
            event_rx,
            event_tx,
            agent: Arc::new(Mutex::new(None)),
            done_tx,
            done_rx,
            current_handle: Arc::new(Mutex::new(None)),
            last_completion: Arc::new(Mutex::new(None)),
        };

        (controller, event_tx_send)
    }

    /// Set the agent that the controller will use for turns.
    pub fn set_agent(&self, agent: crate::Agent) {
        *self.agent.lock().unwrap() = Some(agent);
    }

    /// Drain the agent from the slot — returns None if empty.
    pub fn take_agent(&self) -> Option<crate::Agent> {
        self.agent.lock().unwrap().take()
    }

    /// Get the internal state Arc for callback creation during init_chat.
    pub fn state_arc(&self) -> Arc<Mutex<TurnState>> {
        Arc::clone(&self.state)
    }

    /// Send channel for external event dispatch.
    pub fn event_tx(&self) -> &UnboundedSender<TuiEvent> {
        &self.event_tx
    }

    /// Pump the controller — drain one event per call.
    ///
    /// Synchronous (no await). Drains one ready event from the channel,
    /// routes ChatInput to pending buffer, then checks for completed turns.
    pub fn pump(&mut self) {
        // Drain one event from channel (non-blocking).
        match self.event_rx.try_recv() {
            Ok(TuiEvent::ChatInput(text)) => {
                // If no turn is active and state is idle, start turn immediately.
                if !self.is_active() {
                    if let Ok(ts) = self.state.lock() {
                        if ts.phase == super::event::TurnPhase::Idle {
                            drop(ts);
                            self.start_turn(&text);
                            return;
                        }
                    }
                }
            }
            Ok(TuiEvent::Key(_)) => {
                // Key events are consumed here to prevent channel buildup.
                // Real key handling is done by panels synchronously during draw.
            }
            Ok(TuiEvent::Mouse(_)) => {
                // Mouse events consumed here to prevent channel buildup.
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {}
        }

        // Drain remaining ready ChatInput events so only the latest one matters.
        while let Ok(TuiEvent::ChatInput(text)) = self.event_rx.try_recv() {
            if !self.is_active() {
                if let Ok(ts) = self.state.lock() {
                    if ts.phase == super::event::TurnPhase::Idle {
                        drop(ts);
                        self.start_turn(&text);
                        return;
                    }
                }
            }
        }

        // Drain completion channel (result + agent).
        while let Ok((agent_opt, result)) = self.done_rx.try_recv() {
            if let Some(agent) = agent_opt {
                *self.agent.lock().unwrap() = Some(agent);
            }
            let completion = Completion {
                result,
                rebuild: RebuildAction::ChatPanel,
            };
            *self.last_completion.lock().unwrap() = Some(completion);
        }
    }

    fn start_turn(&self, input: &str) {
        if self.is_active() {
            return;
        }

        // Drain the agent from the slot.
        let agent = self.agent.lock().unwrap().take();
        let Some(agent) = agent else {
            return;
        };

        {
            if let Ok(mut ts) = self.state.lock() {
                ts.on_turn_start();
            }
        }

        let input_clone = input.to_string();
        let state = Arc::clone(&self.state);
        let handle = Arc::clone(&self.current_handle);
        let done_tx = self.done_tx.clone();

        let task = tokio::spawn(async move {
            let mut agent_inner = Some(agent);
            let turn_result = match agent_inner.as_mut().unwrap().turn(&input_clone, false, None).await {
                Ok(result_text) => TurnResult::Ok(result_text),
                Err(e) => {
                    let err_str = e.to_string();
                    if !err_str.contains("cancelled") {
                        if let Ok(mut ts) = state.lock() {
                            ts.on_error(&err_str);
                        }
                        TurnResult::Err(err_str)
                    } else {
                        TurnResult::Cancelled
                    }
                }
            };

            // On cancellation, don't send completion to pump.
            if turn_result != TurnResult::Cancelled {
                if let TurnResult::Ok(ref t) = turn_result {
                    if t.is_empty() {
                        let fallback = state.lock().map(|s| s.display_text()).unwrap_or_default();
                        if fallback.is_empty() {
                            let _ = done_tx.send((agent_inner, turn_result));
                        } else {
                            let _ = done_tx.send((agent_inner, TurnResult::Ok(fallback)));
                        }
                    } else {
                        let _ = done_tx.send((agent_inner, turn_result));
                    }
                } else {
                    let _ = done_tx.send((agent_inner, turn_result));
                }
            }

            let _ = handle;
        });

        *handle.lock().unwrap() = Some(task.abort_handle());
    }

    /// Check for a finished turn — returns None if empty, Some if there is a completion.
    pub fn poll_completion(&self) -> Option<Completion> {
        self.last_completion.lock().unwrap().take()
    }

    /// Cancel any in-flight turn.
    pub fn cancel_current(&self) {
        if let Some(handle) = self.current_handle.lock().unwrap().take() {
            handle.abort();
        }
        if let Ok(mut ts) = self.state.lock() {
            ts.on_cancel("Turn cancelled by user");
        }
    }

    /// Whether a turn is currently in-flight.
    pub fn is_active(&self) -> bool {
        self.current_handle.lock().unwrap().is_some()
    }
}
