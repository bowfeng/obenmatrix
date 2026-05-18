/// Full-text session search.
/// Maps to `tools/session_search_tool.py` and the FTS5 search in Hermes.

use anyhow::Result;
use oben_models::Session;
use regex::Regex;
use tracing::debug;

/// Search across sessions for relevant content.
pub fn search_sessions(sessions: &[&Session], query: &str, limit: usize) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut results: Vec<SearchResult> = sessions
        .iter()
        .flat_map(|s| {
            s.messages.iter().enumerate().filter_map(|(idx, msg)| {
                let text = match &msg.content {
                    oben_models::MessageContent::Text(t) => t,
                    _ => return None,
                };
                let text_lower = text.to_lowercase();
                let score = query_words.iter().filter(|w| text_lower.contains(**w)).count();
                if score > 0 {
                    Some(SearchResult {
                        session_name: s.name.clone(),
                        message_index: idx,
                        snippet: extract_snippet(text, query, 100),
                        relevance_score: score as f32 / query_words.len() as f32,
                    })
                } else {
                    None
                }
            })
        })
        .collect();

    results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

/// Extract a snippet around the query match.
fn extract_snippet(text: &str, query: &str, max_len: usize) -> String {
    if let Some(pos) = text.to_lowercase().find(&query.to_lowercase()) {
        let start = pos.saturating_sub(max_len / 2);
        let end = (pos + query.len() + max_len / 2).min(text.len());
        let snippet = &text[start..end];
        let prefix = if start > 0 { "..." } else { "" };
        let suffix = if end < text.len() { "..." } else { "" };
        format!("{}{}{}", prefix, snippet, suffix)
    } else {
        text.chars().take(max_len).collect()
    }
}

/// A search result from session memory.
pub struct SearchResult {
    pub session_name: String,
    pub message_index: usize,
    pub snippet: String,
    pub relevance_score: f32,
}

impl SearchResult {
    pub fn display(&self) -> String {
        format!(
            "📄 [{}] (relevance: {:.0}%)\n   {}\n",
            self.session_name,
            self.relevance_score * 100.0,
            self.snippet
        )
    }
}
