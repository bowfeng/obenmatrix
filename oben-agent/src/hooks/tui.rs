use std::sync::Arc;

use parking_lot::Mutex as PlMutex;

use super::kind::*;

/// Shared state reference for TUI hook adapters.
/// All adapters write directly to the same Arc<Mutex<TurnState>>.
struct TuiState {
    state: Arc<PlMutex<crate::event_bus::TurnState>>,
    next_tool_id: std::sync::atomic::AtomicU32,
}

impl TuiState {
    fn new(state: Arc<PlMutex<crate::event_bus::TurnState>>) -> Self {
        Self {
            state,
            next_tool_id: std::sync::atomic::AtomicU32::new(1),
        }
    }

    fn next_tool_id(&self) -> String {
        self.next_tool_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed).to_string()
    }
}

// ---------------------------------------------------------------------------
// Streaming Adapter — writes deltas directly to TurnState
// ---------------------------------------------------------------------------

pub struct TuiStreamingAdapter {
    state: Arc<TuiState>,
}

impl TuiStreamingAdapter {
    pub fn new(state: Arc<PlMutex<crate::event_bus::TurnState>>) -> Self {
        Self {
            state: Arc::new(TuiState::new(state)),
        }
    }
}

impl Hook for TuiStreamingAdapter {
    fn id(&self) -> &str { "tui_streaming" }
    fn priority(&self) -> u32 { 10 }
}

impl StreamingHooks for TuiStreamingAdapter {
    fn on_stream_delta(&self, text: &str) {
        let mut ts = self.state.state.lock();
        ts.on_stream_delta(text);
    }

    fn on_reasoning(&self, text: &str) {
        let mut ts = self.state.state.lock();
        ts.on_reasoning(text);
    }

    fn on_thinking(&self, _text: &str) {}

    fn on_interim_assistant(&self, _text: &str) {}
}

// ---------------------------------------------------------------------------
// Tool Lifecycle Adapter — writes tool events directly to TurnState
// ---------------------------------------------------------------------------

pub struct TuiToolLifecycleAdapter {
    state: Arc<TuiState>,
}

impl TuiToolLifecycleAdapter {
    pub fn new(state: Arc<PlMutex<crate::event_bus::TurnState>>) -> Self {
        Self {
            state: Arc::new(TuiState::new(state)),
        }
    }
}

impl Hook for TuiToolLifecycleAdapter {
    fn id(&self) -> &str { "tui_tools" }
    fn priority(&self) -> u32 { 10 }
}

impl ToolLifecycleHooks for TuiToolLifecycleAdapter {
    fn on_tool_gen(&self, tool_name: &str, call_id: &str) {
        // tool_gen is informational — skip direct update, let on_tool_start handle it
        let _ = (tool_name, call_id);
    }

    fn on_tool_start(&self, tool_name: &str, args: &str) {
        let tid = self.state.next_tool_id();
        let mut ts = self.state.state.lock();
        ts.on_tool_start(&tid, tool_name, args);
    }

    fn on_tool_complete(&self, tool_name: &str, _args: &str, result: &str) {
        let tid = self.state.next_tool_id();
        let mut ts = self.state.state.lock();
        ts.on_tool_complete(&tid, tool_name, result);
    }

    fn on_tool_error(
        &self,
        _tool_name: &str,
        _args: &str,
        _error: &str,
    ) {
        // Tool errors are captured during tool execution in TurnState.
        // We leave the tool in active_tools to reflect that it didn't complete.
    }

    fn on_tool_progress(&self, _tool_name: &str, _preview: &str) {}
}

// ---------------------------------------------------------------------------
// Agent Loop Adapter — writes turn start/end to TurnState
// ---------------------------------------------------------------------------

pub struct TuiAgentLoopAdapter {
    state: Arc<TuiState>,
}

impl TuiAgentLoopAdapter {
    pub fn new(state: Arc<PlMutex<crate::event_bus::TurnState>>) -> Self {
        Self {
            state: Arc::new(TuiState::new(state)),
        }
    }
}

impl Hook for TuiAgentLoopAdapter {
    fn id(&self) -> &str { "tui_agent_loop" }
    fn priority(&self) -> u32 { 10 }
}

impl AgentLoopHooks for TuiAgentLoopAdapter {
    fn on_loop_start(&self) {
        let mut ts = self.state.state.lock();
        ts.on_turn_start();
    }

    fn on_loop_end(&self, outcome: &str) {
        let mut ts = self.state.state.lock();
        ts.on_completed(outcome);
    }
}

// ---------------------------------------------------------------------------
// Turn Lifecycle Adapter — writes turn start/complete/error to TurnState
// ---------------------------------------------------------------------------

pub struct TuiTurnLifecycleAdapter {
    state: Arc<TuiState>,
}

impl TuiTurnLifecycleAdapter {
    pub fn new(state: Arc<PlMutex<crate::event_bus::TurnState>>) -> Self {
        Self {
            state: Arc::new(TuiState::new(state)),
        }
    }
}

impl Hook for TuiTurnLifecycleAdapter {
    fn id(&self) -> &str { "tui_turn" }
    fn priority(&self) -> u32 { 10 }
}

impl TurnLifecycleHooks for TuiTurnLifecycleAdapter {
    fn on_pre_turn(&self) {
        // Pre-turn is handled inside execute_turn; we start turn state
        // only when on_loop_start fires (which happens before).
    }

    fn on_post_turn(&self, _response: &str, _success: bool) {
        // Post-turn completion is handled by on_loop_end (success) and
        // error paths. Both already update TurnState.
    }
}
