use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow;
use super::kind::*;
use crate::ContextWindowManager;
use crate::nudge::NudgeConfig;

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
    pub(crate) agent_loop_hooks: Vec<Box<dyn AgentLoopHooks>>,
    pub(crate) turn_hooks: Vec<Box<dyn TurnLifecycleHooks>>,
    pub(crate) tool_hooks: Vec<Box<dyn ToolLifecycleHooks>>,
    pub(crate) streaming_hooks: std::sync::Arc<std::sync::RwLock<Vec<Box<dyn StreamingHooks>>>>,
    pub(crate) system_hooks: Vec<Box<dyn SystemEventsHooks>>,
    pub(crate) session_hooks: Vec<Box<dyn SessionLifecycleHooks>>,
    pub(crate) interrupt_hooks: Vec<Box<dyn InterruptLifecycleHooks>>,
}

impl HookEngine {
    pub fn new() -> Self {
        Self {
            agent_loop_hooks: Default::default(),
            turn_hooks: Default::default(),
            tool_hooks: Default::default(),
            streaming_hooks: Arc::new(RwLock::new(Default::default())),
            system_hooks: Default::default(),
            session_hooks: Default::default(),
            interrupt_hooks: Default::default(),
        }
    }
    pub fn register_agent_loop(&mut self, hook: Box<dyn AgentLoopHooks>) { self.agent_loop_hooks.push(hook); }
    pub fn register_turn(&mut self, hook: Box<dyn TurnLifecycleHooks>) { self.turn_hooks.push(hook); }
    pub fn register_tool(&mut self, hook: Box<dyn ToolLifecycleHooks>) { self.tool_hooks.push(hook); }
    pub fn register_streaming(&self, hook: Box<dyn StreamingHooks>) { self.streaming_hooks.write().unwrap().push(hook); }
    pub fn register_system(&mut self, hook: Box<dyn SystemEventsHooks>) { self.system_hooks.push(hook); }
    pub fn register_session(&mut self, hook: Box<dyn SessionLifecycleHooks>) { self.session_hooks.push(hook); }
    pub fn register_interrupt(&mut self, hook: Box<dyn InterruptLifecycleHooks>) { self.interrupt_hooks.push(hook); }
    pub fn count(&self) -> usize {
        self.agent_loop_hooks.len() + self.turn_hooks.len() + self.tool_hooks.len()
            + self.streaming_hooks.read().unwrap().len() + self.system_hooks.len() + self.session_hooks.len()
            + self.interrupt_hooks.len()
    }
    pub fn emit_loop_start(&self) { for h in &self.agent_loop_hooks { h.on_loop_start(); } }
    pub fn emit_loop_end(&self, outcome: &str) { for h in &self.agent_loop_hooks { h.on_loop_end(outcome); } }
    pub fn emit_pre_turn(&self) { for h in &self.turn_hooks { h.on_pre_turn(); } }
    pub fn emit_turn_complete(&self, response: &str, _msg_count: usize) { for h in &self.turn_hooks { h.on_post_turn(response, true); } }
    pub fn emit_turn_error(&self, error: &anyhow::Error) { for h in &self.turn_hooks { h.on_post_turn(&error.to_string(), false); } }
    pub fn emit_tool_gen(&self, n: &str, c: &str) { for h in &self.tool_hooks { h.on_tool_gen(n, c); } }
    pub fn emit_tool_start(&self, n: &str, a: &str) { for h in &self.tool_hooks { h.on_tool_start(n, a); } }
    pub fn emit_tool_complete(&self, n: &str, a: &str, r: &str) { for h in &self.tool_hooks { h.on_tool_complete(n, a, r); } }
    pub fn emit_tool_error(&self, n: &str, a: &str, e: &str) { for h in &self.tool_hooks { h.on_tool_error(n, a, e); } }
    pub fn emit_stream_delta(&self, t: &str) { for h in self.streaming_hooks.read().unwrap().iter() { h.on_stream_delta(t); } }
    pub fn emit_thinking(&self, t: &str) { for h in self.streaming_hooks.read().unwrap().iter() { h.on_thinking(t); } }
    pub fn emit_reasoning(&self, t: &str) { for h in self.streaming_hooks.read().unwrap().iter() { h.on_reasoning(t); } }
    pub fn emit_interim_assistant(&self, t: &str) { for h in self.streaming_hooks.read().unwrap().iter() { h.on_interim_assistant(t); } }
    pub fn emit_status(&self, l: &str, m: &str) { for h in &self.system_hooks { h.on_status(l, m); } }
    pub fn emit_session_rotate(&self, p: &str, c: &str) { for h in &self.session_hooks { h.on_session_rotate(p, c); } }
    pub fn emit_compression_start(&self, n: usize) { for h in &self.session_hooks { h.on_compression_start(n); } }
    pub fn emit_compression_complete(&self, s: &str) { for h in &self.session_hooks { h.on_compression_complete(s); } }
    pub fn emit_interrupt_requested(&self) { for h in &self.interrupt_hooks { h.on_interrupt_requested(); } }
    pub fn emit_interrupted(&self, r: &str) { for h in &self.interrupt_hooks { h.on_interrupted(r); } }
    pub fn emit_turn_complete_with_count(&self, response: &str, turn_count: usize, msg_count: usize) {
        for h in &self.turn_hooks { h.on_post_turn(response, true); }
    }
    pub fn post_turn(&self, response: &str, msg_count: usize) {
        self.emit_turn_complete(response, msg_count);
    }
    pub fn turn_count(&self) -> usize {
        self.turn_hooks.len()
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
