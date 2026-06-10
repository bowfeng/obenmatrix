//! Turn events — messages sent from turn controller to UI components.

use std::time::Instant;

/// Activity item types for status feed
#[derive(Debug, Clone)]
pub enum ActivityKind {
    Info,
    ToolStart,
    ToolComplete,
    Streaming,
    Error,
    Completed,
}

#[derive(Debug, Clone)]
pub struct ActivityItem {
    pub kind: ActivityKind,
    pub message: String,
    pub timestamp: Instant,
}

/// Active (in-flight) tool call
#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
    pub started_at: Instant,
    pub context: String,
}

/// Completed tool trail line
#[derive(Debug, Clone)]
pub struct CompletedTool {
    pub name: String,
    pub output_preview: String,
    pub has_error: bool,
}

/// Current turn state
#[derive(Debug, Default)]
pub struct TurnState {
    /// Overall state: idle, streaming, tool_active, or completed
    pub phase: TurnPhase,
    /// Live streaming text accumulated during turn
    pub streaming_text: String,
    /// In-flight tools (at most 1-3)
    pub active_tools: Vec<ActiveTool>,
    /// Completed tool trail (max 8, newest first)
    pub completed_tools: Vec<CompletedTool>,
    /// Reasoning/thinking text
    pub reasoning_text: String,
    /// Activity/status messages feed
    pub activity: Vec<ActivityItem>,
    /// Overall turn outcome
    pub outcome: String,
    /// Whether turn was interrupted mid-stream
    pub interrupted: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum TurnPhase {
    #[default]
    Idle,
    Streaming,
    ToolRunning,
    Completed,
    Error(String),
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }

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
        // Remove from active and add to completed
        let has_error =
            result.to_lowercase().contains("error") || result.to_lowercase().contains("failed");
        let preview = if result.len() > 60 {
            format!("{}...", &result[..60])
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
        // Keep at most 8 completed tools
        if self.completed_tools.len() > 8 {
            self.completed_tools.truncate(8);
        }
        // Remove from active
        self.active_tools.retain(|t| t.id != tool_id);
        let status = if has_error { "error" } else { "✅" };
        self.add_activity(
            if has_error {
                ActivityKind::Error
            } else {
                ActivityKind::ToolComplete
            },
            format!("{status} {tool_name}"),
        );
    }

    pub fn on_stream_delta(&mut self, text: &str) {
        tracing::info!(
            "[TurnState::on_stream_delta] text.len={} text='{}' total_after={} phase={:?}",
            text.len(),
            text,
            self.streaming_text.len() + text.len(),
            self.phase
        );
        self.streaming_text.push_str(text);
        self.add_activity(
            ActivityKind::Streaming,
            format!("Streaming: {}...", text.chars().take(30).collect::<String>()),
        );
    }

    pub fn on_reasoning(&mut self, text: &str) {
        self.reasoning_text.push_str(text);
        if self.reasoning_text.len() > 2000 {
            self.reasoning_text =
                self.reasoning_text[self.reasoning_text.len() - 2000..].to_string();
        }
    }

    pub fn on_completed(&mut self, outcome: &str) {
        self.phase = TurnPhase::Completed;
        self.outcome = outcome.to_string();
        self.active_tools.clear();
        self.streaming_text.clear();
        self.add_activity(ActivityKind::Completed, "Turn completed".to_string());
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
        self.activity.push(ActivityItem {
            kind,
            message,
            timestamp: Instant::now(),
        });
        // Keep last 50 activity items
        if self.activity.len() > 50 {
            self.activity.drain(0..self.activity.len() - 50);
        }
    }

    /// Get current streaming text (first 1000 chars for UI display)
    pub fn display_text(&self) -> String {
        if self.streaming_text.len() > 1000 {
            format!("{}...", &self.streaming_text[..1000])
        } else {
            self.streaming_text.clone()
        }
    }

    /// Get active tools (at most 2)
    pub fn active_tool_names(&self) -> Vec<String> {
        self.active_tools
            .iter()
            .take(2)
            .map(|t| t.name.clone())
            .collect()
    }

    /// Whether this turn has any active processing
    pub fn is_active(&self) -> bool {
        !self.active_tools.is_empty()
            || matches!(self.phase, TurnPhase::Streaming | TurnPhase::ToolRunning)
    }

    /// Reset completely
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}
