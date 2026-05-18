/// Context compression — summarization to manage long conversations.

use anyhow::Result;
use tracing::warn;

use crate::context::ContextManager;

pub struct ContextCompressor {
    strategy: CompressionStrategy,
}

#[derive(Debug, Clone)]
pub enum CompressionStrategy {
    /// Summarize old messages using LLM (requires transport).
    Summary,
    /// Drop oldest messages beyond the cutoff.
    TokenCount,
    /// Don't compress at all.
    None,
}

impl ContextCompressor {
    pub fn new() -> Self {
        Self {
            strategy: CompressionStrategy::Summary,
        }
    }

    pub fn with_strategy(&mut self, strategy: CompressionStrategy) {
        self.strategy = strategy;
    }

    /// Compress context — produce a summary that replaces old messages.
    pub fn summarize_context(&self, context: &ContextManager) -> Result<String> {
        match &self.strategy {
            CompressionStrategy::Summary => {
                // In a full implementation, this would call the LLM to summarize.
                // For now, return a structural summary.
                let msg_count = context.len();
                let token_count = context.estimate_tokens();
                Ok(format!(
                    "[CONTEXT SUMMARY: Conversation has {msg_count} messages, ~{token_count} estimated tokens.]\n\
                     The conversation is ongoing. Refer to the full message history for details."
                ))
            }
            CompressionStrategy::TokenCount => {
                // Drop oldest messages, keep only the most recent.
                Ok("[CONTEXT SUMMARY: Oldest messages have been truncated to fit context window.]\n\
                   Refer to recent messages for current context.".to_string())
            }
            CompressionStrategy::None => {
                warn!("Compression requested but strategy is None");
                Ok(String::new())
            }
        }
    }
}
