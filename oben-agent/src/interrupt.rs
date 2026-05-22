/// Cross-thread interrupt and steer mechanism.
///
/// Mirrors Hermes' `_interrupt_requested` / `_pending_steer` mechanism for
/// gracefully stopping a running turn from another thread (e.g. gateway
/// message receiver, CLI signal handler).

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Thread-safe interrupt state shared between Agent and TurnExecutor.
pub struct InterruptState {
    /// Whether an interrupt was requested.
    interrupted: AtomicBool,
    /// Message that triggered the interrupt (if any).
    message: std::sync::Mutex<Option<String>>,
    /// Pending steer text to inject into the last tool result.
    pending_steer: std::sync::Mutex<Option<String>>,
    /// Cumulative token count across all iterations (updated each turn).
    /// Used for logging/diagnostics.
    _cumulative_tokens: AtomicUsize,
    /// Timestamp of last activity.
    _last_activity: std::sync::Mutex<Instant>,
}

impl InterruptState {
    pub fn new() -> Self {
        Self {
            interrupted: AtomicBool::new(false),
            message: std::sync::Mutex::new(None),
            pending_steer: std::sync::Mutex::new(None),
            _cumulative_tokens: AtomicUsize::new(0),
            _last_activity: std::sync::Mutex::new(Instant::now()),
        }
    }

    /// Check if an interrupt has been requested.
    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::Relaxed)
    }

    /// Set the interrupt flag from another thread.
    pub fn request_interrupt(&self, message: Option<String>) {
        self.interrupted.store(true, Ordering::Release);
        if let Some(msg) = message {
            let mut guard = self.message.lock().unwrap();
            *guard = Some(msg);
        }
    }

    /// Clear the interrupt flag.
    pub fn clear_interrupt(&self) {
        self.interrupted.store(false, Ordering::Release);
        let mut guard = self.message.lock().unwrap();
        *guard = None;
    }

    /// Get the interrupt message (if any), then clear it.
    pub fn drain_interrupt_message(&self) -> Option<String> {
        let mut guard = self.message.lock().unwrap();
        guard.take()
    }

    /// Set pending steer text. Thread-safe: multiple calls before drain
    /// concatenate with newlines.
    pub fn steer(&self, text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }
        let mut guard = self.pending_steer.lock().unwrap();
        let existing = guard.take().unwrap_or_default();
        if existing.is_empty() {
            *guard = Some(trimmed.to_string());
        } else {
            *guard = Some(format!("{}\n{}", existing, trimmed));
        }
        true
    }

    /// Drain pending steer text and clear the slot.
    pub fn drain_pending_steer(&self) -> Option<String> {
        let mut guard = self.pending_steer.lock().unwrap();
        guard.take()
    }

    /// Reset the state for a new turn.
    pub fn reset_for_turn(&self) {
        self.clear_interrupt();
        self.drain_pending_steer();
    }

    /// Update last activity timestamp.
    pub fn touch_activity(&self, desc: &str) {
        let now = Instant::now();
        let mut guard = self._last_activity.lock().unwrap();
        *guard = now;
        // Could also update description if needed
        let _ = desc;
    }

    /// Get activity summary for diagnostics.
    pub fn get_activity_summary(&self) -> ActivitySummary {
        ActivitySummary {
            is_interrupted: self.is_interrupted(),
            last_activity: *self._last_activity.lock().unwrap(),
        }
    }
}

impl std::fmt::Debug for InterruptState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterruptState")
            .field("interrupted", &self.is_interrupted())
            .field("has_message", &self.message.lock().unwrap().is_some())
            .field("has_pending_steer", &self.pending_steer.lock().unwrap().is_some())
            .finish()
    }
}

/// Shared interrupt state — wrap in Arc for sharing between Agent and TurnExecutor.
pub type SharedInterrupt = Arc<InterruptState>;

/// Create a new shared interrupt state.
pub fn shared_interrupt() -> SharedInterrupt {
    Arc::new(InterruptState::new())
}

/// Summary of current activity state.
#[derive(Debug, Clone)]
pub struct ActivitySummary {
    pub is_interrupted: bool,
    pub last_activity: Instant,
}

impl ActivitySummary {
    /// How long since last activity.
    pub fn seconds_since_activity(&self) -> f64 {
        self.last_activity.elapsed().as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_initial_state_not_interrupted() {
        let state = InterruptState::new();
        assert!(!state.is_interrupted());
    }

    #[test]
    fn test_request_interrupt_sets_flag() {
        let state = InterruptState::new();
        state.request_interrupt(Some("stop now".to_string()));
        assert!(state.is_interrupted());
    }

    #[test]
    fn test_clear_interrupt_clears_flag() {
        let state = InterruptState::new();
        state.request_interrupt(Some("stop now".to_string()));
        state.clear_interrupt();
        assert!(!state.is_interrupted());
    }

    #[test]
    fn test_interrupt_message_stored_and_drained() {
        let state = InterruptState::new();
        state.request_interrupt(Some("user interrupted".to_string()));
        assert_eq!(state.drain_interrupt_message(), Some("user interrupted".to_string()));
        assert!(state.drain_interrupt_message().is_none());
    }

    #[test]
    fn test_interrupt_without_message() {
        let state = InterruptState::new();
        state.request_interrupt(None);
        assert!(state.is_interrupted());
        assert!(state.drain_interrupt_message().is_none());
    }

    #[test]
    fn test_steir_accumulates() {
        let state = InterruptState::new();
        assert!(state.steer("first note"));
        assert!(state.steer("second note"));
        let drained = state.drain_pending_steer().unwrap();
        assert_eq!(drained, "first note\nsecond note");
    }

    #[test]
    fn test_steir_ignores_empty() {
        let state = InterruptState::new();
        assert!(!state.steer(""));
        assert!(!state.steer("   "));
    }

    #[test]
    fn test_steir_drain_clears() {
        let state = InterruptState::new();
        state.steer("note");
        assert_eq!(state.drain_pending_steer(), Some("note".to_string()));
        assert!(state.drain_pending_steer().is_none());
    }

    #[test]
    fn test_reset_clears_all() {
        let state = InterruptState::new();
        state.request_interrupt(Some("stop".to_string()));
        state.steer("note");
        state.reset_for_turn();
        assert!(!state.is_interrupted());
        assert!(state.drain_interrupt_message().is_none());
        assert!(state.drain_pending_steer().is_none());
    }

    #[test]
    fn test_thread_safe_interrupt() {
        let state = shared_interrupt();
        let state_for_thread = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(StdDuration::from_millis(50));
            state_for_thread.request_interrupt(Some("from thread".to_string()));
        });

        // Wait for the flag to be set
        thread::sleep(StdDuration::from_millis(100));
        assert!(state.is_interrupted());
        handle.join().unwrap();
    }

    #[test]
    fn test_thread_safe_steir() {
        let state = shared_interrupt();

        let state3 = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(StdDuration::from_millis(50));
            state3.steer("from thread");
        });

        thread::sleep(StdDuration::from_millis(100));
        state.steer("from main");
        let drained = state.drain_pending_steer().unwrap();
        assert!(drained.contains("from thread"));
        assert!(drained.contains("from main"));
        handle.join().unwrap();
    }

    #[test]
    fn test_activity_summary() {
        let state = InterruptState::new();
        state.touch_activity("doing work");
        let summary = state.get_activity_summary();
        assert!(!summary.is_interrupted);
        assert!(summary.seconds_since_activity() < 1.0);
    }

    #[test]
    fn test_shared_interrupt_arc_sharing() {
        let state = shared_interrupt();
        let state2 = Arc::clone(&state);
        state2.request_interrupt(Some("shared".to_string()));
        assert!(state.is_interrupted());
        assert!(state2.is_interrupted());
    }
}
