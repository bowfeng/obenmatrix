use super::kind::*;
use crate::event_bus::EventBus;
use std::io::{self, Write};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// AgentLoopAdapter
// ---------------------------------------------------------------------------

pub struct AgentLoopAdapter {
    bus: Arc<EventBus>,
}

impl AgentLoopAdapter {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }
}

impl AgentLoopHooks for AgentLoopAdapter {
    fn on_loop_start(&self) {
        self.bus.begin_turn();
    }
    fn on_loop_end(&self, _outcome: &str) {}
}

// ---------------------------------------------------------------------------
// ToolLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct ToolLifecycleAdapter {
    bus: Arc<EventBus>,
}

impl ToolLifecycleAdapter {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }
}

impl Default for ToolLifecycleAdapter {
    fn default() -> Self {
        Self::new(Arc::new(EventBus::new()))
    }
}

impl ToolLifecycleHooks for ToolLifecycleAdapter {
    fn on_tool_gen(&self, tool_name: &str, call_id: &str) {
        self.bus.on_tool_start(tool_name, call_id, "");
    }
    fn on_tool_start(&self, tool_name: &str, args: &str) {
        self.bus.on_tool_start(tool_name, args, "");
    }
    fn on_tool_complete(&self, tool_name: &str, args: &str, result: &str) {
        self.bus.on_tool_complete(tool_name, args, result);
    }
    fn on_tool_error(&self, tool_name: &str, args: &str, error: &str) {
        self.bus.on_tool_start(tool_name, args, &format!("ERROR: {error}"));
    }
    fn on_tool_progress(&self, _tool_name: &str, _preview: &str) {}
}

// ---------------------------------------------------------------------------
// StreamingAdapter
// ---------------------------------------------------------------------------

pub struct StreamingAdapter {
    bus: Arc<EventBus>,
}

impl StreamingAdapter {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }
}

impl Default for StreamingAdapter {
    fn default() -> Self {
        Self::new(Arc::new(EventBus::new()))
    }
}

impl StreamingHooks for StreamingAdapter {
    fn on_stream_delta(&self, text: &str) {
        self.bus.on_stream_delta(text);
    }
    fn on_thinking(&self, _text: &str) {}
    fn on_reasoning(&self, text: &str) {
        self.bus.on_reasoning(text);
    }
    fn on_interim_assistant(&self, _text: &str) {}
}

// ---------------------------------------------------------------------------
// SystemEventsAdapter
// ---------------------------------------------------------------------------

pub struct SystemEventsAdapter {
    bus: Arc<EventBus>,
}

impl SystemEventsAdapter {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }
}

impl SystemEventsHooks for SystemEventsAdapter {
    fn on_status(&self, level: &str, message: &str) {
        self.bus.on_system_event("status", &format!("[{level}] {message}"));
    }
    fn on_step(&self, message: &str) {
        self.bus.on_system_event("step", message);
    }
    fn on_vprint(&self, message: &str) {
        self.bus.on_system_event("vprint", message);
    }
}

// ---------------------------------------------------------------------------
// CLIInteractionAdapter (noop - CLI doesn't need interactive hooks)
// ---------------------------------------------------------------------------

pub struct CLIInteractionAdapter {
    cliio: bool,
}

impl CLIInteractionAdapter {
    pub fn new_noop() -> Self {
        Self { cliio: false }
    }
    pub fn new_clio() -> Self {
        Self { cliio: true }
    }
}

impl Default for CLIInteractionAdapter {
    fn default() -> Self {
        Self::new_noop()
    }
}

impl CLIInteractionHooks for CLIInteractionAdapter {
    fn on_print_prompt(&self) {
        if self.cliio {
            print!("> ");
            let _ = std::io::stdout().flush();
        }
    }
    fn on_print_flush(&self) {
        if self.cliio {
            let _ = std::io::stdout().flush();
        }
    }
    fn on_print_info(&self, message: &str) {
        if self.cliio {
            println!("{}", message);
        }
    }
    fn on_print_newline(&self) {
        if self.cliio {
            println!();
        }
    }
    fn on_read_input(&self) -> Option<String> {
        if self.cliio {
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_ok() {
                Some(input.trim().to_string())
            } else {
                Some(String::new())
            }
        } else {
            None
        }
    }
    fn on_should_exit(&self, input: &str) -> bool {
        if self.cliio {
            input == "quit" || input == "exit"
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// ClarificationAdapter
// ---------------------------------------------------------------------------

pub struct ClarificationAdapter;

impl ClarificationAdapter {
    pub fn new_noop() -> Self {
        Self
    }
}

impl Default for ClarificationAdapter {
    fn default() -> Self {
        Self::new_noop()
    }
}

impl ClarificationHooks for ClarificationAdapter {
    fn on_clarify(&self, _question: &str, _choices: &[String]) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_lifecycle_adapter_dispatches_error() {
        let bus = Arc::new(EventBus::new());
        let adapter = ToolLifecycleAdapter::new(Arc::clone(&bus));

        adapter.on_tool_gen("shell", "call-1");
        adapter.on_tool_start("shell", "{}");
        adapter.on_tool_complete("shell", "{}", "success");
        adapter.on_tool_error("shell", "{}", "timeout occurred");
        adapter.on_tool_progress("shell", "ls");

        // If we got here without panicking, the dispatcher works
    }
}
