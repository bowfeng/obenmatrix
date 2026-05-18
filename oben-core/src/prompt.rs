/// Prompt building — assembles system prompt + conversation history for the API.

use anyhow::Result;
use oben_models::Message;

use crate::context::ContextManager;

pub struct PromptBuilder {
    system_prompt: String,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            system_prompt: oben_config::defaults::default_system_prompt(),
        }
    }

    /// Set a custom system prompt.
    pub fn set_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = prompt.into();
    }

    /// Build the messages array for the LLM API.
    pub fn build_api_messages(&self, context: &ContextManager) -> Result<Vec<Message>> {
        let mut messages = vec![Message::system(self.system_prompt.clone())];
        messages.extend(context.messages().iter().cloned());
        Ok(messages)
    }

    /// Get current system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
}
