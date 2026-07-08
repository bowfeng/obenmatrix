use std::sync::{Arc, Mutex};

use wasmtime::Store as WasmStore;

use super::hook_bridge::{WasmHookBridge, WasmResult};
use crate::kind::*;

fn wrap_call<F>(bridge: &Arc<Mutex<WasmHookBridge>>, hook_name: &str, f: F) -> WasmResult<()>
where
    F: FnOnce(&WasmHookBridge, &mut WasmStore<()>) -> WasmResult<()>,
{
    let bridge_guard = bridge.lock();
    let bridge = match bridge_guard {
        Ok(g) => g,
        Err(_poisoned) => {
            tracing::warn!(hook = hook_name, "WASM hook bridge mutex poisoned, skipping");
            return Ok(());
        }
    };

    let mut store = bridge.store();
    match f(&bridge, &mut store) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(hook = hook_name, error = %e, "WASM hook call failed");
            Err(e)
        }
    }
}

pub struct WasmAgentLoopAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmAgentLoopAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-agent-loop-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmAgentLoopAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl AgentLoopHooks for WasmAgentLoopAdapter {
    fn on_loop_start(&self) {
        tracing::debug!("WASM: AgentLoopHooks::on_loop_start (no WIT export, stub)");
    }
    fn on_loop_end(&self, outcome: &str) {
        tracing::debug!(outcome = %outcome, "WASM: AgentLoopHooks::on_loop_end (no WIT export, stub)");
    }
}

pub struct WasmTurnLifecycleAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmTurnLifecycleAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-turn-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmTurnLifecycleAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl TurnLifecycleHooks for WasmTurnLifecycleAdapter {
    fn on_pre_turn(&self) {
        wrap_call(&self.bridge, "on_pre_turn", |b, s| b.try_call_on_pre_turn(s));
    }
    fn on_post_turn(&self, response: &str, success: bool, _turn_count: u32) {
        wrap_call(&self.bridge, "on_post_turn", |b, s| b.try_call_on_post_turn(s, response.to_string(), success));
    }
}

pub struct WasmToolLifecycleAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmToolLifecycleAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-tool-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmToolLifecycleAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl ToolLifecycleHooks for WasmToolLifecycleAdapter {
    fn on_tool_gen(&self, tool_name: &str, call_id: &str) {
        wrap_call(&self.bridge, "on_tool_gen", |b, s| b.try_call_on_tool_gen(s, tool_name.to_string(), call_id.to_string()));
    }
    fn on_tool_start(&self, tool_name: &str, args: &str) {
        wrap_call(&self.bridge, "on_tool_start", |b, s| b.try_call_on_tool_start(s, tool_name.to_string(), args.to_string()));
    }
    fn on_tool_complete(&self, tool_name: &str, args: &str, result: &str) {
        wrap_call(&self.bridge, "on_tool_complete", |b, s| b.try_call_on_tool_complete(s, tool_name.to_string(), args.to_string(), result.to_string()));
    }
    fn on_tool_error(&self, tool_name: &str, args: &str, error: &str) {
        wrap_call(&self.bridge, "on_tool_error", |b, s| b.try_call_on_tool_error(s, tool_name.to_string(), args.to_string(), error.to_string()));
    }
    fn on_tool_progress(&self, tool_name: &str, preview: &str) {
        wrap_call(&self.bridge, "on_tool_progress", |b, s| b.try_call_on_tool_progress(s, tool_name.to_string(), preview.to_string()));
    }
}

pub struct WasmStreamingAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmStreamingAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-streaming-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmStreamingAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl StreamingHooks for WasmStreamingAdapter {
    fn on_stream_delta(&self, text: &str) {
        wrap_call(&self.bridge, "on_stream_delta", |b, s| b.try_call_on_stream_delta(s, text.to_string()));
    }
    fn on_thinking(&self, text: &str) {
        wrap_call(&self.bridge, "on_thinking", |b, s| b.try_call_on_thinking(s, text.to_string()));
    }
    fn on_reasoning(&self, text: &str) {
        wrap_call(&self.bridge, "on_reasoning", |b, s| b.try_call_on_reasoning(s, text.to_string()));
    }
    fn on_interim_assistant(&self, text: &str) {
        wrap_call(&self.bridge, "on_interim_assistant", |b, s| b.try_call_on_interim_assistant(s, text.to_string()));
    }
}

pub struct WasmSystemEventsAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmSystemEventsAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-system-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmSystemEventsAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl SystemEventsHooks for WasmSystemEventsAdapter {
    fn on_status(&self, level: &str, message: &str) {
        wrap_call(&self.bridge, "on_status", |b, s| b.try_call_on_status(s, level.to_string(), message.to_string()));
    }
}

pub struct WasmSessionLifecycleAdapter {
    id: String,
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmSessionLifecycleAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-session-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmSessionLifecycleAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl SessionLifecycleHooks for WasmSessionLifecycleAdapter {
    fn on_session_rotate(&self, parent_id: &str, child_id: &str) {
        wrap_call(&self.bridge, "on_session_rotate", |b, s| b.try_call_on_session_rotate(s, parent_id.to_string(), child_id.to_string()));
    }
    fn on_compression_start(&self, message_count: usize) {
        wrap_call(&self.bridge, "on_compression_start", |b, s| b.try_call_on_compression_start(s, message_count as u32));
    }
    fn on_compression_complete(&self, status: &str) {
        wrap_call(&self.bridge, "on_compression_complete", |b, s| b.try_call_on_compression_complete(s, status.to_string()));
    }
}

pub struct WasmInterruptLifecycleAdapter {
    id: String,
    #[allow(dead_code)]
    bridge: Arc<Mutex<WasmHookBridge>>,
}

impl WasmInterruptLifecycleAdapter {
    pub fn new(name: &str, bridge: Arc<Mutex<WasmHookBridge>>) -> Self {
        Self {
            id: format!("wasm-interrupt-{name}"),
            bridge,
        }
    }
}

impl Hook for WasmInterruptLifecycleAdapter {
    fn id(&self) -> &str { &self.id }
    fn priority(&self) -> u32 { 100 }
}

impl InterruptLifecycleHooks for WasmInterruptLifecycleAdapter {
    fn on_interrupt_requested(&self) {
        tracing::debug!("WASM: InterruptLifecycleHooks::on_interrupt_requested (no WIT export, stub)");
    }
    fn on_interrupted(&self, reason: &str) {
        tracing::debug!(reason = %reason, "WASM: InterruptLifecycleHooks::on_interrupted (no WIT export, stub)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trait_bounds() {
        struct DummyAdapter;
        impl Hook for DummyAdapter {
            fn id(&self) -> &str { "dummy" }
            fn priority(&self) -> u32 { 100 }
        }
        impl AgentLoopHooks for DummyAdapter {}

        let d = DummyAdapter;
        assert_eq!(d.id(), "dummy");
        assert_eq!(d.priority(), 100);
    }
}
