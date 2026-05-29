//! Turn controller — manages turn lifecycle states (idle/streaming/completed/interrupted)
//! with live streaming, active tool tracking, and activity feed.
//!
//! Mirrors the TypeScript TurnController from hermes-agent, adapted for Rust direct-call architecture.

use super::event::*;

/// Turn controller — tracks state during a turn execution
pub struct TurnController {
    state: TurnState,
}

impl TurnController {
    pub fn new() -> Self {
        Self {
            state: TurnState::new(),
        }
    }

    /// Start a new turn
    pub fn on_turn_start(&mut self) {
        self.state.on_turn_start();
    }

    /// Tool started
    pub fn on_tool_start(&mut self, tool_id: &str, tool_name: &str, context: &str) {
        self.state.on_tool_start(tool_id, tool_name, context);
    }

    /// Tool completed
    pub fn on_tool_complete(&mut self, tool_id: &str, tool_name: &str, result: &str) {
        self.state.on_tool_complete(tool_id, tool_name, result);
    }

    /// Stream delta received
    pub fn on_stream_delta(&mut self, text: &str) {
        self.state.on_stream_delta(text);
    }

    /// Reasoning text received
    pub fn on_reasoning(&mut self, text: &str) {
        self.state.on_reasoning(text);
    }

    /// Turn completed
    pub fn on_completed(&mut self, outcome: &str) {
        self.state.on_completed(outcome);
    }

    /// Turn error
    pub fn on_error(&mut self, error: &str) {
        self.state.on_error(error);
    }

    /// Turn interrupted
    pub fn on_interrupted(&mut self) {
        self.state.on_interrupted();
    }

    /// Get current state
    pub fn state(&self) -> &TurnState {
        &self.state
    }

    /// Reset completely
    pub fn reset(&mut self) {
        self.state.reset();
    }
}
