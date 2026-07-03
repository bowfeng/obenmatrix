//! WASM hook adapters — wraps `WasmHookBridge` exports into `Hook` + kind-specific trait
//! implementations.
//!
//! This file defines 7 adapter structs, each holding an `Arc<Mutex<WasmHookBridge>>`,
//! implementing the corresponding hook traits defined below.  Each adapter satisfies
//! the `Hook` base trait (id + priority) and the per-domain trait (e.g. `AgentLoopHooks`).
//!
//! All trait method calls go through `wrap_call`, which catches wasmtime traps and logs
//! via `tracing::warn` but NEVER panics or propagates errors.

use std::sync::{Arc, Mutex};

use wasmtime::Store as WasmStore;

use super::hook_bridge::{WasmHookBridge, WasmResult};
use crate::kind::*;

// ---------------------------------------------------------------------------
// Shared adapter infrastructure
// ---------------------------------------------------------------------------

/// Helper that wraps a WASM hook invocation, catching errors and logging.
///
/// Every adapter method should call this helper rather than calling
/// `try_call_generic` directly, ensuring consistent error handling,
/// logging, and that errors NEVER propagate or panic.
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

/// Common pattern used by all adapters that accept string arguments.
///
/// Builds a closure that logs what it would call (Phase 1 stub) and returns
/// `Ok(())` so methods never panic.  The signature mirrors what the bridge
/// will eventually do with string pointer + length pairs.
fn wrap_call_str<F>(bridge: &Arc<Mutex<WasmHookBridge>>, hook_name: &str, closure: F)
where
    F: FnOnce(&WasmHookBridge) -> WasmResult<()>,
{
    if let Err(e) = wrap_call(bridge, hook_name, |b, _s| {
        closure(b)
    }) {
        tracing::warn!(hook = hook_name, error = %e, "wasm hook failed");
    }
}

// ---------------------------------------------------------------------------
// Adapter 1 — WasmAgentLoopAdapter
// ---------------------------------------------------------------------------

/// Wraps the agent loop lifecycle hooks (on_loop_start, on_loop_end).
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
        wrap_call_str(&self.bridge, "on_loop_start", |_| Ok(()));
    }

    fn on_loop_end(&self, outcome: &str) {
        wrap_call_str(&self.bridge, "on_loop_end", |_| Ok(()));
        let _ = outcome;
    }
}

// ---------------------------------------------------------------------------
// Adapter 2 — WasmTurnLifecycleAdapter
// ---------------------------------------------------------------------------

/// Wraps the per-turn lifecycle hooks (on_pre_turn, on_post_turn).
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
        wrap_call_str(&self.bridge, "on_pre_turn", |_| Ok(()));
    }

    fn on_post_turn(&self, response: &str, success: bool, _turn_count: u32) {
        wrap_call_str(&self.bridge, "on_post_turn", |_| Ok(()));
        let _ = response;
        let _ = success;
    }
}

// ---------------------------------------------------------------------------
// Adapter 3 — WasmToolLifecycleAdapter
// ---------------------------------------------------------------------------

/// Wraps the tool execution lifecycle hooks
/// (on_tool_gen, on_tool_start, on_tool_complete, on_tool_error, on_tool_progress).
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
        wrap_call_str(&self.bridge, "on_tool_gen", |_| Ok(()));
        let _ = tool_name;
        let _ = call_id;
    }

    fn on_tool_start(&self, tool_name: &str, args: &str) {
        wrap_call_str(&self.bridge, "on_tool_start", |_| Ok(()));
        let _ = tool_name;
        let _ = args;
    }

    fn on_tool_complete(&self, tool_name: &str, args: &str, result: &str) {
        wrap_call_str(&self.bridge, "on_tool_complete", |_| Ok(()));
        let _ = tool_name;
        let _ = args;
        let _ = result;
    }

    fn on_tool_error(&self, tool_name: &str, args: &str, error: &str) {
        wrap_call_str(&self.bridge, "on_tool_error", |_| Ok(()));
        let _ = tool_name;
        let _ = args;
        let _ = error;
    }

    fn on_tool_progress(&self, tool_name: &str, preview: &str) {
        wrap_call_str(&self.bridge, "on_tool_progress", |_| Ok(()));
        let _ = tool_name;
        let _ = preview;
    }
}

// ---------------------------------------------------------------------------
// Adapter 4 — WasmStreamingAdapter
// ---------------------------------------------------------------------------

/// Wraps the LLM output streaming hooks
/// (on_stream_delta, on_thinking, on_reasoning, on_interim_assistant).
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
        wrap_call_str(&self.bridge, "on_stream_delta", |_| Ok(()));
        let _ = text;
    }

    fn on_thinking(&self, text: &str) {
        wrap_call_str(&self.bridge, "on_thinking", |_| Ok(()));
        let _ = text;
    }

    fn on_reasoning(&self, text: &str) {
        wrap_call_str(&self.bridge, "on_reasoning", |_| Ok(()));
        let _ = text;
    }

    fn on_interim_assistant(&self, text: &str) {
        wrap_call_str(&self.bridge, "on_interim_assistant", |_| Ok(()));
        let _ = text;
    }
}

// ---------------------------------------------------------------------------
// Adapter 5 — WasmSystemEventsAdapter
// ---------------------------------------------------------------------------

/// Wraps the system events hook (on_status).
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
        wrap_call_str(&self.bridge, "on_status", |_| Ok(()));
        let _ = level;
        let _ = message;
    }
}

// ---------------------------------------------------------------------------
// Adapter 6 — WasmSessionLifecycleAdapter
// ---------------------------------------------------------------------------

/// Wraps the session lifecycle hooks
/// (on_session_rotate, on_compression_start, on_compression_complete).
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
        wrap_call_str(&self.bridge, "on_session_rotate", |_| Ok(()));
        let _ = parent_id;
        let _ = child_id;
    }

    fn on_compression_start(&self, message_count: usize) {
        wrap_call_str(&self.bridge, "on_compression_start", |_| Ok(()));
        let _ = message_count;
    }

    fn on_compression_complete(&self, status: &str) {
        wrap_call_str(&self.bridge, "on_compression_complete", |_| Ok(()));
        let _ = status;
    }
}

// ---------------------------------------------------------------------------
// Adapter 7 — WasmInterruptLifecycleAdapter
// ---------------------------------------------------------------------------

/// Wraps the interrupt lifecycle hooks
/// (on_interrupt_requested, on_interrupted).
pub struct WasmInterruptLifecycleAdapter {
    id: String,
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
        wrap_call_str(&self.bridge, "on_interrupt_requested", |_| Ok(()));
    }

    fn on_interrupted(&self, reason: &str) {
        wrap_call_str(&self.bridge, "on_interrupted", |_| Ok(()));
        let _ = reason;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trait_bounds() {
        // Dummy struct implementing both traits to verify trait bounds compile
        struct DummyAdapter;
        impl Hook for DummyAdapter {
            fn id(&self) -> &str { "dummy" }
        }
        impl AgentLoopHooks for DummyAdapter {}

        let d = DummyAdapter;
        assert_eq!(d.id(), "dummy");
        assert_eq!(d.priority(), 100);
    }
}
