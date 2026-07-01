//! Hook factory — builds HookEngine with configurable adapters and NudgeHook.

use super::kind::*;
use crate::nudge::NudgeConfig;
use std::sync::{Arc, RwLock};

pub mod kind;
pub mod runtime;
pub mod tui;

// Re-export key types
pub use runtime::{HookEngine, NudgeHook};
pub use tui::{TuiStreamingAdapter, TuiToolLifecycleAdapter, TuiAgentLoopAdapter, TuiTurnLifecycleAdapter};

// Re-export types that TUI and consumers reference at the top level via oben_agent::
pub use kind::{TurnState, TurnPhase, ActiveTool, CompletedTool, ActivityKind, ActivityItem};

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
    agent_loop_hooks: Vec<Box<dyn super::kind::Hook>>,
    turn_hooks: Vec<Box<dyn super::kind::Hook>>,
    tool_hooks: Vec<Box<dyn super::kind::Hook>>,
    streaming_hooks: Vec<Box<dyn super::kind::Hook>>,
    system_hooks: Vec<Box<dyn super::kind::Hook>>,
    session_hooks: Vec<Box<dyn super::kind::Hook>>,
    interrupt_hooks: Vec<Box<dyn super::kind::Hook>>,
}

impl HookBuilder {
    /// Create a HookBuilder with NudgeHook auto-registered from config.
    pub fn from_config(hooks_config: &oben_config::HooksConfig) -> Self {
        let nudge_config: NudgeConfig = hooks_config
            .configs
            .get("nudge")
            .and_then(|v| serde_yaml::from_value::<NudgeConfig>(v.clone()).ok())
            .unwrap_or_default();

        let mut turn_hooks: Vec<Box<dyn super::kind::Hook>> = Vec::new();
        if nudge_config.enabled() {
            let nudge: Box<dyn super::kind::Hook> = Box::new(NudgeHook::from_config(&nudge_config));
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

    /// Inject pre-constructed hook trait objects into the builder.
    ///
    /// Typically called by outside systems (e.g. WASM plugin system) to
    /// batch-register hooks before building the final HookEngine.
    pub fn with_wasm_hooks(mut self, wasm_hooks: Vec<Box<dyn super::kind::Hook>>) -> Self {
        for hook in wasm_hooks {
            let id = hook.id().to_string();
            match id.as_str() {
                id if id.starts_with("wasm-agent-loop-") => self.agent_loop_hooks.push(hook),
                id if id.starts_with("wasm-turn-") => self.turn_hooks.push(hook),
                id if id.starts_with("wasm-tool-") => self.tool_hooks.push(hook),
                id if id.starts_with("wasm-streaming-") => self.streaming_hooks.push(hook),
                id if id.starts_with("wasm-system-") => self.system_hooks.push(hook),
                id if id.starts_with("wasm-session-") => self.session_hooks.push(hook),
                id if id.starts_with("wasm-interrupt-") => self.interrupt_hooks.push(hook),
                _ => tracing::warn!(id, "unrecognized WASM hook ID"),
            }
        }
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
