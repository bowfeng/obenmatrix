use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, types::Value, OptionalExtension};
use tracing::info;

use oben_models::{Message, MessageRole, Session, SessionMetadata, SessionSource, SummaryChunk};

fn now_ts() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

fn ts_to_datetime(ts: f64) -> chrono::DateTime<chrono::Utc> {
    let millis = (ts * 1000.0) as i64;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis).unwrap_or(chrono::Utc::now())
}

fn parse_source(s: &str) -> SessionSource {
    match s {
        "cli" => SessionSource::Cli,
        "gateway" => SessionSource::Gateway,
        "telegram" => SessionSource::Telegram,
        "discord" => SessionSource::Discord,
        "slack" => SessionSource::Slack,
        "web" => SessionSource::Web,
        "tool" => SessionSource::Tool,
        "cron" => SessionSource::Cron,
        "batch" => SessionSource::Batch,
        _ => SessionSource::Cli,
    }
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL DEFAULT 'cli',
    model TEXT,
    system_prompt TEXT,
    parent_session_id TEXT,
    started_at REAL NOT NULL,
    ended_at REAL,
    end_reason TEXT,
    title TEXT,
    preview TEXT,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    handoff_state TEXT,
    handoff_platform TEXT,
    handoff_error TEXT,
    FOREIGN KEY (parent_session_id) REFERENCES sessions(id)
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL,
    content TEXT,
    tool_call_id TEXT,
    tool_calls TEXT,
    tool_name TEXT,
    timestamp REAL NOT NULL,
    token_count INTEGER,
    finish_reason TEXT,
    reasoning TEXT,
    reasoning_content TEXT,
    reasoning_details TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
";

const FTS_SQL: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content);
";

fn reconcile_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)")?;
    conn.execute("INSERT OR IGNORE INTO schema_version (version) VALUES (11)", [])?;
    Ok(())
}

fn row_to_message(row: &rusqlite::Row) -> std::result::Result<Message, rusqlite::Error> {
    let role_str: String = row.get("role")?;
    let role = match role_str.as_str() {
        "system" => MessageRole::System,
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    };
    let content: String = row.get("content").unwrap_or_default();
    let tool_calls: Option<String> = row.get("tool_calls").ok();
    let tool_calls = tool_calls.and_then(|tc| serde_json::from_str(&tc).ok());
    let tool_call_id: Option<String> = row.get("tool_call_id").ok();
    let id: Option<i64> = row.get("id").ok();
    Ok(Message {
        role,
        content: oben_models::MessageContent::Text(content),
        id,
        tool_call_ids: tool_call_id.into_iter().collect(),
        tool_calls,
    })
}

pub struct SessionDB {
    #[allow(dead_code)]
    db_path: std::path::PathBuf,
    conn: std::sync::Mutex<Option<Connection>>,
}

impl SessionDB {
    pub fn new<P: AsRef<std::path::Path>>(db_path: P) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        let db_dir = db_path.parent().unwrap_or(db_path.as_ref());
        std::fs::create_dir_all(db_dir)?;
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_size_limit=1000000; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute_batch(FTS_SQL)?;
        reconcile_schema(&conn)?;
        info!("Opened session DB at {}", db_path.display());
        Ok(Self { db_path, conn: std::sync::Mutex::new(Some(conn)) })
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T>
    where F: FnOnce(&Connection) -> Result<T>,
    {
        let lock = self.conn.lock().unwrap();
        match lock.as_ref() {
            Some(c) => f(c),
            None => Err(anyhow!("database connection is closed")),
        }
    }

    fn with_conn_mut<F, T>(&self, f: F) -> Result<T>
    where F: FnOnce(&mut Connection) -> Result<T>,
    {
        let mut lock = self.conn.lock().unwrap();
        match lock.as_mut() {
            Some(c) => f(c),
            None => Err(anyhow!("database connection is closed")),
        }
    }

    pub fn get_or_create_session(&self, name: &str) -> Result<Session> {
        let session = self.with_conn(|conn| {
            let id: Option<String> = conn.query_row(
                "SELECT id FROM sessions WHERE COALESCE(title, '') = ? LIMIT 1",
                [name],
                |row| row.get(0),
            ).ok();
            match id {
                Some(id) => self.session_from_id(conn, &id),
                None => self.create_session_record(conn, name),
            }
        })?;
        Ok(session)
    }

    fn create_session_record(&self, conn: &Connection, name: &str) -> Result<Session> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let ts = now_ts();
        conn.execute(
            "INSERT INTO sessions (id, title, started_at, source, message_count) VALUES (?, ?, ?, 'cli', 0)",
            params![id.clone(), name, ts],
        )?;
        let metadata = SessionMetadata {
            id: id.clone(), name: name.to_string(), source: SessionSource::Cli,
            title: Some(name.to_string()), started_at: now, message_count: 0,
            ..Default::default()
        };
        Ok(Session {
            id: id.clone(), name: name.to_string(), created_at: now, updated_at: now,
            messages: Vec::new(), memory_context: None, summary_chunks: Vec::new(),
            persisted_message_count: 0, metadata,
        })
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, COALESCE(title, ''), source, model, system_prompt, parent_session_id, started_at, ended_at, end_reason, title, preview, handoff_state, message_count, tool_call_count, input_tokens, output_tokens, handoff_platform, handoff_error FROM sessions WHERE id = ?"
            )?;
            match stmt.query_row([session_id], |row| {
                let source_str: String = row.get("source")?;
                let started_at_f64: f64 = row.get("started_at")?;
                let title: String = row.get("title")?;
                let ended_at: Option<f64> = row.get("ended_at")?;
                Ok(SessionMetadata {
                    id: row.get("id")?, name: title.clone(), source: parse_source(&source_str),
                    model: row.get("model")?, system_prompt: row.get("system_prompt")?,
                    parent_session_id: row.get("parent_session_id")?,
                    started_at: ts_to_datetime(started_at_f64),
                    ended_at: ended_at.map(|v| ts_to_datetime(v)),
                    end_reason: row.get("end_reason")?, title: Some(title),
                    preview: row.get("preview")?,
                    message_count: row.get("message_count")?,
                    tool_call_count: row.get("tool_call_count")?,
                    input_tokens: row.get("input_tokens")?,
                    output_tokens: row.get("output_tokens")?,
                    handoff_state: row.get("handoff_state")?,
                    handoff_platform: row.get("handoff_platform")?,
                    handoff_error: row.get("handoff_error")?,
                })
            }) {
                Ok(metadata) => Ok(Some(Session {
                    id: metadata.id.clone(), name: metadata.name.clone(),
                    created_at: metadata.started_at, updated_at: metadata.started_at,
                    messages: Vec::new(), memory_context: None, summary_chunks: Vec::new(),
                    persisted_message_count: 0, metadata,
                })),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow!(e)),
            }
        })
    }

    fn session_from_id(&self, conn: &Connection, session_id: &str) -> Result<Session> {
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(title, ''), source, model, system_prompt, parent_session_id, \
             started_at, ended_at, end_reason, title, preview, handoff_state, \
             message_count, tool_call_count, input_tokens, output_tokens, handoff_platform, handoff_error \
             FROM sessions WHERE id = ?"
        )?;
        let row: (
            String, String, String, Option<String>, Option<String>, Option<String>,
            f64, Option<f64>, Option<String>, Option<String>, Option<String>,
            Option<String>, usize, usize, usize, usize, Option<String>, Option<String>,
        ) = match stmt.query_row([session_id], |row| {
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
                row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?,
                row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                row.get(15)?, row.get(16)?, row.get(17)?,
            ))
        }) {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(anyhow!("session not found: {}", session_id)),
            Err(e) => return Err(anyhow!(e)),
        };

        let source = parse_source(&row.2);
        let title_str = row.1.clone();
        let started_at = ts_to_datetime(row.6);
        let ended_at = row.7.map(ts_to_datetime);

        let metadata = SessionMetadata {
            id: row.0, name: title_str.clone(), source, model: row.3, system_prompt: row.4,
            parent_session_id: row.5, started_at, ended_at,
            end_reason: row.8, title: Some(title_str.clone()), preview: row.10,
            handoff_state: row.11, handoff_platform: row.16, handoff_error: row.17,
            message_count: row.12, tool_call_count: row.13,
            input_tokens: row.14, output_tokens: row.15,
        };

        Ok(Session {
            id: metadata.id.clone(), name: metadata.name.clone(),
            created_at: started_at, updated_at: started_at,
            messages: Vec::new(), memory_context: None,
            summary_chunks: Vec::new(), persisted_message_count: 0,
            metadata,
        })
    }

    pub fn save_messages(&self, session_id: &str, messages: &[Message]) -> Result<()> {
        self.with_conn_mut(|conn| {
            conn.execute("UPDATE sessions SET message_count = ?, ended_at = ? WHERE id = ?",
                params![messages.len(), now_ts(), session_id])?;
            conn.execute("DELETE FROM messages WHERE session_id = ?", params![session_id])?;
            for msg in messages {
                let role = match msg.role {
                    MessageRole::System => "system", MessageRole::User => "user",
                    MessageRole::Assistant => "assistant", MessageRole::Tool => "tool",
                };
                let content = msg.content.to_text();
                let tool_calls = msg.tool_calls.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default());
                let tool_call_id = if msg.tool_call_ids.len() > 0 {
                    Some(msg.tool_call_ids.join(","))
                } else {
                    None
                };
                // Ignore return — rowid will be returned on next load via row_to_message.
                conn.execute(
                    "INSERT INTO messages (session_id, role, content, tool_calls, tool_call_id, timestamp, tool_name) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![session_id, role, content, tool_calls, tool_call_id, now_ts(), msg.tool_calls.as_ref().map(|_| "unknown")],
                )?;
            }
            let _ = conn.execute("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')", []);
            Ok(())
        })
    }

    pub fn save_new_messages(&self, session_id: &str, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        self.with_conn_mut(|conn| {
            // Bulk insert with UPSERT on the autoincrement id (use session_id+timestamp+role as unique guard).
            // We insert without specifying id so SQLite auto-allocates, and skip any that already exist.
            let role_str = |role: &MessageRole| -> &str {
                match role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                }
            };

            // Insert each message; sqlite's ignore ensures we don't insert duplicates
            // if called while the DB was partially updated.
            for msg in messages {
                let role = role_str(&msg.role);
                let content = msg.content.to_text();
                let tool_calls = msg.tool_calls.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default());
                let tool_call_id = if msg.tool_call_ids.len() > 0 {
                    Some(msg.tool_call_ids.join(","))
                } else {
                    None
                };
                let _id = conn.execute(
                    "INSERT OR IGNORE INTO messages (session_id, role, content, tool_calls, tool_call_id, timestamp, tool_name) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![session_id, role, content, tool_calls, tool_call_id, now_ts(), msg.tool_calls.as_ref().map(|_| "unknown")],
                )?;
                // Only assign id if this was a new insert (not an IGNORE)
                // We can't reliably know from the return value, so skip
                // — the next load will pick up the DB-generated id.
            }

            // Update the session metadata count
            let new_count = messages.len();
            conn.execute(
                "UPDATE sessions SET message_count = ? + ?, ended_at = ? WHERE id = ?",
                params![new_count, 0_i32, now_ts(), session_id],
            )?;

            // Rebuild FTS index to include new messages
            let _ = conn.execute("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')", []);
            Ok(())
        })
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM messages WHERE session_id = ? ORDER BY id")?;
            let mut rows = stmt.query_map(params![session_id], row_to_message)?;
            let mut messages = Vec::new();
            while let Some(row_result) = rows.next() {
                messages.push(row_result?);
            }
            Ok(messages)
        })
    }

    pub fn get_messages_around(&self, session_id: &str, around_message_id: i64, window: usize) -> Result<MessagesAround> {
        if window == 0 {
            return Ok(MessagesAround { window: Vec::new(), messages_before: 0, messages_after: 0 });
        }
        self.with_conn(|conn| {
            let anchor_exists = conn.query_row(
                "SELECT 1 FROM messages WHERE id = ? AND session_id = ? LIMIT 1",
                params![around_message_id, session_id],
                |row| row.get::<_, i32>(0),
            ).optional()?.map(|v| v == 1).unwrap_or(false);

            if !anchor_exists {
                return Ok(MessagesAround { window: Vec::new(), messages_before: 0, messages_after: 0 });
            }

            let mut stmt = conn.prepare(
                "SELECT * FROM messages WHERE session_id = ? AND id <= ? ORDER BY id DESC LIMIT ?"
            )?;
            let before: Vec<Message> = stmt.query_map(params![session_id, around_message_id, window + 1], row_to_message)?
                .filter_map(|r| r.ok()).collect::<Vec<_>>();

            let mut stmt = conn.prepare(
                "SELECT * FROM messages WHERE session_id = ? AND id > ? ORDER BY id ASC LIMIT ?"
            )?;
            let after: Vec<Message> = stmt.query_map(params![session_id, around_message_id, window], row_to_message)?
                .filter_map(|r| r.ok()).collect::<Vec<_>>();

            let after_len = after.len();
            let mut all = before.into_iter().rev().chain(after.into_iter()).collect::<Vec<_>>();
            if !all.iter().any(|m| m.id == Some(around_message_id)) {
                if let Ok(msg) = conn.query_row(
                    "SELECT * FROM messages WHERE id = ? AND session_id = ?",
                    params![around_message_id, session_id], row_to_message)
                {
                    all.push(msg);
                    all.sort_by(|a, b| {
                        let aid = a.id.unwrap_or(0);
                        let bid = b.id.unwrap_or(0);
                        aid.cmp(&bid)
                    });
                }
            }
            let window_len = all.len();
            Ok(MessagesAround {
                window: all,
                messages_before: window_len.saturating_sub(after_len + 1),
                messages_after: after_len,
            })
        })
    }

    pub fn get_anchored_view(&self, session_id: &str, around_message_id: i64, window: usize, bookend: usize) -> Result<AnchoredView> {
        let primitive = self.get_messages_around(session_id, around_message_id, window)?;
        if primitive.window.is_empty() {
            return Ok(AnchoredView { window: Vec::new(), messages_before: 0, messages_after: 0, bookend_start: Vec::new(), bookend_end: Vec::new() });
        }
        let filtered: Vec<Message> = primitive.window.into_iter()
            .filter(|m| m.id == Some(around_message_id) || matches!(m.role, MessageRole::User | MessageRole::Assistant))
            .collect();
        let first_id = filtered.first().and_then(|m| m.id).unwrap_or(-1);
        let last_id = filtered.last().and_then(|m| m.id).unwrap_or(-1);

        let bookend_start = if first_id > 0 {
            self._load_bookend(session_id, first_id, bookend, true)?
        } else {
            Vec::new()
        };
        let bookend_end = if last_id > 0 {
            self._load_bookend(session_id, last_id, bookend, false)?
        } else {
            Vec::new()
        };

        Ok(AnchoredView {
            window: filtered,
            messages_before: primitive.messages_before,
            messages_after: primitive.messages_after,
            bookend_start, bookend_end,
        })
    }

    fn _load_bookend(&self, session_id: &str, id: i64, bookend: usize, forward: bool) -> Result<Vec<Message>> {
        if bookend == 0 { return Ok(Vec::new()); }
        let clause = if forward { "id < ?" } else { "id > ?" };
        let order = if forward { "ORDER BY id ASC" } else { "ORDER BY id DESC" };
        let sql = format!("SELECT * FROM messages WHERE session_id = ? AND {} AND role IN ('user', 'assistant') AND length(content) > 0 {} LIMIT ?", clause, order);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query_map(params![session_id, id, bookend], row_to_message)?;
            let mut result = Vec::new();
            while let Some(row_result) = rows.next() {
                result.push(row_result?);
            }
            if !forward { result.reverse(); }
            Ok(result)
        })
    }

    pub fn list_sessions(&self, source: Option<&str>, exclude_sources: &[&str], limit: usize, offset: usize, include_children: bool) -> Result<Vec<SessionMetadata>> {
        self.with_conn(|conn| {
            let mut conditions = Vec::new();
            let mut bind: Vec<Value> = Vec::new();
            if !include_children { conditions.push("(s.parent_session_id IS NULL)".to_string()); }
            if let Some(src) = source { conditions.push("s.source = ?".to_string()); bind.push(Value::from(src.to_string())); }
            if !exclude_sources.is_empty() {
                let placeholders: Vec<String> = (0..exclude_sources.len()).map(|_| "?".to_string()).collect();
                conditions.push(format!("s.source NOT IN ({})", placeholders.join(",")));
                for src in exclude_sources { bind.push(Value::from((*src).to_string())); }
            }
            let where_clause = if conditions.is_empty() { String::new() } else { format!("WHERE {}", conditions.join(" AND ")) };
            let query = format!(
                "SELECT id, COALESCE(title, '') AS title, source, model, COALESCE(system_prompt, '') AS system_prompt, parent_session_id, started_at, ended_at, end_reason, title AS display_title, preview, handoff_state, message_count, tool_call_count, input_tokens, output_tokens, handoff_platform, handoff_error \
                 FROM sessions s {} ORDER BY started_at DESC LIMIT ? OFFSET ?",
                where_clause
            );
            bind.push(Value::from(limit as i64));
            bind.push(Value::from(offset as i64));
            let mut stmt = conn.prepare(&query)?;
            let mut rows = stmt.query_map(rusqlite::params_from_iter(bind.iter()), |row| {
                let source_str: String = row.get("source")?;
                let started_at_f64: f64 = row.get("started_at")?;
                let started_at = ts_to_datetime(started_at_f64);
                let title_str: String = row.get("title")?;
                Ok(SessionMetadata {
                    id: row.get("id")?, name: title_str.clone(), source: parse_source(&source_str),
                    model: row.get("model")?, system_prompt: row.get("system_prompt")?,
                    parent_session_id: row.get("parent_session_id")?,
                    started_at, ended_at: None, end_reason: row.get("end_reason")?,
                    title: Some(title_str), preview: row.get("preview")?,
                    message_count: row.get("message_count")?,
                    tool_call_count: row.get("tool_call_count")?,
                    input_tokens: row.get("input_tokens")?,
                    output_tokens: row.get("output_tokens")?,
                    handoff_state: row.get("handoff_state")?,
                    handoff_platform: row.get("handoff_platform")?,
                    handoff_error: row.get("handoff_error")?,
                })
            })?;
            let mut result = Vec::new();
            while let Some(row_result) = rows.next() {
                result.push(row_result?);
            }
            Ok(result)
        })
    }

    pub fn search_messages(&self, query: &str, limit: usize, _role_filter: Option<&[&str]>) -> Result<Vec<SearchHit>> {
        self.with_conn(|conn| {
            let sanitized = query.replace('%', "\\%").replace('_', "\\_");
            let like_query = format!("%{}%", sanitized);
            let query_str = format!(
                "SELECT m.id, m.session_id, m.role, m.content, m.timestamp, COALESCE(s.title, ''), s.started_at, '' \
                 FROM messages m JOIN sessions s ON m.session_id = s.id \
                 WHERE m.content LIKE ? AND s.parent_session_id IS NULL \
                 ORDER BY m.timestamp DESC LIMIT {}", limit
            );
            let mut stmt = conn.prepare(&query_str)?;
            let mut rows = stmt.query_map(params![Value::from(like_query)], |row| {
                let id_int: i64 = row.get(0)?;
                Ok(SearchHit {
                    id: id_int.to_string(), session_id: row.get(1)?,
                    session_title: row.get(5)?, role: row.get(2)?,
                    content: row.get(3)?, timestamp: row.get(4)?,
                    snippet: String::new(),
                })
            })?;
            let mut result = Vec::new();
            while let Some(row_result) = rows.next() {
                result.push(row_result?);
            }
            Ok(result)
        })
    }

    pub fn resolve_session_tip(&self, session_id: &str) -> Result<String> {
        self.with_conn(|conn| {
            let mut current = session_id.to_string();
            for _ in 0..100 {
                let has_messages = conn.query_row(
                    "SELECT 1 FROM messages WHERE session_id = ? LIMIT 1",
                    params![current],
                    |row| row.get::<_, i32>(0),
                ).optional()?.map(|v| v == 1).unwrap_or(false);
                if has_messages { return Ok(current); }
                match conn.query_row(
                    "SELECT id FROM sessions WHERE parent_session_id = ? ORDER BY started_at DESC LIMIT 1",
                    params![current], |row| row.get(0))
                {
                    Ok(cid) => current = cid,
                    Err(_) => return Ok(current),
                }
            }
            Ok(current)
        })
    }

    pub fn resolve_resume_session(&self, session_id: &str) -> Result<String> {
        self.with_conn(|conn| {
            let mut current = session_id.to_string();
            let mut seen = vec![current.clone()];
            for _ in 0..100 {
                let has_messages = conn.query_row(
                    "SELECT 1 FROM messages WHERE session_id = ? LIMIT 1",
                    params![current],
                    |row| row.get::<_, i32>(0),
                ).optional()?.map(|v| v == 1).unwrap_or(false);
                if has_messages { return Ok(current); }
                match conn.query_row(
                    "SELECT id FROM sessions WHERE parent_session_id = ? ORDER BY started_at DESC, id DESC LIMIT 1",
                    params![current], |row| row.get(0))
                {
                    Ok(cid) if !seen.contains(&cid) => { seen.push(cid.clone()); current = cid; }
                    _ => return Ok(current),
                }
            }
            Ok(current)
        })
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.with_conn_mut(|conn| {
            conn.execute("DELETE FROM messages WHERE session_id = ?", params![session_id])?;
            conn.execute("DELETE FROM sessions WHERE id = ?", params![session_id])?;
            Ok(())
        })
    }

    /// Mark a session as ended with a reason.
    pub fn end_session(&self, session_id: &str, end_reason: &str) -> Result<()> {
        self.with_conn_mut(|conn| {
            conn.execute(
                "UPDATE sessions SET ended_at = ?, end_reason = ? WHERE id = ?",
                params![now_ts(), end_reason, session_id],
            )?;
            Ok(())
        })
    }

    /// Set the parent_session_id for a session.
    pub fn set_parent_session_id(&self, session_id: &str, parent_id: &str) -> Result<()> {
        self.with_conn_mut(|conn| {
            conn.execute(
                "UPDATE sessions SET parent_session_id = ? WHERE id = ?",
                params![parent_id, session_id],
            )?;
            Ok(())
        })
    }

    pub fn close(&self) -> Result<()> {
        let mut lock = self.conn.lock().unwrap();
        if let Some(conn) = lock.take() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
            drop(conn);
        }
        Ok(())
    }
}

pub struct MessagesAround {
    pub window: Vec<Message>,
    pub messages_before: usize,
    pub messages_after: usize,
}

pub struct AnchoredView {
    pub window: Vec<Message>,
    pub messages_before: usize,
    pub messages_after: usize,
    pub bookend_start: Vec<Message>,
    pub bookend_end: Vec<Message>,
}

pub struct SearchHit {
    pub id: String,
    pub session_id: String,
    pub session_title: Option<String>,
    pub role: String,
    pub content: String,
    pub timestamp: f64,
    pub snippet: String,
}

pub struct DiscoveryResult {
    pub query: String,
    pub results: Vec<DiscoveryEntry>,
    pub count: usize,
}

#[derive(Clone)]
pub struct DiscoveryEntry {
    pub session_id: String,
    pub title: Option<String>,
    pub when: String,
    pub source: SessionSource,
    pub model: Option<String>,
    pub snippet: String,
    pub matched_role: String,
    pub match_message_id: String,
    pub window: Vec<Message>,
    pub bookend_start: Vec<Message>,
    pub bookend_end: Vec<Message>,
    pub messages_before: usize,
    pub messages_after: usize,
}

pub struct BrowseResult {
    pub results: Vec<BrowseEntry>,
    pub count: usize,
}

#[derive(Clone)]
pub struct BrowseEntry {
    pub session_id: String,
    pub title: Option<String>,
    pub source: SessionSource,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_active: chrono::DateTime<chrono::Utc>,
    pub message_count: usize,
    pub preview: Option<String>,
}

pub fn sanitize_fts5_query(query: &str) -> String {
    query.replace('"', "\\\"").replace('(', "\\(").replace(')', "\\)")
}

// ── SessionManager (SQLite-backed, wraps SessionDB) ────────────────────────

/// In-memory session cache with SQLite persistence via SessionDB.
pub struct SessionManager {
    db: SessionDB,
    sessions: std::collections::HashMap<String, Session>,
    active_session_id: Option<String>,
}

impl SessionManager {
    pub fn new() -> Result<Self> {
        let db_dir = dirs::home_dir()
            .map(|d| d.join(".obenagent").join("memory"))
            .unwrap_or_else(|| std::path::PathBuf::from("~/.obenagent"));
        let db_path = db_dir.join("state.db");
        Self::new_with_db_path(db_path)
    }

    pub fn new_with_path(storage_path: std::path::PathBuf) -> Result<Self> {
        let db_path = storage_path.join("state.db");
        Self::new_with_db_path(db_path)
    }

    fn new_with_db_path(db_path: std::path::PathBuf) -> Result<Self> {
        let db = SessionDB::new(&db_path)?;
        Ok(Self {
            db,
            sessions: std::collections::HashMap::new(),
            active_session_id: None,
        })
    }

    fn find_session_key_by_name(&self, name: &str) -> Option<String> {
        self.sessions.iter().find(|(_, s)| s.name == name).map(|(k, _)| k.clone())
    }

    fn load_session_into_cache(&mut self, session_id: &str) -> Result<()> {
        // Get metadata from DB
        let meta = self.db.get_session(session_id)?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        // Load messages from DB
        let messages = self.db.load_messages(session_id)?;
        let msg_count = messages.len();
        let mut session = meta;
        session.messages = messages;
        session.persisted_message_count = msg_count;
        self.sessions.insert(session_id.to_string(), session);
        if self.active_session_id.is_none() {
            self.active_session_id = Some(session_id.to_string());
        }
        Ok(())
    }

    pub fn init(&mut self) -> Result<()> {
        // List all sessions from DB and load into cache
        let session_ids: Vec<String> = self.db.list_sessions(None, &[], 1000, 0, false)?
            .iter().map(|s| s.id.clone()).collect();
        for sid in session_ids {
            if let Ok(session) = self.db.get_session(&sid) {
                if let Some(mut s) = session {
                    let msgs = self.db.load_messages(&sid)?;
                    s.persisted_message_count = msgs.len();
                    s.messages = msgs;
                    self.sessions.insert(sid.clone(), s);
                }
            }
        }
        if let Some(last) = self.sessions.values().max_by_key(|s| s.updated_at) {
            self.active_session_id = Some(last.id.clone());
        }
        Ok(())
    }

    pub fn get_or_create_session(&mut self, name: &str) -> &mut Session {
        let key = self.find_session_key_by_name(name);
        match key {
            Some(key) => self.sessions.get_mut(&key).unwrap(),
            None => {
                let session = self.db.get_or_create_session(name).unwrap();
                let id = session.id.clone();
                let messages = self.db.load_messages(&id).unwrap_or_default();
                let mut full_session = session;
                full_session.messages = messages;
                self.sessions.insert(id.clone(), full_session);
                self.active_session_id = Some(id.clone());
                self.sessions.get_mut(&id).unwrap()
            }
        }
    }

    pub fn new_session(&mut self, name: &str) -> &mut Session {
        let session = self.db.get_or_create_session(name).unwrap();
        let id = session.id.clone();
        let messages = self.db.load_messages(&id).unwrap_or_default();
        let mut full_session = session;
        full_session.messages = messages;
        self.sessions.insert(id.clone(), full_session);
        self.active_session_id = Some(id);
        self.sessions.get_mut(self.active_session_id.as_ref().unwrap()).unwrap()
    }

    pub fn create(&mut self, name: &str) -> &mut Session {
        self.new_session(name)
    }

    pub fn switch_session(&mut self, session_id: &str) -> Result<&mut Session> {
        let current_active = self.active_session_id.clone();
        if let Some(ref active_id) = current_active {
            if active_id != session_id {
                self.save(Some(active_id))?;
            }
        }
        self.load(Some(session_id))?;
        self.active_session_id = Some(session_id.to_string());
        self.sessions.get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))
    }

    pub fn switch(&mut self, session_id: &str) -> Result<SwitchResult> {
        let session_id = self.find_key(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        let session = self.switch_session(&session_id)?;
        Ok(SwitchResult {
            session_id: session_id.clone(),
            name: session.name.clone(),
            message_count: session.message_count(),
            was_new: false,
        })
    }

    pub fn switch_or_create(&mut self, key: &str) -> Result<SwitchResult> {
        if let Ok(sr) = self.switch(key) {
            return Ok(sr);
        }
        let session = self.create(key);
        Ok(SwitchResult {
            session_id: session.id.clone(),
            name: session.name.clone(),
            message_count: session.message_count(),
            was_new: true,
        })
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.active_session_id.as_ref().and_then(|id| self.sessions.get(id))
    }

    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_session_id.as_ref().and_then(|id| self.sessions.get_mut(id))
    }

    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn list(&self) -> Vec<&Session> {
        self.list_sessions()
    }

    pub fn save(&mut self, session_id: Option<&str>) -> Result<()> {
        let sid: String = match session_id {
            Some(id) => id.to_string(),
            None => match &self.active_session_id {
                Some(id) => id.clone(),
                None => { info!("No active session to save"); return Ok(()); }
            },
        };
        let session = self.sessions.get(&sid)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", sid))?;
        let new_count = session.messages.len() - session.persisted_message_count;

        // No new messages to persist
        if new_count == 0 {
            return Ok(());
        }

        let start = session.messages.len() - new_count;
        let new_messages: Vec<Message> = session.messages[start..].iter().cloned().collect();
        drop(session);

        self.db.save_new_messages(&sid, &new_messages)?;
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.persisted_message_count = s.messages.len();
        }
        Ok(())
    }

    pub fn load(&mut self, session_id: Option<&str>) -> Result<()> {
        match session_id {
            Some(id) => self.load_session_into_cache(id),
            None => {
                // Load all sessions from DB
                let ids: Vec<String> = self.db.list_sessions(None, &[], 1000, 0, false)?
                    .iter().map(|s| s.id.clone()).collect();
                for sid in ids {
                    if let Ok(session) = self.db.get_session(&sid) {
                        if let Some(mut s) = session {
                            let msgs = self.db.load_messages(&sid)?;
                            s.persisted_message_count = msgs.len();
                            s.messages = msgs;
                            self.sessions.insert(sid.clone(), s);
                        }
                    }
                }
                if let Some(last) = self.sessions.values().max_by_key(|s| s.updated_at) {
                    self.active_session_id = Some(last.id.clone());
                }
                Ok(())
            }
        }
    }

    pub fn find_key(&self, key: &str) -> Option<String> {
        if self.sessions.contains_key(key) {
            return Some(key.to_string());
        }
        self.find_session_key_by_name(key)
    }

    pub fn remove_session(&mut self, key: &str) -> Result<()> {
        // Resolve name → UUID (like switch() does)
        let resolved = self.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", key))?;

        self.db.delete_session(&resolved)?;
        self.sessions.remove(&resolved);
        if self.active_session_id.as_deref() == Some(&resolved) {
            self.active_session_id = None;
        }
        Ok(())
    }

    pub fn save_session_to_file(&mut self, session: &Session) -> Result<()> {
        // SQLite-backed — just write to DB
        self.db.save_messages(&session.id, &session.messages)?;
        if let Some(s) = self.sessions.get_mut(&session.id) {
            s.persisted_message_count = session.messages.len();
        }
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }


    #[inline]
    pub fn active(&self) -> Option<&Session> {
        self.active_session()
    }

    #[inline]
    pub fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }

    #[inline]
    pub fn save_session(&mut self, session_id: &str) -> Result<()> {
        self.save(Some(session_id))
    }

    #[inline]
    pub fn delete(&mut self, session_id: &str) -> Result<()> {
        self.remove_session(session_id)
    }

    /// Split a session after compression: end the parent, create a child.
    ///
    /// After context compression, the old session is marked as ended with
    /// `end_reason = "compression"` and a new child session is created with
    /// `parent_session_id` pointing to the old session.
    ///
    /// The child's title is auto-numbered: "chat-123" → "chat-123 (2)".
    pub fn split_after_compression(&mut self, parent_id: &str) -> Result<Session> {
        // 1. Mark parent as ended with compression reason
        self.db.end_session(parent_id, "compression")?;

        // 2. Save parent's messages to DB before creating child
        let parent = self.sessions.get(parent_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", parent_id))?;
        let parent_messages = parent.messages.clone();
        self.db.save_messages(parent_id, &parent_messages)?;

        // 3. Determine child title: extract base name and append "(N)"
        let base_title = parent.metadata.title.as_deref().unwrap_or(&parent.name);
        let child_title = self.next_child_title(base_title, &parent_id);

        // 4. Create child session record in DB
        let child_session = self.db.get_or_create_session(&child_title)?;
        let child_id = child_session.id.clone();

        // 5. Set parent_session_id in DB
        self.db.set_parent_session_id(&child_id, parent_id)?;

        // 6. Load child messages (empty) and messages into cache
        let child_messages = self.db.load_messages(&child_id)?;
        let mut full_child = child_session;
        full_child.messages = child_messages;
        self.sessions.insert(child_id.clone(), full_child.clone());

        // 7. Set child as active session
        self.active_session_id = Some(child_id.clone());

        Ok(full_child)
    }

    /// Compute the next auto-numbered title for a child session.
    ///
    /// Finds the highest existing child number and increments it.
    /// Examples: "chat" → "chat (2)", "chat (5)" → "chat (6)".
    fn next_child_title(&self, base: &str, parent_id: &str) -> String {
        let next_num = self.find_next_child_number(base, parent_id);
        format!("{} ({})", base, next_num)
    }

    /// Find the next child number for a given base title.
    fn find_next_child_number(&self, base: &str, parent_id: &str) -> usize {
        // Scan in-memory sessions for existing children
        let mut max_num = 1;
        for s in self.sessions.values() {
            if s.metadata.parent_session_id.as_deref() == Some(parent_id) {
                if let Some(title) = &s.metadata.title {
                    // Parse "base (N)" suffix
                    if let Some(suffix) = title.strip_prefix(base) {
                        if let Some(num) = suffix.strip_prefix(" (").and_then(|s| s.strip_suffix(')')) {
                            if let Ok(n) = num.parse::<usize>() {
                                if n > max_num { max_num = n; }
                            }
                        }
                    }
                }
            }
        }
        max_num + 1
    }

    /// Clone a session: create a new session with the same messages.
    pub fn clone_session(&mut self, name: &str) -> Option<Session> {
        let target = self.find_key(name)?;
        let src = self.sessions.get(&target)?.clone();
        let mut new_session = Session::new(name);
        new_session.messages = src.messages.clone();
        new_session.summary_chunks = src.summary_chunks.clone();
        let new_id = new_session.id.clone();
        self.sessions.insert(new_id.clone(), new_session.clone());
        self.active_session_id = Some(new_id.clone());
        Some(self.sessions.get(&new_id).unwrap().clone())
    }
}

pub struct SwitchResult {
    pub session_id: String,
    pub name: String,
    pub message_count: usize,
    pub was_new: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::Message;

    fn make_test_dir() -> std::path::PathBuf {
        tempfile::tempdir().unwrap().path().join("sessions")
    }

    #[test] fn test_manager_creates_session() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path).unwrap();
        let session = mgr.new_session("test-session");
        assert_eq!(session.name, "test-session");
        assert!(!session.id.is_empty());
        assert_eq!(mgr.session_count(), 1);
    }

    #[test] fn test_manager_list_sessions() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path).unwrap();
        mgr.new_session("s1"); mgr.new_session("s2");
        assert_eq!(mgr.list_sessions().len(), 2);
    }

    #[test] fn test_manager_get_or_create_reuses_existing() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path).unwrap();
        let s1 = mgr.get_or_create_session("my-session");
        s1.add_message(Message::user("first"));
        let s2 = mgr.get_or_create_session("my-session");
        assert_eq!(s2.name, "my-session");
        assert_eq!(s2.message_count(), 1);
    }

    #[test] fn test_save_and_load_roundtrip() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path.clone()).unwrap();
        let session = mgr.new_session("persist-test");
        session.add_message(Message::user("hello"));
        session.add_message(Message::assistant("hi there"));
        let count = mgr.session_count();
        mgr.save(None).unwrap();
        let mut mgr2 = SessionManager::new_with_path(path.clone()).unwrap();
        mgr2.load(None).unwrap();
        assert_eq!(mgr2.session_count(), count);
        let loaded = mgr2.list_sessions().into_iter().next().unwrap();
        assert_eq!(loaded.name, "persist-test");
        assert_eq!(loaded.message_count(), 2);
    }

    #[test] fn test_switch_session() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path).unwrap();
        let s1 = mgr.new_session("s1"); s1.add_message(Message::user("msg in s1"));
        let s1_id = s1.id.clone();
        mgr.save(None).unwrap(); // persist s1 to DB
        let s2 = mgr.new_session("s2"); s2.add_message(Message::user("msg in s2"));
        assert!(mgr.active_session().unwrap().name == "s2");
        let switched = mgr.switch_session(&s1_id).unwrap();
        assert_eq!(switched.name, "s1");
        assert_eq!(switched.message_count(), 1);
    }

    #[test] fn test_switch_to_nonexistent_session_fails() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path).unwrap();
        let err = mgr.switch_session("nonexistent-id").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test] fn test_load_empty_directory() {
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path.clone()).unwrap();
        mgr.save(None).unwrap();
        let mut mgr2 = SessionManager::new_with_path(path.clone()).unwrap();
        mgr2.load(None).unwrap();
        assert_eq!(mgr2.session_count(), 0);
    }

    #[test] fn test_save_load_messages_persisted() {
        // SQLite persists all messages to DB — verified by round-trip load
        let path = make_test_dir();
        let mut mgr = SessionManager::new_with_path(path.clone()).unwrap();
        let session = mgr.new_session("persist-test");
        session.add_message(Message::user("msg1"));
        session.add_message(Message::assistant("msg2"));
        session.add_message(Message::user("msg3"));
        session.add_message(Message::assistant("msg4"));
        mgr.save(None).unwrap();
        let mut mgr2 = SessionManager::new_with_path(path.clone()).unwrap();
        mgr2.load(None).unwrap();
        let loaded = mgr2.list_sessions().into_iter().next().unwrap();
        assert_eq!(loaded.message_count(), 4);
    }

    #[test] fn test_manager_delete() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        mgr.create("to-delete");
        assert_eq!(mgr.list().len(), 1);
        let id = mgr.list()[0].id.clone();
        mgr.remove_session(&id).unwrap();
        assert_eq!(mgr.list().len(), 0);
    }

    #[test] fn test_manager_delete_by_name() {
        // Regression: deletion was broken when called with session name
        // (not UUID), because remove_session passed the name directly
        // to db.delete_session() and HashMap::remove() which both
        // expect the UUID primary key.
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        mgr.create("delete-by-name");
        assert_eq!(mgr.list().len(), 1);
        mgr.remove_session("delete-by-name").unwrap();
        assert_eq!(mgr.list().len(), 0);
    }

    fn make_db_dir() -> std::path::PathBuf {
        tempfile::tempdir().unwrap().path().join("db")
    }

    #[test] fn test_db_create_and_get_session() {
        let path = make_db_dir().join("state.db");
        let db = SessionDB::new(&path).unwrap();
        let session = db.get_or_create_session("test-session").unwrap();
        assert_eq!(session.name, "test-session");
        assert!(!session.id.is_empty());
        let session2 = db.get_or_create_session("test-session").unwrap();
        assert_eq!(session.id, session2.id);
    }

    #[test] fn test_db_save_and_load_messages() {
        let path = make_db_dir().join("state2.db");
        let db = SessionDB::new(&path).unwrap();
        let session = db.get_or_create_session("msg-test").unwrap();
        let session_id = session.id.clone();
        db.save_messages(&session_id, &[
            Message::user("hello"), Message::assistant("hi there"), Message::user("how are you"),
        ]).unwrap();
        let loaded = db.load_messages(&session_id).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].content.to_text(), "hello");
        assert_eq!(loaded[1].content.to_text(), "hi there");
    }

    #[test] fn test_db_messages_around() {
        let path = make_db_dir().join("around.db");
        let db = SessionDB::new(&path).unwrap();
        let session = db.get_or_create_session("around-test").unwrap();
        let sid = session.id.clone();
        let msgs: Vec<Message> = (0..10).map(|i| Message::user(format!("message {}", i))).collect();
        db.save_messages(&sid, &msgs).unwrap();
        let loaded = db.load_messages(&sid).unwrap();
        let anchor_id: i64 = loaded[5].id.unwrap();
        let result = db.get_messages_around(&sid, anchor_id, 2).unwrap();
        assert!(result.window.len() >= 3);
    }

    #[test] fn test_db_delete_session() {
        let path = make_db_dir().join("delete.db");
        let db = SessionDB::new(&path).unwrap();
        let sid = db.get_or_create_session("del-me").unwrap().id;
        let sid2 = db.get_or_create_session("del-me").unwrap().id;
        assert_eq!(sid, sid2, "same session should be returned");
        db.delete_session(&sid).unwrap();
        let sid3 = db.get_or_create_session("del-me").unwrap().id;
        assert_ne!(sid, sid3, "should create new session after delete");
    }

    #[test] fn test_db_list_sessions() {
        let path = make_db_dir().join("list.db");
        let db = SessionDB::new(&path).unwrap();
        db.get_or_create_session("session-a").unwrap();
        db.get_or_create_session("session-b").unwrap();
        let sessions = db.list_sessions(None, &[], 10, 0, false).unwrap();
        assert!(sessions.len() >= 2);
    }

    // ── split_after_compression tests ─────────────────────────────────────

    #[test]
    fn test_split_after_compression_creates_child_session() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let parent_id = {
            let parent = mgr.new_session("test-chat");
            parent.add_message(Message::user("hello"));
            parent.add_message(Message::assistant("hi there"));
            parent.id.clone()
        };
        mgr.save(None).unwrap();

        let child = mgr.split_after_compression(&parent_id).unwrap();

        // New session was created
        assert!(!child.id.is_empty());
        assert_ne!(child.id, parent_id, "child must have different id");

        // Parent is marked ended
        let parent_db = mgr.db.get_session(&parent_id).unwrap().unwrap();
        assert_eq!(parent_db.metadata.end_reason, Some("compression".to_string()), "parent end_reason should be 'compression'");
        assert!(parent_db.metadata.ended_at.is_some(), "parent ended_at should be set");
    }

    #[test]
    fn test_split_after_compression_sets_parent_session_id() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let parent_id = {
            let parent = mgr.new_session("test-chat-2");
            parent.add_message(Message::user("hello"));
            parent.id.clone()
        };
        mgr.save(None).unwrap();

        let child = mgr.split_after_compression(&parent_id).unwrap();

        // Child references parent
        let child_db = mgr.db.get_session(&child.id).unwrap().unwrap();
        assert_eq!(child_db.metadata.parent_session_id, Some(parent_id), "child parent_session_id should match parent");
    }

    #[test]
    fn test_split_after_compression_auto_numbers_title() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let parent_id = {
            let parent = mgr.new_session("chat-12345");
            parent.id.clone()
        };
        mgr.save(None).unwrap();

        let child = mgr.split_after_compression(&parent_id).unwrap();

        // Title should be "chat-12345 (2)"
        let child_db = mgr.db.get_session(&child.id).unwrap().unwrap();
        assert_eq!(child_db.metadata.title.as_deref(), Some("chat-12345 (2)"), "title should be auto-numbered");
    }

    #[test]
    fn test_split_after_compression_returns_child() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let parent_id = {
            let parent = mgr.new_session("split-test");
            parent.add_message(Message::user("msg1"));
            parent.add_message(Message::assistant("msg2"));
            parent.id.clone()
        };
        mgr.save(None).unwrap();

        let child = mgr.split_after_compression(&parent_id).unwrap();

        // Returned session is the child
        assert_eq!(child.id, child.id);
        assert_eq!(child.name, "split-test (2)".to_string(), "title should be auto-numbered");
    }
}
