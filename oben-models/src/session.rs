use serde::{Deserialize, Serialize};

/// A summary chunk in summary.jsonl — indicates which raw messages it covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryChunk {
    /// primary id of the first message covered
    pub from: i64,
    /// primary id of the last message covered
    pub to: i64,
    /// The summary text covering messages [from, to].
    pub summary: String,
}

/// Source tag for sessions (cli, telegram, discord, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum SessionSource {
    #[default]
    Cli,
    Gateway,
    Telegram,
    Discord,
    Slack,
    Web,
    Tool,
    Cron,
    Batch,
    #[serde(other)]
    Other,
}

/// Session metadata stored in the SQLite sessions table.
/// Mirrors `hermes_state.py` SCHEMA_SQL sessions table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMetadata {
    pub id: String,
    pub name: String,
    pub source: SessionSource,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub parent_session_id: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub end_reason: Option<String>, // "compression", "branched", "cancelled", etc.
    pub title: Option<String>,
    pub preview: Option<String>,    // first 60 chars of first user message
    pub message_count: usize,
    pub tool_call_count: usize,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub handoff_state: Option<String>,
    pub handoff_platform: Option<String>,
    pub handoff_error: Option<String>,
}

/// A conversation session (in-memory view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub messages: Vec<crate::Message>,
    /// Compressed memory / context snapshot for long sessions.
    pub memory_context: Option<String>,
    /// Summary chunks indicating which raw message lines are covered.
    #[serde(default)]
    pub summary_chunks: Vec<SummaryChunk>,
    #[serde(default)]
    pub persisted_message_count: usize,
    /// SQLite metadata (populated when using SQLite-backed store).
    #[serde(default)]
    pub metadata: SessionMetadata,
}

impl Session {
    pub fn new(name: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            memory_context: None,
            summary_chunks: Vec::new(),
            persisted_message_count: 0,
            metadata: SessionMetadata::default(),
        }
    }

    pub fn add_message(&mut self, msg: crate::Message) {
        self.messages.push(msg);
        self.updated_at = chrono::Utc::now();
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn compress_memory(&mut self, context: impl Into<String>) {
        self.memory_context = Some(context.into());
    }
}
