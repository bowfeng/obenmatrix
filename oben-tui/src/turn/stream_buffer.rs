//! Stream buffer — manages accumulated streaming text from agent tokens.

use std::sync::{Arc, Mutex};

/// Shared buffer for streaming text during turn
#[derive(Debug, Default)]
pub struct StreamBuffer {
    text: Arc<Mutex<String>>,
}

impl StreamBuffer {
    pub fn new() -> Self {
        Self {
            text: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Push delta text into buffer
    pub fn push(&self, text: &str) {
        if let Ok(mut buf) = self.text.lock() {
            if buf.len() < 2000 {
                buf.push_str(text);
            }
        }
    }

    /// Get current text
    pub fn get(&self) -> String {
        self.text.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Clear buffer
    pub fn clear(&self) {
        if let Ok(mut buf) = self.text.lock() {
            buf.clear();
        }
    }

    /// Clone for sharing across tasks
    pub fn clone_buffer(&self) -> StreamBuffer {
        StreamBuffer {
            text: Arc::clone(&self.text),
        }
    }
}
