// Hook traits organized by kind. Each trait represents a domain of agent
// lifecycle events. HookEngine maintains separate queues per kind so that
// broadcast dispatch is precise and type-safe.

use std::time::Instant;

// ─────────────────────────────────────────────────────────────────────────────
// TurnPhase
// ─────────────────────────────────────────────────────────────────────────────

/// Current turn phase.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum TurnPhase {
    #[default]
    Idle,
    Streaming,
    ToolRunning,
    Completed,
    Error(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Activity types and items
// ─────────────────────────────────────────────────────────────────────────────

/// Activity item types for status feed.
#[derive(Debug, Clone)]
pub enum ActivityKind {
    Info,
    ToolStart,
    ToolComplete,
    Streaming,
    Error,
    Completed,
}

/// An entry in the activity/status feed.
#[derive(Debug, Clone)]
pub struct ActivityItem {
    pub kind: ActivityKind,
    pub message: String,
    pub timestamp: Instant,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Active (in-flight) tool call.
#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
    pub started_at: Instant,
    pub context: String,
}

/// Completed tool trail entry.
#[derive(Debug, Clone)]
pub struct CompletedTool {
    pub name: String,
    pub output_preview: String,
    pub has_error: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// TurnState — the state machine
// ─────────────────────────────────────────────────────────────────────────────

/// TurnState — pure state machine, updated by hook adapters.
///
/// All TUI adapters write directly to TurnState via `Arc<Mutex<TurnState>>`.
#[derive(Debug)]
pub struct TurnState {
    pub phase: TurnPhase,
    pub streaming_text: String,
    pub active_tools: Vec<ActiveTool>,
    pub completed_tools: Vec<CompletedTool>,
    pub reasoning_text: String,
    pub activity: Vec<ActivityItem>,
    pub outcome: String,
    pub interrupted: bool,
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnState {
    pub fn new() -> Self {
        Self {
            phase: TurnPhase::default(),
            streaming_text: String::new(),
            active_tools: Vec::new(),
            completed_tools: Vec::new(),
            reasoning_text: String::new(),
            activity: Vec::new(),
            outcome: String::new(),
            interrupted: false,
        }
    }

    /// Update method for hook dispatch. Called by hook adapters when events arrive.
    pub fn on_turn_start(&mut self) {
        self.phase = TurnPhase::Streaming;
        self.streaming_text.clear();
        self.active_tools.clear();
        self.completed_tools.clear();
        self.activity.clear();
        self.reasoning_text.clear();
        self.outcome.clear();
        self.add_activity(ActivityKind::Info, "Turn started".into());
    }

    pub fn on_tool_start(&mut self, tool_id: &str, tool_name: &str, context: &str) {
        self.active_tools.push(ActiveTool {
            id: tool_id.to_string(),
            name: tool_name.to_string(),
            started_at: Instant::now(),
            context: context.to_string(),
        });
        self.add_activity(ActivityKind::ToolStart, format!("Running: {tool_name}"));
    }

    pub fn on_tool_complete(&mut self, tool_id: &str, tool_name: &str, result: &str) {
        let has_error =
            result.to_lowercase().contains("error") || result.to_lowercase().contains("failed");
        let preview: String = if result.chars().count() > 60 {
            result.chars().take(60).collect::<String>() + "..."
        } else {
            result.to_string()
        };
        self.completed_tools.insert(
            0,
            CompletedTool {
                name: tool_name.to_string(),
                output_preview: preview,
                has_error,
            },
        );
        if self.completed_tools.len() > 8 {
            self.completed_tools.truncate(8);
        }
        self.active_tools.retain(|t| t.id != tool_id);
        let status = if has_error { "error" } else { "✅" };
        self.add_activity(
            if has_error { ActivityKind::Error } else { ActivityKind::ToolComplete },
            format!("{status} {tool_name}"),
        );
    }

    pub fn on_stream_delta(&mut self, text: &str) {
        self.streaming_text.push_str(text);
        self.add_activity(
            ActivityKind::Streaming,
            format!(
                "Streaming: {}...",
                text.chars().take(30).collect::<String>()
            ),
        );
    }

    pub fn on_reasoning(&mut self, text: &str) {
        self.reasoning_text.push_str(text);
        let char_count = self.reasoning_text.chars().count();
        if char_count > 2000 {
            let skip = char_count - 2000;
            self.reasoning_text = self.reasoning_text.chars().skip(skip).collect();
        }
    }

    pub fn on_completed(&mut self, outcome: &str) {
        self.phase = TurnPhase::Completed;
        self.outcome = outcome.to_string();
        self.active_tools.clear();
        // Don't clear completed_tools or reasoning_text here — the TUI flushes them
        // to message_entries in the next draw via update_from_turn_state when it sees
        // prev_phase=Streaming && settled=true. Clearing here would lose them.
        // They are cleared on the next on_turn_start / on_pre_turn.
        self.add_activity(ActivityKind::Completed, "Turn completed".to_string());
        // Don't clear streaming_text here either — same reason, cleared on next on_turn_start.
    }

    pub fn on_error(&mut self, error: &str) {
        self.phase = TurnPhase::Error(error.to_string());
        self.active_tools.clear();
        self.add_activity(ActivityKind::Error, format!("Error: {error}"));
    }

    pub fn on_interrupted(&mut self) {
        self.phase = TurnPhase::Idle;
        self.outcome = "interrupted".to_string();
        self.active_tools.clear();
        self.add_activity(ActivityKind::Info, "Turn interrupted".to_string());
    }

    pub fn on_cancel(&mut self, reason: &str) {
        self.on_interrupted();
        self.outcome = reason.to_string();
    }

    pub fn add_activity(&mut self, kind: ActivityKind, message: String) {
        self.activity.push(ActivityItem { kind, message, timestamp: Instant::now() });
        if self.activity.len() > 50 {
            self.activity.drain(0..self.activity.len() - 50);
        }
    }

    /// Get current streaming text (first 1000 chars for UI display).
    pub fn display_text(&self) -> String {
        if self.streaming_text.chars().count() > 1000 {
            self.streaming_text.chars().take(1000).collect::<String>() + "..."
        } else {
            self.streaming_text.clone()
        }
    }

    /// Get active tools (at most 2).
    pub fn active_tool_names(&self) -> Vec<String> {
        self.active_tools.iter().take(2).map(|t| t.name.clone()).collect()
    }

    /// Whether this turn has any active processing.
    pub fn is_active(&self) -> bool {
        !self.active_tools.is_empty()
            || matches!(self.phase, TurnPhase::Streaming | TurnPhase::ToolRunning)
    }

    /// Reset completely.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hook traits
// ─────────────────────────────────────────────────────────────────────────────

/// Hook base trait — all hooks carry id and priority metadata.
pub trait Hook: Send + Sync {
    fn id(&self) -> &str;
    fn priority(&self) -> u32 { 100 }
}

/// Agent loop lifecycle — fires once per run_loop invocation.
pub trait AgentLoopHooks: Hook {
    fn on_loop_start(&self) {}

    /// Called when the agent loop ends (after all turns complete).
    /// `outcome` describes why the loop ended: "completed", "interrupted", "budget_exhausted", etc.
    fn on_loop_end(&self, _outcome: &str) {}
}

/// Per-turn lifecycle — fires once per execute_turn call.
pub trait TurnLifecycleHooks: Hook {
    /// Called before the turn cycle begins (budget check, compression prep).
    fn on_pre_turn(&self) {}

    /// Called after the turn cycle ends (all tool calls resolved or final response).
    /// `response` is the final text output. `success` indicates if the turn completed without error.
    fn on_post_turn(&self, _response: &str, _success: bool) {}
}

/// Tool execution lifecycle.
pub trait ToolLifecycleHooks: Hook {
    /// Tool call generated by LLM (before execution).
    fn on_tool_gen(&self, _tool_name: &str, _call_id: &str) {}
    /// Tool execution begins.
    fn on_tool_start(&self, _tool_name: &str, _args: &str) {}
    /// Tool execution completes.
    fn on_tool_complete(&self, _tool_name: &str, _args: &str, _result: &str) {}
    /// Tool execution failed with an error.
    fn on_tool_error(&self, _tool_name: &str, _args: &str, _error: &str) {}
    /// Tool progress update (optional, for long-running tools).
    fn on_tool_progress(&self, _tool_name: &str, _preview: &str) {}
}

/// LLM output streaming events.
pub trait StreamingHooks: Hook {
    /// Main response text delta.
    fn on_stream_delta(&self, _text: &str) {}
    /// Thinking/reasoning text delta (from models that support it).
    fn on_thinking(&self, _text: &str) {}
    /// Reasoning text delta (separate from thinking).
    fn on_reasoning(&self, _text: &str) {}
    /// Interim assistant message (non-streaming, full text).
    fn on_interim_assistant(&self, _text: &str) {}
}

/// System status and diagnostic events.
pub trait SystemEventsHooks: Hook {
    /// Status message with level (e.g., "lifecycle", "warn", "info").
    fn on_status(&self, _level: &str, _message: &str) {}
}

/// Session lifecycle events.
pub trait SessionLifecycleHooks: Hook {
    /// Called when a session is rotated (compressed → parent ended, child created).
    /// `parent_id` is the old session, `child_id` is the new session.
    fn on_session_rotate(&self, _parent_id: &str, _child_id: &str) {}

    /// Called before context compression begins.
    fn on_compression_start(&self, _message_count: usize) {}

    /// Called after context compression completes.
    /// `status` describes the result: "compacted", "unchanged", "ineffective".
    fn on_compression_complete(&self, _status: &str) {}
}

/// Interrupt lifecycle events.
pub trait InterruptLifecycleHooks: Hook {
    /// Called when an interrupt is requested (user presses Ctrl+C).
    fn on_interrupt_requested(&self) {}

    /// Called when the turn actually responds to the interrupt.
    fn on_interrupted(&self, _reason: &str) {}
}
