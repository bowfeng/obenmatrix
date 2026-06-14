/// Agent events — output channel for platform integration.
///
/// Mirrors Hermes' 11+ callback parameters consolidated into a single struct.
/// All callbacks are `Option<Box<dyn Fn(...) + Send + Sync>>`.
///
/// Note: `AgentCallbacks` is NOT `Clone` because `Box<dyn Fn>` doesn't impl `Clone`.
/// This is by design - callers create the events once and pass by reference.
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
/// let mut events = AgentCallbacks::default();
/// events.with_relay(relay);
///
/// // Now calling events triggers relay subscribers too
/// events.call_tool_progress("shell", "ls");
/// ```
use std::sync::Arc;

// Re-export hook traits for use in AgentCallbacks
use crate::hooks::kind::*;
#[cfg(test)]
use crate::hooks::adapters::*;

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

/// Callback relay — shared subscriber list for parent<→>child event forwarding.
pub mod relay {
    pub use super::relay_impl::*;
}

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
// AgentCallbacks — composite of functional lifecycle hooks
// ---------------------------------------------------------------------------

/// Agent callbacks — composite of functional lifecycle hooks plus optional relay.
///
/// Replaces the old 15+ closure field pattern with 10 domain-specific hook traits.
/// Each field is optional — set only the hooks you need.
///
/// For backward compatibility, use the adapter types in `hooks::adapters` to wrap
/// closure-style callbacks. The `CallbackAdapter` below provides a unified adapter
/// that supports all event types and fires relay subscribers.
///
/// ## Relay Integration
///
/// When a relay is set, the adapter internally fires relay subscribers before
/// invoking direct callbacks, matching the old `call_*` behavior.

#[derive(Default)]
pub struct AgentCallbacks {
    pub agent_loop: Option<Box<dyn AgentLoopHooks>>,
    pub turn_lifecycle: Option<Box<dyn TurnLifecycleHooks>>,
    pub api_lifecycle: Option<Box<dyn ApiLifecycleHooks>>,
    pub tool_lifecycle: Option<Box<dyn ToolLifecycleHooks>>,
    pub streaming: Option<Box<dyn StreamingHooks>>,
    pub system_events: Option<Box<dyn SystemEventsHooks>>,
    pub session_lifecycle: Option<Box<dyn SessionLifecycleHooks>>,
    pub interrupt_lifecycle: Option<Box<dyn InterruptLifecycleHooks>>,
    pub cli_interaction: Option<Box<dyn CLIInteractionHooks>>,
    pub clarification: Option<Box<dyn ClarificationHooks>>,
    /// Optional relay for parent<→>child event forwarding.
    pub relay: Option<Arc<relay_impl::CallbacksRelay>>,
}

/// Convenience trait to attach a relay to an existing `AgentCallbacks`.
pub trait WithRelay {
    fn with_relay(self, relay: Arc<relay_impl::CallbacksRelay>) -> Self;
}

impl WithRelay for AgentCallbacks {
    fn with_relay(mut self, relay: Arc<relay_impl::CallbacksRelay>) -> Self {
        self.relay = Some(relay);
        self
    }
}

/// Internal helper to get a relay-formatted string.
fn relay_format(name: &str) -> String {
    format!("[{name}]")
}

impl AgentCallbacks {
    // ── Tool lifecycle ────────────────────────────────────────────────

    pub fn on_tool_progress(&self, tool_name: &str, args_preview: &str) {
        self.fire_relay("tool_progress", &format!("{tool_name}: {args_preview}"));
        if let Some(hook) = &self.tool_lifecycle {
            hook.on_tool_progress(tool_name, args_preview);
        }
    }

    pub fn on_tool_start(&self, tool_name: &str, args_json: &str) {
        self.fire_relay("tool_start", tool_name);
        if let Some(hook) = &self.tool_lifecycle {
            hook.on_tool_start(tool_name, args_json);
        }
    }

    pub fn on_tool_complete(&self, tool_name: &str, args_json: &str, result: &str) {
        self.fire_relay("tool_complete", &format!("{tool_name}: {result}"));
        if let Some(hook) = &self.tool_lifecycle {
            hook.on_tool_complete(tool_name, args_json, result);
        }
    }

    pub fn on_tool_gen(&self, tool_name: &str, call_id: &str) {
        self.fire_relay("tool_gen", &format!("{tool_name} {call_id}"));
        if let Some(hook) = &self.tool_lifecycle {
            hook.on_tool_gen(tool_name, call_id);
        }
    }

    // ── Streaming ─────────────────────────────────────────────────────

    pub fn on_stream_delta(&self, text: &str) {
        self.fire_relay("stream_delta", text);
        if let Some(hook) = &self.streaming {
            hook.on_stream_delta(text);
        }
    }

    pub fn on_thinking(&self, text: &str) {
        self.fire_relay("thinking", text);
        if let Some(hook) = &self.streaming {
            hook.on_thinking(text);
        }
    }

    pub fn on_reasoning(&self, text: &str) {
        self.fire_relay("reasoning", text);
        if let Some(hook) = &self.streaming {
            hook.on_reasoning(text);
        }
    }

    pub fn on_interim_assistant(&self, text: &str) {
        self.fire_relay("interim_assistant", text);
        if let Some(hook) = &self.streaming {
            hook.on_interim_assistant(text);
        }
    }

    // ── Clarification ─────────────────────────────────────────────────

    pub fn on_clarify(&self, question: &str, choices: &[String]) -> String {
        self.fire_relay("clarify", &format!("question: {question}, choices: {choices:?}"));
        if let Some(hook) = &self.clarification {
            hook.on_clarify(question, choices)
        } else {
            String::new()
        }
    }

    // ── System events ─────────────────────────────────────────────────

    pub fn on_step(&self, message: &str) {
        self.fire_relay("step", message);
        if let Some(hook) = &self.system_events {
            hook.on_step(message);
        }
    }

    pub fn on_status(&self, level: &str, message: &str) {
        self.fire_relay("status", &format!("{level}: {message}"));
        if let Some(hook) = &self.system_events {
            hook.on_status(level, message);
        }
    }

    pub fn on_vprint(&self, message: &str) {
        self.fire_relay("vprint", message);
        if let Some(hook) = &self.system_events {
            hook.on_vprint(message);
        }
    }

    // ── CLI interaction ───────────────────────────────────────────────

    pub fn on_print_prompt(&self) {
        if let Some(hook) = &self.cli_interaction {
            hook.on_print_prompt();
        }
    }

    pub fn on_print_flush(&self) {
        if let Some(hook) = &self.cli_interaction {
            hook.on_print_flush();
        }
    }

    pub fn on_print_info(&self, message: &str) {
        if let Some(hook) = &self.cli_interaction {
            hook.on_print_info(message);
        }
    }

    pub fn on_print_newline(&self) {
        if let Some(hook) = &self.cli_interaction {
            hook.on_print_newline();
        }
    }

    pub fn on_read_input(&self) -> Option<String> {
        if let Some(hook) = &self.cli_interaction {
            hook.on_read_input()
        } else {
            None
        }
    }

    pub fn on_should_exit(&self, input: &str) -> bool {
        if let Some(hook) = &self.cli_interaction {
            hook.on_should_exit(input)
        } else {
            false
        }
    }

    // ── Agent loop ────────────────────────────────────────────────────

    pub fn on_loop_start(&self) {
        if let Some(hook) = &self.agent_loop {
            hook.on_loop_start();
        }
    }

    pub fn on_loop_end(&self, outcome: &str) {
        if let Some(hook) = &self.agent_loop {
            hook.on_loop_end(outcome);
        }
    }

    // ── Turn lifecycle ────────────────────────────────────────────────

    pub fn on_pre_turn(&self) {
        if let Some(hook) = &self.turn_lifecycle {
            hook.on_pre_turn();
        }
    }

    pub fn on_post_turn(&self, response: &str, success: bool) {
        if let Some(hook) = &self.turn_lifecycle {
            hook.on_post_turn(response, success);
        }
    }

    // ── API lifecycle ─────────────────────────────────────────────────

    pub fn on_api_call_start(&self) {
        self.fire_relay("api_call_start", "");
        if let Some(hook) = &self.api_lifecycle {
            hook.on_api_call_start();
        }
    }

    pub fn on_api_call_complete(&self) {
        self.fire_relay("api_call_complete", "");
        if let Some(hook) = &self.api_lifecycle {
            hook.on_api_call_complete();
        }
    }

    pub fn on_api_call_error(&self, error: &str) {
        self.fire_relay("api_call_error", error);
        if let Some(hook) = &self.api_lifecycle {
            hook.on_api_call_error(error);
        }
    }

    // ── Session lifecycle ────────────────────────────────────────────

    pub fn on_session_rotate(&self, parent_id: &str, child_id: &str) {
        if let Some(hook) = &self.session_lifecycle {
            hook.on_session_rotate(parent_id, child_id);
        }
    }

    pub fn on_compression_start(&self, message_count: usize) {
        if let Some(hook) = &self.session_lifecycle {
            hook.on_compression_start(message_count);
        }
    }

    pub fn on_compression_complete(&self, status: &str) {
        if let Some(hook) = &self.session_lifecycle {
            hook.on_compression_complete(status);
        }
    }

    // ── Interrupt lifecycle ───────────────────────────────────────────

    pub fn on_interrupt_requested(&self) {
        if let Some(hook) = &self.interrupt_lifecycle {
            hook.on_interrupt_requested();
        }
    }

    pub fn on_interrupted(&self, reason: &str) {
        if let Some(hook) = &self.interrupt_lifecycle {
            hook.on_interrupted(reason);
        }
    }

    // ── Internal ──────────────────────────────────────────────────────

    fn fire_relay(&self, event_type: &str, data: &str) {
        if let Some(ref relay) = self.relay {
            let formatted = format!("[{event_type}] {data}");
            match event_type {
                "tool_progress" => relay.tool_progress.notify(&formatted),
                "tool_start" => relay.tool_start.notify(&formatted),
                "tool_complete" => relay.tool_complete.notify(&formatted),
                "thinking" => relay.thinking.notify(&formatted),
                "reasoning" => relay.reasoning.notify(&formatted),
                "clarify" => relay.clarify.notify(&formatted),
                "step" => relay.step.notify(&formatted),
                "stream_delta" => relay.stream_delta.notify(&formatted),
                "interim_assistant" => relay.interim_assistant.notify(&formatted),
                "tool_gen" => relay.tool_gen.notify(&formatted),
                "status" => relay.status.notify(&formatted),
                "vprint" => relay.vprint.notify(&formatted),
                _ => {}
            }
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
        cb.on_tool_progress("shell", "ls");
        cb.on_tool_start("shell", "{}");
        cb.on_tool_complete("shell", "{}", "done");
        cb.on_tool_gen("shell", "call-1");
        cb.on_stream_delta("hello");
        cb.on_interim_assistant("hi");
        cb.on_step("step 1");
        cb.on_status("lifecycle", "started");
        cb.on_vprint("verbose");
        cb.on_clarify("which one?", &["a".to_string(), "b".to_string()]);
        cb.on_loop_start();
        cb.on_loop_end("completed");
        cb.on_pre_turn();
        cb.on_post_turn("done", true);
    }

    #[test]
    fn test_tool_lifecycle_hook() {
        let invoked = std::sync::Arc::new(std::sync::Mutex::new(false));
        let invoked_clone = invoked.clone();
        let cb = AgentCallbacks {
            tool_lifecycle: Some(Box::new(ToolLifecycleAdapter {
                start: Some(Box::new(move |name: &str, _args: &str| {
                    assert_eq!(name, "shell");
                    *invoked_clone.lock().unwrap() = true;
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        cb.on_tool_start("shell", "{}");
        assert!(*invoked.lock().unwrap());
    }

    #[test]
    fn test_clarify_returns_answer() {
        let cb = AgentCallbacks {
            clarification: Some(Box::new(ClarificationAdapter {
                handler: Some(Box::new(|_q: &str, _c: &[String]| "A".to_string())),
            })),
            ..Default::default()
        };
        assert_eq!(cb.on_clarify("pick one?", &["A".into(), "B".into()]), "A");
    }

    #[test]
    fn test_relay_subscriber_sees_events() {
        let callbacks_relay = relay_impl::CallbacksRelay::new();
        let received = std::sync::Arc::new(std::sync::Mutex::new(None));
        let received_clone = received.clone();

        callbacks_relay
            .tool_progress
            .subscribe(move |msg: &String| {
                *received_clone.lock().unwrap() = Some(msg.clone());
            });

        let cb = AgentCallbacks::default().with_relay(Arc::new(callbacks_relay));

        cb.on_tool_progress("shell", "ls");

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

        callbacks_relay
            .tool_start
            .subscribe(move |msg: &String| {
                rc.lock().unwrap().push(msg.clone());
            });

        let cb = AgentCallbacks {
            tool_lifecycle: Some(Box::new(ToolLifecycleAdapter {
                start: Some(Box::new(move |_n: &str, _p: &str| {
                    *di.lock().unwrap() = true;
                })),
                ..Default::default()
            })),
            ..Default::default()
        }
        .with_relay(Arc::new(callbacks_relay));

        cb.on_tool_start("shell", "ls");

        let msgs = relay_received.lock().unwrap();
        assert!(!msgs.is_empty());
        assert!(msgs.iter().any(|m| m.contains("tool_start")));

        assert!(*direct_invoked.lock().unwrap());
    }

    #[test]
    fn test_streaming_hook() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rc = received.clone();
        let cb = AgentCallbacks {
            streaming: Some(Box::new(StreamingAdapter {
                delta: Some(Box::new(move |text: &str| {
                    rc.lock().unwrap().push(text.to_string());
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        cb.on_stream_delta("hello");
        let msgs = received.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "hello");
    }

    #[test]
    fn test_system_events_hook() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rc = received.clone();
        let cb = AgentCallbacks {
            system_events: Some(Box::new(SystemEventsAdapter {
                status: Some(Box::new(move |level: &str, msg: &str| {
                    rc.lock().unwrap().push(format!("{level}: {msg}"));
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        cb.on_status("warn", "disk full");
        let msgs = received.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "warn: disk full");
    }

    #[test]
    fn test_agent_loop_hooks() {
        let started = std::sync::Arc::new(std::sync::Mutex::new(false));
        let outcome = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sc = started.clone();
        let oc = outcome.clone();
        let cb = AgentCallbacks {
            agent_loop: Some(Box::new(AgentLoopAdapter {
                start: Some(Box::new(move || *sc.lock().unwrap() = true)),
                end: Some(Box::new(move |o: &str| *oc.lock().unwrap() = o.to_string())),
            })),
            ..Default::default()
        };
        cb.on_loop_start();
        assert!(*started.lock().unwrap());
        cb.on_loop_end("interrupted");
        assert_eq!(*outcome.lock().unwrap(), "interrupted");
    }
}
