/// Rich callback system for platform integration.
///
/// Mirrors Hermes' 11+ callback parameters consolidated into a single struct.
/// All callbacks are `Option<Box<dyn Fn(...) + Send + Sync>>`.
///
/// Note: `AgentCallbacks` is NOT `Clone` because `Box<dyn Fn>` doesn't impl `Clone`.
/// This is by design - callers create the callbacks once and pass by reference.
///
/// ## Relay Integration
///
/// For subagent delegation, `AgentCallbacks` includes an optional `Arc<CallbacksRelay>`
/// that proxies all callback invocations through a shared relay. This allows the
/// parent process to subscribe to child agent events without cloning closures.
///
/// ```no_run
/// // Parent creates a relay for subagent events
/// use std::sync::Arc;
/// use oben_agent::callbacks::{AgentCallbacks, CallbacksRelay};
/// let relay = CallbacksRelay::new();
///
/// // Subscribers register on the relay
/// relay.subscribe(|msg| {
///     println!("Parent sees: {}", msg);
/// });
///
/// // AgentCallbacks gets the relay
/// let mut callbacks = AgentCallbacks::default();
/// callbacks.with_relay(relay);
///
/// // Now calling callbacks triggers relay subscribers too
/// callbacks.call_tool_progress("shell", "ls");
/// ```

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

/// Callback relay — shared subscriber list for parent<→>child event forwarding.
pub mod relay {
    pub use super::relay_impl::*;
}

// Re-define the relay logic inline so callbacks/ and callbacks.rs coexist.
// (Moving this to a separate file would require restructuring the module tree.)
mod relay_impl {
    use std::sync::{Arc, Mutex};

    /// A thread-safe collection of callbacks for a single event type.
    ///
    /// `Args` is the tuple of arguments passed to each callback.
    /// Subscribers are `Fn(&Args) + Send + Sync`.
    pub struct CallbackRelay<A> {
        subscribers: Mutex<Vec<Arc<dyn Fn(&A) + Send + Sync + 'static>>>,
    }

    impl<A: Clone + Send + Sync + 'static> CallbackRelay<A> {
        pub fn new() -> Self {
            Self {
                subscribers: Mutex::new(Vec::new()),
            }
        }

        pub fn subscribe(&self, subscriber: impl Fn(&A) + Send + Sync + 'static) {
            self.subscribers.lock().unwrap().push(Arc::new(subscriber));
        }

        pub fn unsubscribe(&self, subscriber: &Arc<dyn Fn(&A) + Send + Sync>) -> bool {
            let mut subs = self.subscribers.lock().unwrap();
            let before = subs.len();
            subs.retain(|s| !Arc::ptr_eq(s, subscriber));
            let removed = before - subs.len();
            removed > 0
        }

        pub fn notify(&self, args: &A) {
            let subs = self.subscribers.lock().unwrap();
            for sub in subs.iter() {
                sub(args);
            }
        }

        pub fn clear(&self) {
            self.subscribers.lock().unwrap().clear();
        }

        pub fn len(&self) -> usize {
            self.subscribers.lock().unwrap().len()
        }

        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }
    }

    impl<A: Clone + Send + Sync + 'static> Default for CallbackRelay<A> {
        fn default() -> Self {
            Self::new()
        }
    }

    /// A typed relay for a specific callback event.
    pub struct TypedRelay<A> {
        inner: CallbackRelay<A>,
    }

    impl<A: Clone + Send + Sync + 'static> TypedRelay<A> {
        #[doc(hidden)]
        pub fn new() -> Self {
            Self {
                inner: CallbackRelay::new(),
            }
        }

        pub fn subscribe(&self, subscriber: impl Fn(&A) + Send + Sync + 'static) {
            self.inner.subscribe(subscriber);
        }

        pub fn notify(&self, args: &A) {
            self.inner.notify(args);
        }
    }

    /// Convenience: `Notify` is for `CallbackRelay` to provide a unified API.
    pub trait Notify<A> {
        fn subscribe(&self, subscriber: impl Fn(&A) + Send + Sync + 'static);
        fn notify(&self, args: &A);
    }

    impl<A: Clone + Send + Sync + 'static> Notify<A> for CallbackRelay<A> {
        fn subscribe(&self, subscriber: impl Fn(&A) + Send + Sync + 'static) {
            self.subscribe(subscriber);
        }
        fn notify(&self, args: &A) {
            self.notify(args);
        }
    }

    /// Pre-typed relays for common events (using String to avoid lifetime issues).
    pub type ToolProgressRelay = TypedRelay<String>;
    pub type ToolStartRelay = TypedRelay<String>;
    pub type ToolCompleteRelay = TypedRelay<String>;
    pub type ThinkingRelay = TypedRelay<String>;
    pub type StepRelay = TypedRelay<String>;
    pub type StatusRelay = TypedRelay<String>;
    pub type PrintRelay = TypedRelay<String>;
    pub type ReasoningRelay = TypedRelay<String>;
    pub type StreamDeltaRelay = TypedRelay<String>;
    pub type InterimAssistantRelay = TypedRelay<String>;
    pub type ToolGenRelay = TypedRelay<String>;
    pub type ClarifyRelay = TypedRelay<String>;

    /// All relays collected into a single struct for passing between parent/child agents.
    pub struct CallbacksRelay {
        /// Relay for `tool_progress: (tool_name, args_preview)`.
        pub tool_progress: ToolProgressRelay,
        /// Relay for `tool_start: (tool_name, args_json)`.
        pub tool_start: ToolStartRelay,
        /// Relay for `tool_complete: (tool_name, args_json, result)`.
        pub tool_complete: ToolCompleteRelay,
        /// Relay for `thinking: message`.
        pub thinking: ThinkingRelay,
        /// Relay for `reasoning: message`.
        pub reasoning: ReasoningRelay,
        /// Relay for `clarify: (question, choices) -> answer`.
        pub clarify: ClarifyRelay,
        /// Relay for `step: message`.
        pub step: StepRelay,
        /// Relay for `stream_delta: message`.
        pub stream_delta: StreamDeltaRelay,
        /// Relay for `interim_assistant: message`.
        pub interim_assistant: InterimAssistantRelay,
        /// Relay for `tool_gen: (tool_name, call_id)`.
        pub tool_gen: ToolGenRelay,
        /// Relay for `status: (level, message)`.
        pub status: StatusRelay,
        /// Relay for `vprint: message`.
        pub vprint: PrintRelay,
    }

    impl CallbacksRelay {
        pub fn new() -> Self {
            Self {
                tool_progress: ToolProgressRelay::new(),
                tool_start: ToolStartRelay::new(),
                tool_complete: ToolCompleteRelay::new(),
                thinking: ThinkingRelay::new(),
                reasoning: ReasoningRelay::new(),
                clarify: ClarifyRelay::new(),
                step: StepRelay::new(),
                stream_delta: StreamDeltaRelay::new(),
                interim_assistant: InterimAssistantRelay::new(),
                tool_gen: ToolGenRelay::new(),
                status: StatusRelay::new(),
                vprint: PrintRelay::new(),
            }
        }

        /// Format a tool progress message for relay subscribers.
        pub fn format_progress(name: &str, preview: &str) -> String {
            format!("[tool_progress] {name}: {preview}")
        }

        /// Format a tool start message for relay subscribers.
        pub fn format_tool_start(name: &str, _args: &str) -> String {
            format!("[tool_start] {name}")
        }

        /// Format a tool complete message for relay subscribers.
        pub fn format_tool_complete(name: &str, _args: &str, result: &str) -> String {
            format!("[tool_complete] {name}: {result}")
        }

        /// Format a thinking message for relay subscribers.
        pub fn format_thinking(text: &str) -> String {
            format!("[thinking] {text}")
        }

        /// Format a step message for relay subscribers.
        pub fn format_step(message: &str) -> String {
            format!("[step] {message}")
        }
    }

    impl Default for CallbacksRelay {
        fn default() -> Self {
            Self::new()
        }
    }
}

// ---------------------------------------------------------------------------
// AgentCallbacks — rich callback set plus optional relay
// ---------------------------------------------------------------------------

/// Agent callbacks — rich set for platform integration.
#[derive(Default)]
pub struct AgentCallbacks {
    /// Tool progress: (tool_name, args_preview)
    pub tool_progress: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool started: (tool_name, args_json)
    pub tool_start: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool completed: (tool_name, args_json, result)
    pub tool_complete: Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>,
    /// Thinking/thought stream delta
    pub thinking: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Reasoning stream delta
    pub reasoning: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Clarification request: (question, choices) -> user answer
    pub clarify: Option<Box<dyn Fn(&str, &[String]) -> String + Send + Sync>>,
    /// Step-by-step status message
    pub step: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Token stream delta (for TTS etc.)
    pub stream_delta: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Interim assistant message (non-streaming, full text)
    pub interim_assistant: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Tool generation event: (tool_name, call_id)
    pub tool_gen: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Lifecycle status: (level, message)
    pub status: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Verbose print — always visible even during streaming
    pub vprint: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Optional relay for parent<→>child event forwarding.
    ///
    /// When set, all callback invocations are forwarded through the relay
    /// so that parent subscribers can observe child agent events.
    pub relay: Option<Arc<relay_impl::CallbacksRelay>>,
}

/// Convenience trait to attach a relay to an existing `AgentCallbacks`.
/// Mirrors the fluent-builder pattern in agent construction.
pub trait WithRelay {
    fn with_relay(self, relay: Arc<relay_impl::CallbacksRelay>) -> Self;
}

impl WithRelay for AgentCallbacks {
    fn with_relay(mut self, relay: Arc<relay_impl::CallbacksRelay>) -> Self {
        self.relay = Some(relay);
        self
    }
}

impl AgentCallbacks {
    /// Call tool progress callback.
    pub fn call_tool_progress(&self, tool_name: &str, args_preview: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = relay_impl::CallbacksRelay::format_progress(tool_name, args_preview);
            relay.tool_progress.notify(&formatted);
        }
        if let Some(cb) = &self.tool_progress {
            cb(tool_name, args_preview);
        }
    }

    /// Call tool start callback.
    pub fn call_tool_start(&self, tool_name: &str, args_json: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = relay_impl::CallbacksRelay::format_tool_start(tool_name, args_json);
            relay.tool_start.notify(&formatted);
        }
        if let Some(cb) = &self.tool_start {
            cb(tool_name, args_json);
        }
    }

    /// Call tool complete callback.
    pub fn call_tool_complete(&self, tool_name: &str, args_json: &str, result: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = relay_impl::CallbacksRelay::format_tool_complete(tool_name, args_json, result);
            relay.tool_complete.notify(&formatted);
        }
        if let Some(cb) = &self.tool_complete {
            cb(tool_name, args_json, result);
        }
    }

    /// Call thinking callback.
    pub fn call_thinking(&self, text: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = relay_impl::CallbacksRelay::format_thinking(text);
            relay.thinking.notify(&formatted);
        }
        if let Some(cb) = &self.thinking {
            cb(text);
        }
    }

    /// Call reasoning callback.
    pub fn call_reasoning(&self, text: &str) {
        if let Some(ref relay) = self.relay {
            relay.reasoning.notify(&format!("[reasoning] {text}"));
        }
        if let Some(cb) = &self.reasoning {
            cb(text);
        }
    }

    /// Call clarify callback — returns user answer or empty string.
    pub fn call_clarify(&self, question: &str, choices: &[String]) -> String {
        if let Some(ref relay) = self.relay {
            relay.clarify.notify(&format!("[clarify] question: {question}, choices: {:?}", choices));
        }
        if let Some(cb) = &self.clarify {
            cb(question, choices)
        } else {
            String::new()
        }
    }

    /// Call step callback.
    pub fn call_step(&self, message: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = relay_impl::CallbacksRelay::format_step(message);
            relay.step.notify(&formatted);
        }
        if let Some(cb) = &self.step {
            cb(message);
        }
    }

    /// Call stream delta callback.
    pub fn call_stream_delta(&self, text: &str) {
        if let Some(ref relay) = self.relay {
            relay.stream_delta.notify(&format!("[stream_delta] {text}"));
        }
        if let Some(cb) = &self.stream_delta {
            cb(text);
        }
    }

    /// Call interim assistant callback.
    pub fn call_interim_assistant(&self, text: &str) {
        if let Some(ref relay) = self.relay {
            relay.interim_assistant.notify(&format!("[interim_assistant] {text}"));
        }
        if let Some(cb) = &self.interim_assistant {
            cb(text);
        }
    }

    /// Call tool gen callback.
    pub fn call_tool_gen(&self, tool_name: &str, call_id: &str) {
        if let Some(ref relay) = self.relay {
            relay.tool_gen.notify(&format!("[tool_gen] {tool_name} {call_id}"));
        }
        if let Some(cb) = &self.tool_gen {
            cb(tool_name, call_id);
        }
    }

    /// Call status callback.
    pub fn call_status(&self, level: &str, message: &str) {
        if let Some(ref relay) = self.relay {
            relay.status.notify(&format!("[status] {level}: {message}"));
        }
        if let Some(cb) = &self.status {
            cb(level, message);
        }
    }

    /// Call vprint callback.
    pub fn call_vprint(&self, message: &str) {
        if let Some(ref relay) = self.relay {
            relay.vprint.notify(&format!("[vprint] {message}"));
        }
        if let Some(cb) = &self.vprint {
            cb(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::relay_impl;
    use super::*;

    #[test]
    fn test_default_callbacks_noop() {
        let cb = AgentCallbacks::default();
        cb.call_tool_progress("shell", "ls");
        cb.call_tool_start("shell", "{}");
        cb.call_tool_complete("shell", "{}", "done");
        cb.call_thinking("thinking...");
        cb.call_reasoning("reasoning...");
        cb.call_step("step 1");
        cb.call_stream_delta("hello");
        cb.call_interim_assistant("hi");
        cb.call_tool_gen("shell", "call-1");
        cb.call_status("lifecycle", "started");
        cb.call_vprint("verbose");
        let answer = cb.call_clarify("which one?", &["a".to_string(), "b".to_string()]);
        assert_eq!(answer, "");
    }

    #[test]
    fn test_tool_progress_callback() {
        let invoked = std::sync::Arc::new(std::sync::Mutex::new(false));
        let invoked_clone = invoked.clone();
        let cb = AgentCallbacks {
            tool_progress: Some(Box::new(move |name: &str, preview: &str| {
                assert_eq!(name, "shell");
                assert_eq!(preview, "ls");
                *invoked_clone.lock().unwrap() = true;
            })),
            ..Default::default()
        };
        cb.call_tool_progress("shell", "ls");
        assert!(*invoked.lock().unwrap());
    }

    #[test]
    fn test_clarify_callback_returns_answer() {
        let cb = AgentCallbacks {
            clarify: Some(Box::new(move |_q: &str, _c: &[String]| "A".to_string())),
            ..Default::default()
        };
        assert_eq!(cb.call_clarify("pick one?", &["A".into(), "B".into()]), "A");
    }

    #[test]
    fn test_relay_subscriber_sees_events() {
        let callbacks_relay = relay_impl::CallbacksRelay::new();
        let received = std::sync::Arc::new(std::sync::Mutex::new(None));
        let received_clone = received.clone();

        callbacks_relay.tool_progress.subscribe(move |msg: &String| {
            *received_clone.lock().unwrap() = Some(msg.clone());
        });

        let cb = AgentCallbacks::default()
            .with_relay(Arc::new(callbacks_relay));

        cb.call_tool_progress("shell", "ls");

        let msg = received.lock().unwrap().take().unwrap();
        assert!(msg.contains("tool_progress"));
        assert!(msg.contains("shell"));
    }

    #[test]
    fn test_relay_and_direct_callback_both_fire() {
        let callbacks_relay = relay_impl::CallbacksRelay::new();
        let relay_received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rc = relay_received.clone();

        let direct_invoked = std::sync::Arc::new(std::sync::Mutex::new(false));
        let di = direct_invoked.clone();

        callbacks_relay.tool_progress.subscribe(move |msg: &String| {
            rc.lock().unwrap().push(msg.clone());
        });

        let cb = AgentCallbacks {
            tool_progress: Some(Box::new(move |_n: &str, _p: &str| {
                *di.lock().unwrap() = true;
            })),
            ..Default::default()
        }.with_relay(Arc::new(callbacks_relay));

        cb.call_tool_progress("shell", "ls");

        let msgs = relay_received.lock().unwrap();
        assert!(!msgs.is_empty());
        assert!(msgs.iter().any(|m| m.contains("tool_progress")));

        assert!(*direct_invoked.lock().unwrap());
    }
}
