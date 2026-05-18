use serde::{Deserialize, Serialize};

/// A conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<crate::Message>,
    /// Compressed memory / context snapshot for long sessions.
    pub memory_context: Option<String>,
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
