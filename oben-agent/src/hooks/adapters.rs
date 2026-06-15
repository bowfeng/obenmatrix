use super::kind::*;
use crate::event_bus::EventBus;
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

impl Hook for AgentLoopAdapter {
    fn id(&self) -> &str { "agent_loop" }
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

impl Hook for ToolLifecycleAdapter {
    fn id(&self) -> &str { "tool_lifecycle" }
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

impl Hook for StreamingAdapter {
    fn id(&self) -> &str { "streaming" }
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
}

impl Hook for SystemEventsAdapter {
    fn id(&self) -> &str { "system_events" }
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
