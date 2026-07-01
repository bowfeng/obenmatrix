use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow;
use super::kind::*;
use crate::ContextWindowManager;
use crate::nudge::NudgeConfig;

// ──────────────────────
// Trait object casting helpers — SAFETY: all queues contain only types that
// implement every subset of Hook traits iterated over by the dispatcher.
// Casting from &dyn Base to &dyn Sub re-binds the fat pointer to a
// more-specific vtable. The underlying vtable pointer layout is identical
// because Rust stores only a single `&self` pointer + vtable pointer in
// every trait object; the vtable contents happen to be compatible when
// the concrete type implements the superset of traits.
// ──────────────────────

#[inline]
unsafe fn cast_agent_loop(hook: &dyn super::kind::Hook) -> &dyn AgentLoopHooks {
    // SAFETY: All hooks in agent_loop_hooks implement AgentLoopHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn AgentLoopHooks>(hook)
}

#[inline]
unsafe fn cast_turn(hook: &dyn super::kind::Hook) -> &dyn TurnLifecycleHooks {
    // SAFETY: All hooks in turn_hooks implement TurnLifecycleHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn TurnLifecycleHooks>(hook)
}

#[inline]
unsafe fn cast_tool(hook: &dyn super::kind::Hook) -> &dyn ToolLifecycleHooks {
    // SAFETY: All hooks in tool_hooks implement ToolLifecycleHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn ToolLifecycleHooks>(hook)
}

#[inline]
unsafe fn cast_streaming(hook: &dyn super::kind::Hook) -> &dyn StreamingHooks {
    // SAFETY: All hooks in streaming_hooks implement StreamingHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn StreamingHooks>(hook)
}

#[inline]
unsafe fn cast_system(hook: &dyn super::kind::Hook) -> &dyn SystemEventsHooks {
    // SAFETY: All hooks in system_hooks implement SystemEventsHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn SystemEventsHooks>(hook)
}

#[inline]
unsafe fn cast_session(hook: &dyn super::kind::Hook) -> &dyn SessionLifecycleHooks {
    // SAFETY: All hooks in session_hooks implement SessionLifecycleHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn SessionLifecycleHooks>(hook)
}

#[inline]
unsafe fn cast_interrupt(hook: &dyn super::kind::Hook) -> &dyn InterruptLifecycleHooks {
    // SAFETY: All hooks in interrupt_hooks implement InterruptLifecycleHooks.
    std::mem::transmute::<&dyn super::kind::Hook, &dyn InterruptLifecycleHooks>(hook)
}

// ---------------------------------------------------------------------------
// NudgeHook — concrete hook that triggers memory/skill reviews
// ---------------------------------------------------------------------------

pub struct NudgeHook {
    config: NudgeConfig,
    turn_count: AtomicUsize,
    has_memory_tools: bool,
    sub_turn_callback: Option<Mutex<Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>>>
}


impl NudgeHook {
    pub fn from_config(config: &NudgeConfig) -> Self {
        Self {
            config: config.clone(),
            turn_count: AtomicUsize::new(0),
            has_memory_tools: false,
            sub_turn_callback: None,
        }
    }

    pub fn from_config_internal(config: &oben_config::HooksConfig) -> Option<Self> {
        config.enabled.iter().any(|t| t == "nudge")
        .then(|| {
            NudgeConfig::default()
        })
        .map(|nc| Self {
            config: nc,
            turn_count: AtomicUsize::new(0),
            has_memory_tools: false,
            sub_turn_callback: None,
        })
    }

    pub fn set_sub_turn_callback<F>(&mut self, f: F)
    where F: Fn(&str) -> anyhow::Result<()> + Send + Sync + 'static {
        self.sub_turn_callback = Some(Mutex::new(Box::new(f)));
    }

    pub fn set_turn_count(&mut self, count: usize) {
        self.turn_count.store(count, Ordering::SeqCst);
    }

    pub fn set_memory_tools(&mut self, v: bool) { self.has_memory_tools = v; }
}

/// Clone a set of callbacks so they can be injected into multiple hooks.
pub fn collect_callbacks() -> Vec<Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync + 'static>> {
    Vec::new()
}

impl Hook for NudgeHook {
    fn id(&self) -> &str { "nudge" }
    fn priority(&self) -> u32 { 10 }
}

impl TurnLifecycleHooks for NudgeHook {
    fn on_pre_turn(&self) {}

    fn on_post_turn(&self, _response: &str, _success: bool) {
        if !self.config.enabled() || !self.has_memory_tools { return; }
        let turns = self.turn_count.fetch_add(1, Ordering::SeqCst);
        let threshold = self.config.memory_nudge_interval;
        if turns < threshold {
            return;
        }
        self.turn_count.store(0, Ordering::SeqCst);
        if let Some(ref callback) = self.sub_turn_callback {
            if let Ok(guard) = callback.lock() {
                let prompt = crate::nudge::build_nudge_prompt(self.config.memory_enabled(), self.config.skill_enabled());
                let _ = guard(&prompt);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hook engine — pure broadcast dispatcher, categorized by kind
// ---------------------------------------------------------------------------

pub struct HookEngine {
    pub(crate) agent_loop_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) turn_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) tool_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) streaming_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) system_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) session_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
    pub(crate) interrupt_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn super::kind::Hook>>>>,
}

impl HookEngine {
    pub fn new() -> Self {
        Self {
            agent_loop_hooks: Arc::new(RwLock::new(Default::default())),
            turn_hooks: Arc::new(RwLock::new(Default::default())),
            tool_hooks: Arc::new(RwLock::new(Default::default())),
            streaming_hooks: Arc::new(RwLock::new(Default::default())),
            system_hooks: Arc::new(RwLock::new(Default::default())),
            session_hooks: Arc::new(RwLock::new(Default::default())),
            interrupt_hooks: Arc::new(RwLock::new(Default::default())),
        }
    }
    pub fn register_agent_loop(&self, hook: Box<dyn AgentLoopHooks>) { self.agent_loop_hooks.write().unwrap().push(hook); }
    pub fn register_turn(&self, hook: Box<dyn TurnLifecycleHooks>) { self.turn_hooks.write().unwrap().push(hook); }
    pub fn register_tool(&self, hook: Box<dyn ToolLifecycleHooks>) { self.tool_hooks.write().unwrap().push(hook); }
    pub fn register_streaming(&self, hook: Box<dyn StreamingHooks>) { self.streaming_hooks.write().unwrap().push(hook); }
    pub fn register_system(&self, hook: Box<dyn SystemEventsHooks>) { self.system_hooks.write().unwrap().push(hook); }
    pub fn register_session(&self, hook: Box<dyn SessionLifecycleHooks>) { self.session_hooks.write().unwrap().push(hook); }
    pub fn register_interrupt(&self, hook: Box<dyn InterruptLifecycleHooks>) { self.interrupt_hooks.write().unwrap().push(hook); }
    /// Inject pre-constructed hook trait objects into the engine.
    ///
    /// WASM plugins register their hooks here. Hooks are dispatched to the
    /// correct queue based on their `id()` prefix (e.g. "wasm-tool-*" goes to
    /// the tool_hooks queue).
    pub fn insert_wasm_hooks(&self, hooks: impl IntoIterator<Item = Box<dyn super::kind::Hook>>) {
        for hook in hooks {
            let id = hook.id().to_string();
            match id.as_str() {
                id if id.starts_with("wasm-agent-loop-") => self.agent_loop_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-turn-") => self.turn_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-tool-") => self.tool_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-streaming-") => self.streaming_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-system-") => self.system_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-session-") => self.session_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-interrupt-") => self.interrupt_hooks.write().unwrap().push(hook),
                _ => tracing::warn!(id, "unrecognized WASM hook ID pattern"),
            }
        }
    }
    pub fn count(&self) -> usize {
        self.agent_loop_hooks.read().unwrap().len() + self.turn_hooks.read().unwrap().len() + self.tool_hooks.read().unwrap().len()
            + self.streaming_hooks.read().unwrap().len() + self.system_hooks.read().unwrap().len() + self.session_hooks.read().unwrap().len()
            + self.interrupt_hooks.read().unwrap().len()
    }
    pub fn emit_loop_start(&self) {
        for raw in self.agent_loop_hooks.read().unwrap().iter() {
            unsafe { cast_agent_loop(raw.as_ref()).on_loop_start(); }
        }
    }
    pub fn emit_loop_end(&self, outcome: &str) {
        for raw in self.agent_loop_hooks.read().unwrap().iter() {
            unsafe { cast_agent_loop(raw.as_ref()).on_loop_end(outcome); }
        }
    }
    pub fn emit_pre_turn(&self) {
        for raw in self.turn_hooks.read().unwrap().iter() {
            unsafe { cast_turn(raw.as_ref()).on_pre_turn(); }
        }
    }
    pub fn emit_turn_complete(&self, response: &str, _msg_count: usize) {
        for raw in self.turn_hooks.read().unwrap().iter() {
            unsafe { cast_turn(raw.as_ref()).on_post_turn(response, true); }
        }
    }
    pub fn emit_turn_error(&self, error: &anyhow::Error) {
        for raw in self.turn_hooks.read().unwrap().iter() {
            unsafe { cast_turn(raw.as_ref()).on_post_turn(&error.to_string(), false); }
        }
    }
    pub fn emit_tool_gen(&self, n: &str, c: &str) {
        for raw in self.tool_hooks.read().unwrap().iter() {
            unsafe { cast_tool(raw.as_ref()).on_tool_gen(n, c); }
        }
    }
    pub fn emit_tool_start(&self, n: &str, a: &str) {
        for raw in self.tool_hooks.read().unwrap().iter() {
            unsafe { cast_tool(raw.as_ref()).on_tool_start(n, a); }
        }
    }
    pub fn emit_tool_complete(&self, n: &str, a: &str, r: &str) {
        for raw in self.tool_hooks.read().unwrap().iter() {
            unsafe { cast_tool(raw.as_ref()).on_tool_complete(n, a, r); }
        }
    }
    pub fn emit_tool_error(&self, n: &str, a: &str, e: &str) {
        for raw in self.tool_hooks.read().unwrap().iter() {
            unsafe { cast_tool(raw.as_ref()).on_tool_error(n, a, e); }
        }
    }
    pub fn emit_stream_delta(&self, t: &str) {
        let streaming_hooks = self.streaming_hooks.read().unwrap();
        tracing::debug!(
            "[emit_stream_delta] streaming_hooks_count={}, delta_len={}",
            streaming_hooks.len(),
            t.chars().count(),
        );
        for raw in streaming_hooks.iter() {
            tracing::debug!(
                "[emit_stream_delta] emitting to hook={} (total {} registered)",
                raw.id(),
                streaming_hooks.len(),
            );
            unsafe { cast_streaming(raw.as_ref()).on_stream_delta(t); }
        }
    }
    pub fn emit_thinking(&self, t: &str) {
        for raw in self.streaming_hooks.read().unwrap().iter() {
            unsafe { cast_streaming(raw.as_ref()).on_thinking(t); }
        }
    }
    pub fn emit_reasoning(&self, t: &str) {
        for raw in self.streaming_hooks.read().unwrap().iter() {
            unsafe { cast_streaming(raw.as_ref()).on_reasoning(t); }
        }
    }
    pub fn emit_interim_assistant(&self, t: &str) {
        for raw in self.streaming_hooks.read().unwrap().iter() {
            unsafe { cast_streaming(raw.as_ref()).on_interim_assistant(t); }
        }
    }
    pub fn emit_status(&self, l: &str, m: &str) {
        for raw in self.system_hooks.read().unwrap().iter() {
            unsafe { cast_system(raw.as_ref()).on_status(l, m); }
        }
    }
    pub fn emit_session_rotate(&self, p: &str, c: &str) {
        for raw in self.session_hooks.read().unwrap().iter() {
            unsafe { cast_session(raw.as_ref()).on_session_rotate(p, c); }
        }
    }
    pub fn emit_compression_start(&self, n: usize) {
        for raw in self.session_hooks.read().unwrap().iter() {
            unsafe { cast_session(raw.as_ref()).on_compression_start(n); }
        }
    }
    pub fn emit_compression_complete(&self, s: &str) {
        for raw in self.session_hooks.read().unwrap().iter() {
            unsafe { cast_session(raw.as_ref()).on_compression_complete(s); }
        }
    }
    pub fn emit_interrupt_requested(&self) {
        for raw in self.interrupt_hooks.read().unwrap().iter() {
            unsafe { cast_interrupt(raw.as_ref()).on_interrupt_requested(); }
        }
    }
    pub fn emit_interrupted(&self, r: &str) {
        for raw in self.interrupt_hooks.read().unwrap().iter() {
            unsafe { cast_interrupt(raw.as_ref()).on_interrupted(r); }
        }
    }
    pub fn emit_turn_complete_with_count(&self, response: &str, _turn_count:usize, _msg_count: usize) {
        for raw in self.turn_hooks.read().unwrap().iter() {
            unsafe { cast_turn(raw.as_ref()).on_post_turn(response, true); }
        }
    }
    pub fn post_turn(&self, response: &str, msg_count: usize) {
        self.emit_turn_complete(response, msg_count);
    }
    pub fn turn_count(&self) -> usize {
        self.turn_hooks.read().unwrap().len()
    }

    /// Execute a sub-turn for a triggered hook.
    ///
    /// Called when `post_turn()` determines a sub-turn should execute.
    pub async fn run_hook_turn(
        prompt: String,
        ctx: &mut dyn ContextWindowManager,
        transport: &dyn oben_models::TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn oben_models::SessionManager,
        session_id: &str,
        call_mode: &oben_models::CallMode,
        conversation_config: &crate::coordinator::ConversationConfig,
    ) -> anyhow::Result<()> {
        let turn_config = crate::turn_executor::TurnConfig {
            retry_config: conversation_config.retry_config.clone(),
            hooks: None,
            fallback_chain: if conversation_config.fallback_configs.is_empty() {
                None
            } else {
                Some(crate::fallback::FallbackChain::new(
                    conversation_config
                        .fallback_configs
                        .iter()
                        .map(|fb| crate::fallback::FallbackConfig {
                            provider: fb.provider.clone(),
                            model: fb.model.clone(),
                            api_key: fb.api_key.clone(),
                            base_url: fb.base_url.clone(),
                        })
                        .collect(),
                ))
            },
            dispatch_config: conversation_config.dispatch_config.clone(),
            max_iterations: conversation_config.max_iterations,
        };
        let result = crate::turn_executor::TurnExecutor::execute_turn_with_config(
            ctx, transport, tools, session_manager, session_id,
            oben_models::Message::user(&prompt),
            call_mode,
            None,
            None,
            turn_config,
        )
        .await?;
        tracing::debug!("Hook sub-turn: {} chars", result.text.chars().count());
        Ok(())
    }
}

impl Default for HookEngine { fn default() -> Self { Self::new() } }
