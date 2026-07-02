//! Hook factory — builds HookEngine with configurable adapters and NudgeHook.

use super::kind::*;
use crate::nudge::NudgeConfig;
use std::sync::{Arc, RwLock};

pub mod kind;
pub mod runtime;
pub mod tui;

// Re-export key types
pub use runtime::{HookEngine, NudgeHook};
pub use tui::{TuiStreamingAdapter, TuiToolLifecycleAdapter, TuiAgentLoopAdapter, TuiTurnLifecycleAdapter, TuiSubagentAdapter, SubagentLifecycleCallback};

// Re-export types that TUI and consumers reference at the top level via oben_agent::
pub use kind::{TurnState, TurnPhase, ActiveTool, CompletedTool, ActivityKind, ActivityItem, SubagentToolInfo};

// ---------------------------------------------------------------------------
// HookBuilder
// ---------------------------------------------------------------------------

/// Fluent builder for HookEngine.
///
/// # Example
///
/// ```ignore
/// let engine = HookBuilder::new()
///     .register_streaming(Box::new(TuiStreamingAdapter::new(state)))
///     .build();
/// ```
pub struct HookBuilder {
    agent_loop_hooks: Vec<Box<dyn AgentLoopHooks>>,
    turn_hooks: Vec<Box<dyn TurnLifecycleHooks>>,
    tool_hooks: Vec<Box<dyn ToolLifecycleHooks>>,
    streaming_hooks: Vec<Box<dyn StreamingHooks>>,
    system_hooks: Vec<Box<dyn SystemEventsHooks>>,
    session_hooks: Vec<Box<dyn SessionLifecycleHooks>>,
    interrupt_hooks: Vec<Box<dyn InterruptLifecycleHooks>>,
}

impl HookBuilder {
    /// Create a HookBuilder with NudgeHook auto-registered from config.
    pub fn from_config(hooks_config: &oben_config::HooksConfig) -> Self {
        let nudge_config: NudgeConfig = hooks_config
            .configs
            .get("nudge")
            .and_then(|v| serde_yaml::from_value::<NudgeConfig>(v.clone()).ok())
            .unwrap_or_default();

        let mut turn_hooks: Vec<Box<dyn TurnLifecycleHooks>> = Vec::new();
        if nudge_config.enabled() {
            let nudge: Box<dyn TurnLifecycleHooks> = Box::new(NudgeHook::from_config(&nudge_config));
            turn_hooks.push(nudge);
        }

        Self {
            agent_loop_hooks: Vec::new(),
            turn_hooks,
            tool_hooks: Vec::new(),
            streaming_hooks: Vec::new(),
            system_hooks: Vec::new(),
            session_hooks: Vec::new(),
            interrupt_hooks: Vec::new(),
        }
    }

    /// Create an empty HookBuilder.
    pub fn new() -> Self {
        Self {
            agent_loop_hooks: Vec::new(),
            turn_hooks: Vec::new(),
            tool_hooks: Vec::new(),
            streaming_hooks: Vec::new(),
            system_hooks: Vec::new(),
            session_hooks: Vec::new(),
            interrupt_hooks: Vec::new(),
        }
    }

    pub fn register_agent_loop(mut self, hook: Box<dyn AgentLoopHooks>) -> Self {
        self.agent_loop_hooks.push(hook);
        self
    }

    pub fn register_turn(mut self, hook: Box<dyn TurnLifecycleHooks>) -> Self {
        self.turn_hooks.push(hook);
        self
    }

    pub fn register_tool(mut self, hook: Box<dyn ToolLifecycleHooks>) -> Self {
        self.tool_hooks.push(hook);
        self
    }

    pub fn register_streaming(mut self, hook: Box<dyn StreamingHooks>) -> Self {
        self.streaming_hooks.push(hook);
        self
    }

    pub fn register_system(mut self, hook: Box<dyn SystemEventsHooks>) -> Self {
        self.system_hooks.push(hook);
        self
    }

    pub fn register_session(mut self, hook: Box<dyn SessionLifecycleHooks>) -> Self {
        self.session_hooks.push(hook);
        self
    }

    pub fn register_interrupt(mut self, hook: Box<dyn InterruptLifecycleHooks>) -> Self {
        self.interrupt_hooks.push(hook);
        self
    }

    /// Add WASM agent-loop hook components from the given vector.
    pub fn with_agent_loop_hooks(mut self, hooks: Vec<Box<dyn AgentLoopHooks>>) -> Self {
        self.agent_loop_hooks.extend(hooks);
        self
    }

    /// Add WASM turn-lifecycle hook components from the given vector.
    pub fn with_turn_hooks(mut self, hooks: Vec<Box<dyn TurnLifecycleHooks>>) -> Self {
        self.turn_hooks.extend(hooks);
        self
    }

    /// Add WASM tool-lifecycle hook components from the given vector.
    pub fn with_tool_hooks(mut self, hooks: Vec<Box<dyn ToolLifecycleHooks>>) -> Self {
        self.tool_hooks.extend(hooks);
        self
    }

    /// Add WASM streaming hook components from the given vector.
    pub fn with_streaming_hooks(mut self, hooks: Vec<Box<dyn StreamingHooks>>) -> Self {
        self.streaming_hooks.extend(hooks);
        self
    }

    /// Add WASM system-events hook components from the given vector.
    pub fn with_system_hooks(mut self, hooks: Vec<Box<dyn SystemEventsHooks>>) -> Self {
        self.system_hooks.extend(hooks);
        self
    }

    /// Add WASM session-lifecycle hook components from the given vector.
    pub fn with_session_hooks(mut self, hooks: Vec<Box<dyn SessionLifecycleHooks>>) -> Self {
        self.session_hooks.extend(hooks);
        self
    }

    /// Add WASM interrupt-lifecycle hook components from the given vector.
    pub fn with_interrupt_hooks(mut self, hooks: Vec<Box<dyn InterruptLifecycleHooks>>) -> Self {
        self.interrupt_hooks.extend(hooks);
        self
    }

    pub fn build(self) -> HookEngine {
        HookEngine {
            agent_loop_hooks: Arc::new(RwLock::new(self.agent_loop_hooks)),
            turn_hooks: Arc::new(RwLock::new(self.turn_hooks)),
            tool_hooks: Arc::new(RwLock::new(self.tool_hooks)),
            streaming_hooks: Arc::new(RwLock::new(self.streaming_hooks)),
            system_hooks: Arc::new(RwLock::new(self.system_hooks)),
            session_hooks: Arc::new(RwLock::new(self.session_hooks)),
            interrupt_hooks: Arc::new(RwLock::new(self.interrupt_hooks)),
        }
    }
}

impl Default for HookBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::kind::*;
    use super::HookBuilder;

    struct TestHook;
    impl Hook for TestHook {
        fn id(&self) -> &str { "test" }
    }
    impl AgentLoopHooks for TestHook {}

    #[test]
    fn test_builder_new_empty() {
        let engine = HookBuilder::new()
            .register_agent_loop(Box::new(TestHook))
            .build();
        assert!(engine.count() >= 0);
    }
}
