//! Event bus — dispatches agent events to UI components.
//!
//! The EventBus wraps TurnState and serves as the single event emission point.
//! Callbacks emit events through EventBus which updates the internal TurnState.
//! External consumers subscribe for real-time updates or read the state directly.

use std::sync::{Arc, Mutex};

use crate::turn::turn_state::{ActiveTool, ActivityKind, CompletedTool, TurnPhase, TurnState};

/// Events emitted by the agent during a turn.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Turn started (phase transition + state reset)
    TurnStart,
    /// Incoming text delta from transport stream
    StreamDelta(String),
    /// Tool execution beginning (id, name, context/args)
    ToolStart(String, String, String),
    /// Tool execution finished (id, name, result)
    ToolComplete(String, String, String),
    /// Reasoning/thinking text delta
    Reasoning(String),
    /// Turn completed successfully (outcome string)
    TurnCompleted(String),
    /// Turn failed with error
    TurnError(String),
    /// Status message (level, message) — for logging/activity trace
    Status(String, String),
    /// Verbose print (always-visible message)
    VPrint(String),
    /// Thinking text delta
    Thinking(String),
    /// Interim assistant message
    InterimAssistant(String),
}

/// Subscriber trait for real-time event consumption.
pub trait EventSubscriber: Send + Sync {
    fn on_event(&self, event: &AgentEvent);
}

/// Event bus — wraps TurnState with event emission and subscription.
pub struct EventBus {
    state: Arc<Mutex<TurnState>>,
    subscribers: Vec<Box<dyn EventSubscriber + Send + Sync>>,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TurnState::new())),
            subscribers: Vec::new(),
        }
    }

    /// Clone inner state — shared with subscribers.
    pub fn state(&self) -> Arc<Mutex<TurnState>> {
        Arc::clone(&self.state)
    }

    /// Subscribe to events. The subscriber receives every event as it's emitted.
    pub fn subscribe(&mut self, subscriber: Box<dyn EventSubscriber + Send + Sync>) {
        self.subscribers.push(subscriber);
    }

    /// Emit an event: update TurnState, notify all subscribers.
    pub fn emit(&self, event: AgentEvent) {
        // Update internal state first — subscribers read from this state.
        self.update_state(&event);

        // Notify subscribers.
        for sub in &self.subscribers {
            sub.on_event(&event);
        }
    }

    /// Emit without updating state (e.g., for external sources).
    /// Just dispatches to subscribers.
    pub fn emit_to_subscribers(&self, event: AgentEvent) {
        for sub in &self.subscribers {
            sub.on_event(&event);
        }
    }

    /// Emit events sequentially (batch).
    pub fn emit_many(&self, events: impl IntoIterator<Item = AgentEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    /// Check if a turn is currently in progress.
    pub fn is_turn_active(&self) -> bool {
        self.state.lock().unwrap().is_active()
    }

    /// Get the current streaming phase.
    pub fn phase(&self) -> TurnPhase {
        self.state.lock().unwrap().phase.clone()
    }

    // ── State mutation methods (mirror TurnState for convenience) ──────

    /// Begin a new turn. Emits TurnStart event.
    pub fn begin_turn(&self) {
        self.emit(AgentEvent::TurnStart);
    }

    /// Update streaming text from agent.
    pub fn on_stream_delta(&self, text: &str) {
        let len = text.len();
        self.emit(AgentEvent::StreamDelta(text.to_string()));
        if len > 0 {
            tracing::debug!(
                "[event_bus] on_stream_delta: text.len={} total_streaming_text={}",
                len,
                self.state
                    .lock()
                    .map(|s| s.streaming_text.len())
                    .unwrap_or(0)
            );
        }
    }

    /// Record a tool being started.
    pub fn on_tool_start(&self, tool_id: &str, tool_name: &str, context: &str) {
        self.emit(AgentEvent::ToolStart(
            tool_id.to_string(),
            tool_name.to_string(),
            context.to_string(),
        ));
    }

    /// Record a tool being completed.
    pub fn on_tool_complete(&self, tool_id: &str, tool_name: &str, result: &str) {
        self.emit(AgentEvent::ToolComplete(
            tool_id.to_string(),
            tool_name.to_string(),
            result.to_string(),
        ));
    }

    /// Record reasoning/thinking.
    pub fn on_reasoning(&self, text: &str) {
        self.emit(AgentEvent::Reasoning(text.to_string()));
    }

    /// Record finish.
    pub fn on_turn_completed(&self, outcome: &str) {
        self.emit(AgentEvent::TurnCompleted(outcome.to_string()));
    }

    /// Record error.
    pub fn on_turn_error(&self, error: &str) {
        self.emit(AgentEvent::TurnError(error.to_string()));
    }

    /// Log a status message.
    pub fn on_status(&self, level: &str, message: &str) {
        self.emit(AgentEvent::Status(level.to_string(), message.to_string()));
    }

    /// Verbose print.
    pub fn on_vprint(&self, message: &str) {
        self.emit(AgentEvent::VPrint(message.to_string()));
    }

    /// Thinking delta.
    pub fn on_thinking(&self, text: &str) {
        self.emit(AgentEvent::Thinking(text.to_string()));
    }

    /// Interim assistant.
    pub fn on_interim_assistant(&self, text: &str) {
        self.emit(AgentEvent::InterimAssistant(text.to_string()));
    }

    /// Clear the turn state.
    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.reset();
        }
    }

    // ── Internal: state mutation mirror ─────────────────────────────────

    fn update_state(&self, event: &AgentEvent) {
        if let Ok(mut state) = self.state.lock() {
            match event {
                AgentEvent::TurnStart => {
                    state.phase = TurnPhase::Streaming;
                    // Clear streaming_text at turn start so each new turn
                    // begins with a clean slate. Old streaming_text from a
                    // previous turn is no longer needed — it was either
                    // already committed to message_entries (via done_rx) or
                    // will be overwritten by new deltas in this turn.
                    state.streaming_text.clear();
                    state.active_tools.clear();
                    state.completed_tools.clear();
                    state.activity.clear();
                    state.reasoning_text.clear();
                    state.outcome.clear();
                    state.add_activity(ActivityKind::Info, "Turn started".into());
                }
                AgentEvent::StreamDelta(text) => {
                    state.streaming_text.push_str(text);
                    state.add_activity(
                        ActivityKind::Streaming,
                        format!(
                            "Streaming: {}...",
                            text.chars().take(30).collect::<String>()
                        ),
                    );
                }
                AgentEvent::ToolStart(id, name, context) => {
                    state.active_tools.push(ActiveTool {
                        id: id.clone(),
                        name: name.clone(),
                        started_at: std::time::Instant::now(),
                        context: context.clone(),
                    });
                    state.add_activity(ActivityKind::ToolStart, format!("Running: {name}"));
                }
                AgentEvent::ToolComplete(id, name, result) => {
                    self.finish_tool(&mut state, id, name, result);
                }
                AgentEvent::Reasoning(text) => {
                    state.reasoning_text.push_str(text);
                    let char_count = state.reasoning_text.chars().count();
                    if char_count > 2000 {
                        let skip = char_count - 2000;
                        state.reasoning_text = state.reasoning_text.chars().skip(skip).collect();
                    }
                }
                AgentEvent::TurnCompleted(outcome) => {
                    state.phase = TurnPhase::Completed;
                    state.outcome = outcome.clone();
                    state.active_tools.clear();
                    state.streaming_text.clear();
                    state.add_activity(ActivityKind::Completed, "Turn completed".to_string());
                }
                AgentEvent::TurnError(error) => {
                    state.phase = TurnPhase::Error(error.clone());
                    state.active_tools.clear();
                    state.add_activity(ActivityKind::Error, format!("Error: {error}"));
                }
                AgentEvent::Status(_level, _message) => {
                    // Status messages are informational only.
                    // Could add to activity trace if needed.
                }
                AgentEvent::VPrint(_message) => {
                    // Always-visible message — stored in activity.
                    state.add_activity(ActivityKind::Info, format!("📝 {}", _message));
                }
                AgentEvent::Thinking(_text) => {
                    // Thinking is separate from reasoning — could add a new field.
                    // For now, append to reasoning text.
                    state.reasoning_text.push_str(_text);
                }
                AgentEvent::InterimAssistant(_text) => {
                    state.reasoning_text.push_str(_text);
                }
            }
        }
    }

    fn finish_tool(&self, state: &mut TurnState, id: &str, name: &str, result: &str) {
        let has_error =
            result.to_lowercase().contains("error") || result.to_lowercase().contains("failed");
        let preview: String = if result.chars().count() > 60 {
            result.chars().take(60).collect::<String>() + "..."
        } else {
            result.to_string()
        };
        state.completed_tools.insert(
            0,
            CompletedTool {
                name: name.to_string(),
                output_preview: preview,
                has_error,
            },
        );
        if state.completed_tools.len() > 8 {
            state.completed_tools.truncate(8);
        }
        state.active_tools.retain(|t| t.id != id);
        let action = if has_error { "Error" } else { "Tool complete" };
        state.add_activity(
            if has_error {
                ActivityKind::Error
            } else {
                ActivityKind::ToolComplete
            },
            format!("{action}: {name}"),
        );
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_turn_resets_state() {
        let bus = EventBus::new();
        bus.begin_turn();
        let snap = bus.state.lock().unwrap();
        assert_eq!(snap.phase, TurnPhase::Streaming);
        assert!(snap.streaming_text.is_empty());
        assert!(snap.active_tools.is_empty());
        assert!(snap.completed_tools.is_empty());
    }

    #[test]
    fn test_stream_delta_appends_text() {
        let bus = EventBus::new();
        bus.begin_turn();
        bus.on_stream_delta("Hello ");
        bus.on_stream_delta("world");
        let snap = bus.state.lock().unwrap();
        assert_eq!(snap.streaming_text, "Hello world");
    }

    #[test]
    fn test_tool_start_and_complete() {
        let bus = EventBus::new();
        bus.begin_turn();
        bus.on_tool_start("t1", "shell", "ls -la");
        bus.on_tool_complete("t1", "shell", "output done");
        let snap = bus.state.lock().unwrap();
        assert_eq!(snap.active_tools.len(), 0);
        assert_eq!(snap.completed_tools.len(), 1);
        assert_eq!(snap.completed_tools[0].name, "shell");
    }

    #[test]
    fn test_turn_completed_transitions_phase() {
        let bus = EventBus::new();
        bus.begin_turn();
        bus.on_stream_delta("hello");
        bus.on_turn_completed("done");
        let snap = bus.state.lock().unwrap();
        assert_eq!(snap.phase, TurnPhase::Completed);
        assert_eq!(snap.outcome, "done");
        assert!(snap.streaming_text.is_empty());
    }

    #[test]
    fn test_turn_error_transitions_phase() {
        let bus = EventBus::new();
        bus.begin_turn();
        bus.on_stream_delta("hello");
        bus.on_turn_error("model timeout");
        let snap = bus.state.lock().unwrap();
        assert!(matches!(snap.phase, TurnPhase::Error(_)));
    }

    #[test]
    fn test_subscriber_receives_events() {
        let mut bus = EventBus::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        bus.subscribe(Box::new(TestSubscriber(received_clone)));

        bus.begin_turn();
        bus.on_stream_delta("test");
        bus.on_turn_completed("done");

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], AgentEvent::TurnStart));
        assert!(matches!(&events[1], AgentEvent::StreamDelta(s) if s == "test"));
        assert!(matches!(&events[2], AgentEvent::TurnCompleted(s) if s == "done"));
    }

    #[test]
    fn test_emit_many_sequential() {
        let mut bus = EventBus::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        bus.subscribe(Box::new(TestSubscriber(received_clone)));

        bus.emit_many(vec![
            AgentEvent::TurnStart,
            AgentEvent::StreamDelta("a".into()),
            AgentEvent::StreamDelta("b".into()),
        ]);

        let snap = bus.state.lock().unwrap();
        assert_eq!(snap.streaming_text, "ab");
        {
            let events = received.lock().unwrap();
            assert_eq!(events.len(), 3);
        }
    }

    #[test]
    fn test_turn_active_while_streaming() {
        let bus = EventBus::new();
        bus.begin_turn();
        assert!(bus.is_turn_active());
        bus.on_turn_completed("done");
        assert!(!bus.is_turn_active());
    }

    /// Test subscriber that records all events.
    struct TestSubscriber(Arc<Mutex<Vec<AgentEvent>>>);

    impl EventSubscriber for TestSubscriber {
        fn on_event(&self, event: &AgentEvent) {
            self.0.lock().unwrap().push(event.clone());
        }
    }
}
