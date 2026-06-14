/// Adapter types that bridge individual Fn closures to the new hook traits.
///
/// These allow gradual migration: existing code that sets `AgentCallbacks {
/// tool_start: Some(Box::new(|name, args| ...)), .. }` continues to work
/// through adapter wrappers.

use super::kind::*;

// ---------------------------------------------------------------------------
// AgentLoopAdapter
// ---------------------------------------------------------------------------

pub struct AgentLoopAdapter {
    pub start: Option<Box<dyn Fn() + Send + Sync>>,
    pub end: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl AgentLoopAdapter {
    pub fn new() -> Self {
        Self {
            start: None,
            end: None,
        }
    }
}

impl Default for AgentLoopAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentLoopHooks for AgentLoopAdapter {
    fn on_loop_start(&self) {
        if let Some(ref start) = self.start {
            start();
        }
    }
    fn on_loop_end(&self, outcome: &str) {
        if let Some(ref end) = self.end {
            end(outcome);
        }
    }
}

// ---------------------------------------------------------------------------
// TurnLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct TurnLifecycleAdapter {
    pub pre: Option<Box<dyn Fn() + Send + Sync>>,
    pub post: Option<Box<dyn Fn(&str, bool) + Send + Sync>>,
}

impl TurnLifecycleAdapter {
    pub fn new() -> Self {
        Self {
            pre: None,
            post: None,
        }
    }
}

impl Default for TurnLifecycleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnLifecycleHooks for TurnLifecycleAdapter {
    fn on_pre_turn(&self) {
        if let Some(ref pre) = self.pre {
            pre();
        }
    }
    fn on_post_turn(&self, response: &str, success: bool) {
        if let Some(ref post) = self.post {
            post(response, success);
        }
    }
}

// ---------------------------------------------------------------------------
// ApiLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct ApiLifecycleAdapter {
    pub start: Option<Box<dyn Fn() + Send + Sync>>,
    pub complete: Option<Box<dyn Fn() + Send + Sync>>,
    pub error: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl ApiLifecycleAdapter {
    pub fn new() -> Self {
        Self {
            start: None,
            complete: None,
            error: None,
        }
    }
}

impl Default for ApiLifecycleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiLifecycleHooks for ApiLifecycleAdapter {
    fn on_api_call_start(&self) {
        if let Some(ref start) = self.start {
            start();
        }
    }
    fn on_api_call_complete(&self) {
        if let Some(ref complete) = self.complete {
            complete();
        }
    }
    fn on_api_call_error(&self, error: &str) {
        if let Some(ref error_cb) = self.error {
            error_cb(error);
        }
    }
}

// ---------------------------------------------------------------------------
// ToolLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct ToolLifecycleAdapter {
    pub gen: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    pub start: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    pub complete: Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>,
    pub progress: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
}

impl ToolLifecycleAdapter {
    pub fn new() -> Self {
        Self {
            gen: None,
            start: None,
            complete: None,
            progress: None,
        }
    }
}

impl Default for ToolLifecycleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolLifecycleHooks for ToolLifecycleAdapter {
    fn on_tool_gen(&self, tool_name: &str, call_id: &str) {
        if let Some(ref gen) = self.gen {
            gen(tool_name, call_id);
        }
    }
    fn on_tool_start(&self, tool_name: &str, args: &str) {
        if let Some(ref start) = self.start {
            start(tool_name, args);
        }
    }
    fn on_tool_complete(&self, tool_name: &str, args: &str, result: &str) {
        if let Some(ref complete) = self.complete {
            complete(tool_name, args, result);
        }
    }
    fn on_tool_progress(&self, tool_name: &str, preview: &str) {
        if let Some(ref progress) = self.progress {
            progress(tool_name, preview);
        }
    }
}

// ---------------------------------------------------------------------------
// StreamingAdapter
// ---------------------------------------------------------------------------

pub struct StreamingAdapter {
    pub delta: Option<Box<dyn Fn(&str) + Send + Sync>>,
    pub thinking: Option<Box<dyn Fn(&str) + Send + Sync>>,
    pub reasoning: Option<Box<dyn Fn(&str) + Send + Sync>>,
    pub interim: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl StreamingAdapter {
    pub fn new() -> Self {
        Self {
            delta: None,
            thinking: None,
            reasoning: None,
            interim: None,
        }
    }
}

impl Default for StreamingAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingHooks for StreamingAdapter {
    fn on_stream_delta(&self, text: &str) {
        if let Some(ref delta) = self.delta {
            delta(text);
        }
    }
    fn on_thinking(&self, text: &str) {
        if let Some(ref thinking) = self.thinking {
            thinking(text);
        }
    }
    fn on_reasoning(&self, text: &str) {
        if let Some(ref reasoning) = self.reasoning {
            reasoning(text);
        }
    }
    fn on_interim_assistant(&self, text: &str) {
        if let Some(ref interim) = self.interim {
            interim(text);
        }
    }
}

// ---------------------------------------------------------------------------
// SystemEventsAdapter
// ---------------------------------------------------------------------------

pub struct SystemEventsAdapter {
    pub status: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    pub step: Option<Box<dyn Fn(&str) + Send + Sync>>,
    pub vprint: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl SystemEventsAdapter {
    pub fn new() -> Self {
        Self {
            status: None,
            step: None,
            vprint: None,
        }
    }
}

impl Default for SystemEventsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemEventsHooks for SystemEventsAdapter {
    fn on_status(&self, level: &str, message: &str) {
        if let Some(ref status) = self.status {
            status(level, message);
        }
    }
    fn on_step(&self, message: &str) {
        if let Some(ref step) = self.step {
            step(message);
        }
    }
    fn on_vprint(&self, message: &str) {
        if let Some(ref vprint) = self.vprint {
            vprint(message);
        }
    }
}

// ---------------------------------------------------------------------------
// SessionLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct SessionLifecycleAdapter {
    pub rotate: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    pub compression_start: Option<Box<dyn Fn(usize) + Send + Sync>>,
    pub compression_complete: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl SessionLifecycleAdapter {
    pub fn new() -> Self {
        Self {
            rotate: None,
            compression_start: None,
            compression_complete: None,
        }
    }
}

impl Default for SessionLifecycleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionLifecycleHooks for SessionLifecycleAdapter {
    fn on_session_rotate(&self, parent_id: &str, child_id: &str) {
        if let Some(ref rotate) = self.rotate {
            rotate(parent_id, child_id);
        }
    }
    fn on_compression_start(&self, message_count: usize) {
        if let Some(ref start) = self.compression_start {
            start(message_count);
        }
    }
    fn on_compression_complete(&self, status: &str) {
        if let Some(ref complete) = self.compression_complete {
            complete(status);
        }
    }
}

// ---------------------------------------------------------------------------
// InterruptLifecycleAdapter
// ---------------------------------------------------------------------------

pub struct InterruptLifecycleAdapter {
    pub requested: Option<Box<dyn Fn() + Send + Sync>>,
    pub interrupted: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl InterruptLifecycleAdapter {
    pub fn new() -> Self {
        Self {
            requested: None,
            interrupted: None,
        }
    }
}

impl Default for InterruptLifecycleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl InterruptLifecycleHooks for InterruptLifecycleAdapter {
    fn on_interrupt_requested(&self) {
        if let Some(ref requested) = self.requested {
            requested();
        }
    }
    fn on_interrupted(&self, reason: &str) {
        if let Some(ref interrupted) = self.interrupted {
            interrupted(reason);
        }
    }
}

// ---------------------------------------------------------------------------
// CLIInteractionAdapter
// ---------------------------------------------------------------------------

pub struct CLIInteractionAdapter {
    pub print_prompt: Option<Box<dyn Fn() + Send + Sync>>,
    pub print_flush: Option<Box<dyn Fn() + Send + Sync>>,
    pub print_info: Option<Box<dyn Fn(&str) + Send + Sync>>,
    pub print_newline: Option<Box<dyn Fn() + Send + Sync>>,
    pub read_input: Option<Box<dyn Fn() -> Option<String> + Send + Sync>>,
    pub should_exit: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
}

impl CLIInteractionAdapter {
    pub fn new() -> Self {
        Self {
            print_prompt: None,
            print_flush: None,
            print_info: None,
            print_newline: None,
            read_input: None,
            should_exit: None,
        }
    }
}

impl Default for CLIInteractionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIInteractionHooks for CLIInteractionAdapter {
    fn on_print_prompt(&self) {
        if let Some(ref cb) = self.print_prompt {
            cb();
        }
    }
    fn on_print_flush(&self) {
        if let Some(ref cb) = self.print_flush {
            cb();
        }
    }
    fn on_print_info(&self, message: &str) {
        if let Some(ref cb) = self.print_info {
            cb(message);
        }
    }
    fn on_print_newline(&self) {
        if let Some(ref cb) = self.print_newline {
            cb();
        }
    }
    fn on_read_input(&self) -> Option<String> {
        if let Some(ref cb) = self.read_input {
            cb()
        } else {
            None
        }
    }
    fn on_should_exit(&self, input: &str) -> bool {
        if let Some(ref cb) = self.should_exit {
            cb(input)
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// ClarificationAdapter
// ---------------------------------------------------------------------------

pub struct ClarificationAdapter {
    pub handler: Option<Box<dyn Fn(&str, &[String]) -> String + Send + Sync>>,
}

impl ClarificationAdapter {
    pub fn new() -> Self {
        Self { handler: None }
    }
}

impl Default for ClarificationAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClarificationHooks for ClarificationAdapter {
    fn on_clarify(&self, question: &str, choices: &[String]) -> String {
        if let Some(ref handler) = self.handler {
            handler(question, choices)
        } else {
            String::new()
        }
    }
}
