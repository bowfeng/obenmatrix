use serde::{Deserialize, Serialize};

/// A summary chunk in summary.jsonl — indicates which raw messages it covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryChunk {
    /// Line number (1-indexed) in raw.jsonl where this summary starts.
    pub from: usize,
    /// Line number (1-indexed, exclusive) where this summary ends.
    pub to: usize,
    /// The summary text covering messages [from, to).
    pub summary: String,
}

/// A conversation session.
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
    /// Total number of messages in raw.jsonl (for appends).
    #[serde(default)]
    pub persisted_message_count: usize,
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
