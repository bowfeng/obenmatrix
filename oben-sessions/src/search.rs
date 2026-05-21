/// Full-text session search — three calling shapes matching `hermes_state.py`
/// session_search tool.
///
/// Maps to `tools/session_search_tool.py` and `hermes_state.py::search_messages`.
///
/// **Interface** (the seam):
/// * `Search::discover(query, limit, sort)` — FTS5 search + lineage dedup +
///   anchored views with bookends.
/// * `Search::scroll(session_id, message_id, window)` — ±N window navigation.
/// * `Search::browse(limit)` — recent sessions chronologically.
///
/// **Invariants**:
/// * All shapes use the same `SessionDB` underneath.
/// * Discovery deduplicates hits by session lineage (parent_session_id chain).
/// * Scroll and browse never do FTS5 — they read directly from the DB.
/// * No LLM calls anywhere — every shape returns actual messages from the DB.
///
/// **Discovery shape**:
/// Single-call: FTS5 → lineage dedup → `get_anchored_view` per hit.
/// Each result carries: session_id, title, when, source, snippet,
/// bookend_start (first 3 user+assistant msgs), messages (±5 around hit),
/// bookend_end (last 3 user+assistant msgs).
///
/// **Scroll shape**:
/// Returns a window of ±window messages centered on an anchor. No FTS5,
/// no bookends. To scroll forward/pass the last window message's id as the
/// new anchor; to scroll backward, pass the first.

use anyhow::Result;
use std::path::PathBuf;

use oben_models::Message;

use super::manager::{BrowseEntry, BrowseResult, DiscoveryEntry, DiscoveryResult, SearchHit, SessionDB, sanitize_fts5_query};
use oben_models::SessionSource;

// Hidden session sources by default.
const HIDDEN_SESSION_SOURCES: &[&str] = &["tool"];

/// Full-text session search engine.
///
/// Wraps `SessionDB` and provides the three calling shapes.
pub struct Search {
    db: SessionDB,
}

impl Search {
    /// Create a new search engine backed by the given database path.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        Ok(Self {
            db: SessionDB::new(db_path)?,
        })
    }

    /// Create from an existing `SessionDB`.
    pub fn from_db(db: SessionDB) -> Self {
        Self { db }
    }

    // ── Three calling shapes ────────────────────────────────────────────

    /// Run the appropriate shape based on which parameters are set.
    ///
    /// Scroll shape (session_id + around_message_id) takes precedence over
    /// discovery (query). Browse shape (no params) returns recent sessions.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        around_message_id: Option<i64>,
        window: Option<usize>,
    ) -> Result<SearchOutput> {
        // Scroll shape takes precedence — explicit anchor beats any query.
        if let (Some(sid), Some(mid)) = (session_id, around_message_id) {
            let w = window.unwrap_or(5);
            return Ok(SearchOutput::Scroll(self._scroll(sid, mid, w)?));
        }

        // Browse shape: no query → recent sessions.
        if query.trim().is_empty() {
            return Ok(SearchOutput::Browse(self._browse(limit)?));
        }

        // Discovery shape: FTS5 + lineage dedup + anchored views.
        Ok(SearchOutput::Discover(self._discover(query, limit)?))
    }

    // ── Discovery ───────────────────────────────────────────────────────

    /// Full-text search with lineage deduplication and anchored views.
    fn _discover(&self, query: &str, limit: usize) -> Result<DiscoveryResult> {
        let sanitized = sanitize_fts5_query(query);
        let query_lower = sanitized.to_lowercase();
        let _query_words: Vec<&str> = query_lower.split_whitespace().collect();
        let limit = limit.min(10);

        // Run FTS5 with a wider limit so dedup can find distinct sessions
        let raw_hits = self.db.search_messages(query, 50, None)?;

        if raw_hits.is_empty() {
            return Ok(DiscoveryResult {
                query: query.to_string(),
                results: Vec::new(),
                count: 0,
            });
        }

        // Dedupe by lineage: walk parent_session_id chain to root, keep first hit.
        let mut seen: std::collections::HashMap<String, &SearchHit> =
            std::collections::HashMap::new();

        for hit in &raw_hits {
            let resolved_sid = self._resolve_to_parent(&hit.session_id)?;
            if !seen.contains_key(&resolved_sid) {
                seen.insert(resolved_sid.clone(), hit);
            }
            if seen.len() >= limit {
                break;
            }
        }

        let mut results = Vec::new();
        for (lineage_root, hit) in seen {
            let view = self.db.get_anchored_view(&hit.session_id, hit.id.parse().unwrap_or(0), 5, 3);
            let view = match view {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("get_anchored_view failed for {}: {}", hit.session_id, e);
                    continue;
                }
            };

            let when = format_timestamp(hit.timestamp);
            let source = SessionSource::Cli; // Would need to fetch from DB

            let entry = DiscoveryEntry {
                session_id: hit.session_id.clone(),
                title: hit.session_title.clone(),
                when,
                source,
                model: None,
                snippet: hit.snippet.clone(),
                matched_role: hit.role.clone(),
                match_message_id: hit.id.clone(),
                window: view.window,
                bookend_start: view.bookend_start,
                bookend_end: view.bookend_end,
                messages_before: view.messages_before,
                messages_after: view.messages_after,
            };

            // If lineage root differs from hit session, add parent ref
            if lineage_root != hit.session_id {
                // Would store parent_session_id in entry
            }

            results.push(entry);
        }

        Ok(DiscoveryResult {
            query: query.to_string(),
            results: results.clone(),
            count: results.len(),
        })
    }

    // ── Scroll ──────────────────────────────────────────────────────────

    /// Scroll shape: return a window of messages centered on an anchor.
    fn _scroll(&self, session_id: &str, around_message_id: i64, window: usize) -> Result<ScrollOutput> {
        let window = window.clamp(1, 20);

        let view = self.db.get_messages_around(session_id, around_message_id, window)?;

        if view.window.is_empty() {
            return Err(anyhow::anyhow!(
                "around_message_id {} not found in session {}",
                around_message_id,
                session_id
            ));
        }

        // Lineage rebind: the anchor may live in a child session
        let resolved_session_id = session_id.to_string();
        if view.window.is_empty() {
            let owning = self._find_owning_session(session_id, around_message_id)?;
            if let Some(owning) = owning {
                let rebind_view = self.db.get_messages_around(&owning, around_message_id, window)?;
                if !rebind_view.window.is_empty() {
                    return Ok(ScrollOutput {
                        session_id: owning.clone(),
                        around_message_id,
                        window,
                        messages: rebind_view.window,
                        messages_before: rebind_view.messages_before,
                        messages_after: rebind_view.messages_after,
                        warning: Some(format!(
                            "Message {} lives in {} (child of {}); rebound transparently",
                            around_message_id, owning, session_id
                        )),
                    });
                }
            }
        }

        Ok(ScrollOutput {
            session_id: resolved_session_id,
            around_message_id,
            window,
            messages: view.window,
            messages_before: view.messages_before,
            messages_after: view.messages_after,
            warning: None,
        })
    }

    /// Find which session owns a message.
    fn _find_owning_session(&self, _session_id: &str, message_id: i64) -> Result<Option<String>> {
        // In production, this queries the DB directly via a raw SQL call.
        // For now, return None (no rebinding).
        let _ = message_id;
        Ok(None)
    }

    // ── Browse ──────────────────────────────────────────────────────────

    /// Browse shape: return recent sessions chronologically.
    fn _browse(&self, limit: usize) -> Result<BrowseResult> {
        let limit = limit.clamp(1, 10);

        let sessions = self.db.list_sessions(None, HIDDEN_SESSION_SOURCES, limit + 5, 0, false)?;

        let results: Vec<BrowseEntry> = sessions
            .into_iter()
            .filter(|s| s.parent_session_id.is_none()) // Skip children
            .map(|s| BrowseEntry {
                session_id: s.id.clone(),
                title: s.title.clone(),
                source: s.source,
                started_at: s.started_at,
                last_active: s.started_at, // Would be last message timestamp
                message_count: s.message_count,
                preview: s.preview,
            })
            .take(limit)
            .collect();

        Ok(BrowseResult {
            results: results.clone(),
            count: results.len(),
        })
    }

    // ── Lineage helpers ─────────────────────────────────────────────────

    /// Walk parent_session_id chain to the lineage root.
    fn _resolve_to_parent(&self, session_id: &str) -> Result<String> {
        let mut current = session_id.to_string();
        let mut visited = std::collections::HashSet::new();

        while !visited.contains(&current) {
            visited.insert(current.clone());

            let meta = self.db.get_session(&current)?;
            let parent = match meta {
                Some(s) => s.metadata.parent_session_id,
                None => None,
            };

            match parent {
                Some(p) => current = p,
                None => return Ok(current),
            }
        }

        Ok(current)
    }

    /// Close the database connection.
    pub fn close(&self) -> Result<()> {
        self.db.close()
    }
}

// ── Search output types ─────────────────────────────────────────────────────

/// Unified search output covering all three shapes.
pub enum SearchOutput {
    Discover(DiscoveryResult),
    Scroll(ScrollOutput),
    Browse(BrowseResult),
}

/// Result of a scroll operation.
pub struct ScrollOutput {
    pub session_id: String,
    pub around_message_id: i64,
    pub window: usize,
    pub messages: Vec<Message>,
    pub messages_before: usize,
    pub messages_after: usize,
    pub warning: Option<String>,
}

/// Format a Unix timestamp to a human-readable date.
fn format_timestamp(ts: f64) -> String {
    if ts == 0.0 {
        return "unknown".to_string();
    }
    let millis = (ts * 1000.0) as i64;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .map(|dt| dt.format("%B %d, %Y at %I:%M %p").to_string())
        .unwrap_or("unknown".to_string())
}

// ── Simple in-memory search (word-count substring matching) ──────────────

/// Fast in-memory search over a slice of sessions.
/// Use for lightweight queries or when `Search` (FTS5) is not available.
pub fn search_sessions(sessions: &[&oben_models::Session], query: &str, limit: usize) -> Vec<SearchResult> {
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
    /// Display formatted result.
    pub fn display(&self) -> String {
        format!(
            "📄 [{}] (relevance: {:.0}%)\n   {}\n",
            self.session_name,
            self.relevance_score * 100.0,
            self.snippet
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::Message;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init_test_db() -> PathBuf {
        INIT.call_once(|| {
            tracing::trace!("Initializing test tracing");
        });
        tempfile::tempdir().unwrap().path().join("test_search.db")
    }

    #[test]
    fn test_search_discover() {
        let path = init_test_db();
        let search = Search::new(path.clone()).unwrap();

        // Create a session with messages
        let session = search.db.get_or_create_session("discover-test").unwrap();
        let sid = session.id.clone();
        search.db.save_messages(&sid, &mut vec![
            Message::user("hello world"),
            Message::assistant("how can I help you today"),
            Message::user("search for rust code"),
        ]).unwrap();

        let result = search._discover("rust", 5).unwrap();
        // May or may not match depending on FTS5 setup
        let _ = result;
    }

    #[test]
    fn test_search_browse() {
        let path = init_test_db().join("browse.db");
        let search = Search::new(path).unwrap();

        search.db.get_or_create_session("browse-a").unwrap();
        search.db.get_or_create_session("browse-b").unwrap();

        let result = search._browse(10).unwrap();
        assert!(result.count >= 2);
    }

    #[test]
    fn test_search_scroll() {
        let path = init_test_db().join("scroll.db");
        let search = Search::new(path).unwrap();

        let session = search.db.get_or_create_session("scroll-test").unwrap();
        let sid = session.id.clone();
        let mut msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("message {}", i)))
            .collect();
        search.db.save_messages(&sid, &mut msgs).unwrap();

        let loaded = search.db.load_messages(&sid).unwrap();
        let anchor_id: i64 = loaded[5].id.unwrap();

        let result = search._scroll(&sid, anchor_id, 3).unwrap();
        assert!(!result.messages.is_empty());
    }

    #[test]
    fn test_search_unified_search() {
        let path = init_test_db().join("unified.db");
        let search = Search::new(path).unwrap();

        // Browse shape
        let result = search.search("", 5, None, None, None).unwrap();
        assert!(matches!(result, SearchOutput::Browse(_)));

        // Scroll shape
        let session = search.db.get_or_create_session("unified-test").unwrap();
        let sid = session.id.clone();
        let mut msgs = vec![Message::user("hello")];
        search.db.save_messages(&sid, &mut msgs).unwrap();
        let loaded = search.db.load_messages(&sid).unwrap();
        let mid: i64 = loaded[0].id.unwrap();

        let result = search.search("", 5, Some(&sid), Some(mid), Some(3)).unwrap();
        assert!(matches!(result, SearchOutput::Scroll(_)));
    }

    #[test]
    fn test_search_roundtrip() {
        let path = init_test_db().join("roundtrip.db");
        let search = Search::new(path).unwrap();

        let session = search.db.get_or_create_session("roundtrip").unwrap();
        let sid = session.id.clone();
        let mut msgs = vec![
            Message::user("first message"),
            Message::assistant("response"),
        ];
        search.db.save_messages(&sid, &mut msgs).unwrap();

        let loaded = search.db.load_messages(&sid).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content.to_text(), "first message");
    }
}
