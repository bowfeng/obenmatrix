//! Central event bus — pure event dispatcher.
//!
//! **No state in EventBus.** TUI creates `Arc<Mutex<TurnState>>`, wraps it in
//! `TuiSubscriber`, and registers it with EventBus. When Agent callbacks call
//! bus methods (e.g. `bus.on_stream_delta(text)`), EventBus dispatches to all
//! subscribers. `TuiSubscriber` locks the Mutex and updates `TurnState`.
//!
//! TUI polls state directly via the `Arc<Mutex<TurnState>>` — not through EventBus.
//!
//! ```text
//! Agent callbacks ──► EventBus ──► TuiSubscriber ──lock──▶ Mutex<TurnState>
//!                                      ▲
//!                              (TUI polls from same Arc<Mutex>)
//! ```

use parking_lot::Mutex;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

// ─────────────────────────────────────────────────────────────────────────────
// TurnPhase
// ─────────────────────────────────────────────────────────────────────────────

/// Current turn phase.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum TurnPhase {
    #[default]
    Idle,
    Streaming,
    ToolRunning,
    Completed,
    Error(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Activity types and items
// ─────────────────────────────────────────────────────────────────────────────

/// Activity item types for status feed.
#[derive(Debug, Clone)]
pub enum ActivityKind {
    Info,
    ToolStart,
    ToolComplete,
    Streaming,
    Error,
    Completed,
}

/// An entry in the activity/status feed.
#[derive(Debug, Clone)]
pub struct ActivityItem {
    pub kind: ActivityKind,
    pub message: String,
    pub timestamp: Instant,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Active (in-flight) tool call.
#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
    pub started_at: Instant,
    pub context: String,
}

/// Completed tool trail entry.
#[derive(Debug, Clone)]
pub struct CompletedTool {
    pub name: String,
    pub output_preview: String,
    pub has_error: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// TurnState — the state machine
// ─────────────────────────────────────────────────────────────────────────────

/// TurnState — pure state machine, self-updating via EventBus events.
///
/// TurnState **is an EventBus subscriber**. When you call `bus.on_stream_delta(text)`,
/// EventBus notifies all subscribers including TurnState, which updates itself via
/// `on_stream_delta()`.
///
/// `TuiSubscriber` wraps `Arc<Mutex<TurnState>>` to implement `EventSubscriber`
/// cleanly, avoiding the awkwardness of turning `&mut self` into `&self`.
#[derive(Debug)]
pub struct TurnState {
    pub phase: TurnPhase,
    pub streaming_text: String,
    pub active_tools: Vec<ActiveTool>,
    pub completed_tools: Vec<CompletedTool>,
    pub reasoning_text: String,
    pub activity: Vec<ActivityItem>,
    pub outcome: String,
    pub interrupted: bool,
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnState {
    pub fn new() -> Self {
        Self {
            phase: TurnPhase::default(),
            streaming_text: String::new(),
            active_tools: Vec::new(),
            completed_tools: Vec::new(),
            reasoning_text: String::new(),
            activity: Vec::new(),
            outcome: String::new(),
            interrupted: false,
        }
    }

    /// Update method for event dispatch. Called by EventBus when events arrive.
    pub fn on_turn_start(&mut self) {
        self.phase = TurnPhase::Streaming;
        self.streaming_text.clear();
        self.active_tools.clear();
        self.completed_tools.clear();
        self.activity.clear();
        self.reasoning_text.clear();
        self.outcome.clear();
        self.add_activity(ActivityKind::Info, "Turn started".into());
    }

    pub fn on_tool_start(&mut self, tool_id: &str, tool_name: &str, context: &str) {
        self.active_tools.push(ActiveTool {
            id: tool_id.to_string(),
            name: tool_name.to_string(),
            started_at: Instant::now(),
            context: context.to_string(),
        });
        self.add_activity(ActivityKind::ToolStart, format!("Running: {tool_name}"));
    }

    pub fn on_tool_complete(&mut self, tool_id: &str, tool_name: &str, result: &str) {
        let has_error =
            result.to_lowercase().contains("error") || result.to_lowercase().contains("failed");
        let preview: String = if result.chars().count() > 60 {
            result.chars().take(60).collect::<String>() + "..."
        } else {
            result.to_string()
        };
        self.completed_tools.insert(
            0,
            CompletedTool {
                name: tool_name.to_string(),
                output_preview: preview,
                has_error,
            },
        );
        if self.completed_tools.len() > 8 {
            self.completed_tools.truncate(8);
        }
        self.active_tools.retain(|t| t.id != tool_id);
        let status = if has_error { "error" } else { "✅" };
        self.add_activity(
            if has_error { ActivityKind::Error } else { ActivityKind::ToolComplete },
            format!("{status} {tool_name}"),
        );
    }

    pub fn on_stream_delta(&mut self, text: &str) {
        // info!(
        //     "[TurnState::on_stream_delta] text.len={} text='{}' total_after={} phase={:?}",
        //     text.len(),
        //     text,
        //     self.streaming_text.len() + text.len(),
        //     self.phase
        // );
        self.streaming_text.push_str(text);
        self.add_activity(
            ActivityKind::Streaming,
            format!(
                "Streaming: {}...",
                text.chars().take(30).collect::<String>()
            ),
        );
    }

    pub fn on_reasoning(&mut self, text: &str) {
        self.reasoning_text.push_str(text);
        let char_count = self.reasoning_text.chars().count();
        if char_count > 2000 {
            let skip = char_count - 2000;
            self.reasoning_text = self.reasoning_text.chars().skip(skip).collect();
        }
    }

    pub fn on_completed(&mut self, outcome: &str) {
        self.phase = TurnPhase::Completed;
        self.outcome = outcome.to_string();
        self.active_tools.clear();
        self.streaming_text.clear();
        self.add_activity(ActivityKind::Completed, "Turn completed".to_string());
    }

    pub fn on_error(&mut self, error: &str) {
        self.phase = TurnPhase::Error(error.to_string());
        self.active_tools.clear();
        self.add_activity(ActivityKind::Error, format!("Error: {error}"));
    }

    pub fn on_interrupted(&mut self) {
        self.phase = TurnPhase::Idle;
        self.outcome = "interrupted".to_string();
        self.active_tools.clear();
        self.add_activity(ActivityKind::Info, "Turn interrupted".to_string());
    }

    pub fn on_cancel(&mut self, reason: &str) {
        self.on_interrupted();
        self.outcome = reason.to_string();
    }

    pub fn add_activity(&mut self, kind: ActivityKind, message: String) {
        self.activity.push(ActivityItem { kind, message, timestamp: Instant::now() });
        if self.activity.len() > 50 {
            self.activity.drain(0..self.activity.len() - 50);
        }
    }

    /// Get current streaming text (first 1000 chars for UI display).
    pub fn display_text(&self) -> String {
        if self.streaming_text.chars().count() > 1000 {
            self.streaming_text.chars().take(1000).collect::<String>() + "..."
        } else {
            self.streaming_text.clone()
        }
    }

    /// Get active tools (at most 2).
    pub fn active_tool_names(&self) -> Vec<String> {
        self.active_tools.iter().take(2).map(|t| t.name.clone()).collect()
    }

    /// Whether this turn has any active processing.
    pub fn is_active(&self) -> bool {
        !self.active_tools.is_empty()
            || matches!(self.phase, TurnPhase::Streaming | TurnPhase::ToolRunning)
    }

    /// Reset completely.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TuiSubscriber — Arc<Mutex<TurnState>> wrapper implementing EventSubscriber
// ─────────────────────────────────────────────────────────────────────────────

/// Wrapper that converts Arc<Mutex<TurnState>> into an EventBus subscriber.
///
/// When EventBus dispatches events, this calls the wrapped TurnState's
/// `on_*` methods through `Mutex::lock()`.
pub struct TuiSubscriber {
    state: Arc<Mutex<TurnState>>,
}

impl TuiSubscriber {
    pub fn new(state: Arc<Mutex<TurnState>>) -> Self {
        Self { state }
    }

    /// Get shared reference to the inner state Arc for polling.
    pub fn state_ref(&self) -> Arc<Mutex<TurnState>> {
        Arc::clone(&self.state)
    }

    /// Begin a new turn.
    pub fn begin_turn(&self) {
        let mut ts = self.state.lock();
        ts.on_turn_start();
    }

    /// Record a tool being started.
    pub fn on_tool_start(&self, tool_id: &str, tool_name: &str, context: &str) {
        let mut ts = self.state.lock();
        ts.on_tool_start(tool_id, tool_name, context);
    }

    /// Record a tool being completed.
    pub fn on_tool_complete(&self, tool_id: &str, tool_name: &str, result: &str) {
        let mut ts = self.state.lock();
        ts.on_tool_complete(tool_id, tool_name, result);
    }

    /// Update streaming text from agent.
    pub fn on_stream_delta(&self, text: &str) {
        let mut ts = self.state.lock();
        tracing::trace!(delta = text, total_len = ts.streaming_text.len(), "[TuiSubscriber] on_stream_delta");
        ts.on_stream_delta(text);
    }

    /// Record reasoning text.
    pub fn on_reasoning(&self, text: &str) {
        let mut ts = self.state.lock();
        ts.on_reasoning(text);
    }

    /// Record turn completion.
    pub fn on_turn_completed(&self, outcome: &str) {
        let mut ts = self.state.lock();
        ts.on_completed(outcome);
    }

    /// Record turn error.
    pub fn on_turn_error(&self, error: &str) {
        let mut ts = self.state.lock();
        ts.on_error(error);
    }

    /// Clear the turn state.
    pub fn clear(&self) {
        let mut ts = self.state.lock();
        ts.reset();
    }

    /// Check if a turn is currently active.
    pub fn is_turn_active(&self) -> bool {
        self.state.lock().is_active()
    }

    /// Get the current streaming phase.
    pub fn phase(&self) -> TurnPhase {
        self.state.lock().phase.clone()
    }

    /// Record a system/lifecycle event — not response text.
    pub fn on_system_event(&self, _kind: &str, _message: &str) {
        // System events don't append to streaming_text.
        // They are only useful for external subscribers that want to track
        // lifecycle transitions separately from response text.
        // (TuiSubscriber itself doesn't need them in TurnState)
    }
}

impl std::fmt::Debug for TuiSubscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiSubscriber").field("state", &"...").finish()
    }
}

impl EventSubscriber for TuiSubscriber {
    fn on_stream_delta(&self, text: &str) {
        TuiSubscriber::on_stream_delta(self, text);
    }

    fn on_tool_start(&self, tool_id: &str, tool_name: &str, context: &str) {
        TuiSubscriber::on_tool_start(self, tool_id, tool_name, context);
    }

    fn on_tool_complete(&self, tool_id: &str, tool_name: &str, result: &str) {
        TuiSubscriber::on_tool_complete(self, tool_id, tool_name, result);
    }

    fn on_turn_start(&self) {
        TuiSubscriber::begin_turn(self);
    }

    fn on_turn_completed(&self, outcome: &str) {
        TuiSubscriber::on_turn_completed(self, outcome);
    }

    fn on_turn_error(&self, error: &str) {
        TuiSubscriber::on_turn_error(self, error);
    }

    fn on_system_event(&self, kind: &str, message: &str) {
        TuiSubscriber::on_system_event(self, kind, message);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EventSubscriber — receives events directly (for CLI, etc.)
// ─────────────────────────────────────────────────────────────────────────────

/// Subscriber trait — receives events as direct method calls.
///
/// All methods have default no-op implementations.
///
/// **Note:** `TuiSubscriber` wraps `Arc<Mutex<TurnState>>` and implements
/// this trait internally. For CLI/logging subscribers, implement this trait
/// directly.
pub trait EventSubscriber: Send + Sync {
    fn on_stream_delta(&self, _text: &str) {}
    fn on_tool_start(&self, _tool_id: &str, _tool_name: &str, _context: &str) {}
    fn on_tool_complete(&self, _tool_id: &str, _tool_name: &str, _result: &str) {}
    fn on_turn_start(&self) {}
    fn on_turn_completed(&self, _outcome: &str) {}
    fn on_turn_error(&self, _error: &str) {}
    /// System/lifecycle event — not response text.
    fn on_system_event(&self, _kind: &str, _message: &str) {}
}


// ─────────────────────────────────────────────────────────────────────────────
// CliLogSubscriber — prints events to stdout for CLI mode
// ─────────────────────────────────────────────────────────────────────────────

/// Subscriber that prints streaming text, tool results, turn state changes, and
/// system events to stdout.  Register this on the EventBus used by the CLI so
/// all `on_stream_delta`, `on_tool_start`, `on_tool_complete`, `on_turn_start`,
/// `on_turn_completed`, and `on_turn_error` events are visible to the user.
pub struct CliLogSubscriber;

impl EventSubscriber for CliLogSubscriber {
    fn on_stream_delta(&self, text: &str) {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), text.as_bytes());
        let _ = std::io::stdout().flush();
    }
    fn on_tool_start(&self, _tool_id: &str, tool_name: &str, _context: &str) {
        println!("[tool start] {}", tool_name);
    }
    fn on_tool_complete(&self, _tool_id: &str, tool_name: &str, _result: &str) {
        println!("[tool complete] {}", tool_name);
    }
    fn on_turn_start(&self) {
        // newline before new turn for visual separation
    }
    fn on_turn_completed(&self, _outcome: &str) {
        println!();
    }
    fn on_turn_error(&self, error: &str) {
        eprintln!("Error: {}", error);
    }
    fn on_system_event(&self, _kind: &str, _message: &str) {
        // CLI prints system events separately, not interleaved with response text
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EventBus — pure event dispatcher (no state)
// ─────────────────────────────────────────────────────────────────────────────

/// Event bus — pure event dispatcher.
///
/// **No state.** Only holds subscribers. All state lives in subscriber
/// instances (TUI's `TuiSubscriber` / `Arc<Mutex<TurnState>>`, CLI subscriber).
///
/// Usage:
/// ```ignore
/// let bus = EventBus::new();
/// let turn_state = Arc::new(Mutex::new(TurnState::new()));
/// bus.add_subscriber(TuiSubscriber::new(Arc::clone(&turn_state)));
/// bus.add_subscriber(CliLogSubscriber { ... });
///
/// // Agent callbacks:
/// streaming.callback = Box::new(|text| {
///     bus.on_stream_delta(text);
/// });
/// ```
pub struct EventBus {
    subscribers: Vec<Box<dyn EventSubscriber + Send + Sync>>,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
        }
    }

    /// Add a subscriber to event notifications.
    pub fn add_subscriber(&mut self, subscriber: impl EventSubscriber + Send + Sync + 'static) {
        struct Wrapper<S: EventSubscriber + Send + Sync + 'static>(S);
        impl<S: EventSubscriber + Send + Sync + 'static> EventSubscriber for Wrapper<S> {
            fn on_stream_delta(&self, text: &str) {
                self.0.on_stream_delta(text);
            }
            fn on_tool_start(
                &self,
                tool_id: &str,
                tool_name: &str,
                context: &str,
            ) {
                self.0.on_tool_start(tool_id, tool_name, context);
            }
            fn on_tool_complete(
                &self,
                tool_id: &str,
                tool_name: &str,
                result: &str,
            ) {
                self.0.on_tool_complete(tool_id, tool_name, result);
            }
            fn on_turn_start(&self) {
                self.0.on_turn_start();
            }
            fn on_turn_completed(&self, outcome: &str) {
                self.0.on_turn_completed(outcome);
            }
            fn on_turn_error(&self, error: &str) {
                self.0.on_turn_error(error);
            }
            fn on_system_event(&self, kind: &str, message: &str) {
                self.0.on_system_event(kind, message);
            }
        }
        self.subscribers.push(Box::new(Wrapper(subscriber)));
    }

    /// Notify all subscribers — Begin a new turn.
    pub fn begin_turn(&self) {
        for sub in &self.subscribers {
            sub.on_turn_start();
        }
    }

    /// Notify all subscribers — update streaming text from agent.
    pub fn on_stream_delta(&self, text: &str) {
        tracing::trace!(delta = text, subscriber_count = self.subscribers.len(), "[EventBus] on_stream_delta");
        for sub in &self.subscribers {
            sub.on_stream_delta(text);
        }
    }

    /// Notify all subscribers — record a tool being started.
    pub fn on_tool_start(&self, tool_id: &str, tool_name: &str, context: &str) {
        for sub in &self.subscribers {
            sub.on_tool_start(tool_id, tool_name, context);
        }
    }

    /// Notify all subscribers — record a tool being completed.
    pub fn on_tool_complete(&self, tool_id: &str, tool_name: &str, result: &str) {
        for sub in &self.subscribers {
            sub.on_tool_complete(tool_id, tool_name, result);
        }
    }

    /// Notify all subscribers — record turn completion.
    pub fn on_turn_completed(&self, outcome: &str) {
        for sub in &self.subscribers {
            sub.on_turn_completed(outcome);
        }
    }

    /// Notify all subscribers — record turn error.
    pub fn on_turn_error(&self, error: &str) {
        for sub in &self.subscribers {
            sub.on_turn_error(error);
        }
    }

    /// Notify all subscribers — system/lifecycle event (not response text).
    pub fn on_system_event(&self, kind: &str, message: &str) {
        for sub in &self.subscribers {
            sub.on_system_event(kind, message);
        }
    }

    /// Dispatch reasoning — not a subscriber event, TUI handles separately.
    pub fn on_reasoning(&self, _text: &str) {
        // Not dispatched to subscribers — TUI's TuiSubscriber handles
        // reasoning separately. External subs don't care about reasoning.
    }

    /// Clear turn state — not an event, TUI's TuiSubscriber handles separately.
    pub fn clear(&self) {
        // Not an event — TUI's TuiSubscriber handles clearing.
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_subscriber_begin_turn_updates_state() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.begin_turn();
        let state = sub.state_ref();
        let snap = state.lock();
        assert_eq!(snap.phase, TurnPhase::Streaming);
        assert!(snap.streaming_text.is_empty());
        assert!(snap.active_tools.is_empty());
    }

    #[test]
    fn tui_subscriber_stream_delta_appends_text() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.on_stream_delta("Hello ");
        sub.on_stream_delta("world");
        let state = sub.state_ref();
        let snap = state.lock();
        assert_eq!(snap.streaming_text, "Hello world");
    }

    #[test]
    fn tui_subscriber_tool_start_and_complete() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.on_tool_start("t1", "shell", "ls -la");
        sub.on_tool_complete("t1", "shell", "output done");
        let state = sub.state_ref();
        let snap = state.lock();
        assert_eq!(snap.active_tools.len(), 0);
        assert_eq!(snap.completed_tools.len(), 1);
        assert_eq!(snap.completed_tools[0].name, "shell");
    }

    #[test]
    fn tui_subscriber_turn_completed_transitions_phase() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.begin_turn();
        sub.on_stream_delta("hello");
        sub.on_turn_completed("done");
        let state = sub.state_ref();
        let snap = state.lock();
        assert_eq!(snap.phase, TurnPhase::Completed);
        assert_eq!(snap.outcome, "done");
    }

    #[test]
    fn tui_subscriber_turn_error_transitions_phase() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.begin_turn();
        sub.on_turn_error("model timeout");
        let state = sub.state_ref();
        let snap = state.lock();
        assert!(matches!(snap.phase, TurnPhase::Error(_)));
    }

    #[test]
    fn tui_subscriber_turn_active_while_streaming() {
        let sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        sub.begin_turn();
        assert!(sub.is_turn_active());
        sub.on_stream_delta("hello");
        sub.on_turn_completed("done");
        assert!(!sub.is_turn_active());
    }

    #[test]
    fn eventbus_dispatches_to_all_subscribers() {
        struct LogSub {
            logs: Arc<Mutex<Vec<String>>>,
        }
        impl EventSubscriber for LogSub {
            fn on_stream_delta(&self, text: &str) {
                self.logs.lock().push(text.to_string());
            }
            fn on_tool_start(&self, _id: &str, name: &str, _ctx: &str) {
                self.logs.lock().push(format!("tool_start: {name}"));
            }
            fn on_tool_complete(&self, _id: &str, name: &str, _res: &str) {
                self.logs.lock().push(format!("tool_complete: {name}"));
            }
            fn on_turn_start(&self) {
                self.logs.lock().push("turn_start".into());
            }
        }

        let mut bus = EventBus::new();
        let logs = Arc::new(Mutex::new(Vec::new()));
        bus.add_subscriber(LogSub {
            logs: Arc::clone(&logs),
        });

        // Add a TuiSubscriber as well
        let tui_sub = TuiSubscriber::new(Arc::new(Mutex::new(TurnState::new())));
        bus.add_subscriber(tui_sub);

        bus.begin_turn();
        bus.on_stream_delta("hello");
        bus.on_tool_start("t1", "shell", "ls");
        bus.on_tool_complete("t1", "shell", "output");

        let log = logs.lock();
        assert!(log.contains(&"turn_start".to_string()));
        assert!(log.contains(&"hello".to_string()));
        assert!(log.contains(&"tool_start: shell".to_string()));
        assert!(log.contains(&"tool_complete: shell".to_string()));
    }
}
