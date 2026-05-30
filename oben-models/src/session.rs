use serde::{Deserialize, Serialize};


/// A single message in a conversation.

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

/// Detailed origin information for a session, used for dynamic context injection.
/// Mirrors Hermes `SessionSource` with platform, channel, and user metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionOrigin {
    /// Platform the session originated from (cli, telegram, discord, etc.)
    pub platform: SessionSource,
    /// Chat/channel ID (unique per conversation)
    pub chat_id: Option<String>,
    /// Human-readable chat/channel name
    pub chat_name: Option<String>,
    /// Chat type: "dm", "group", "channel", "thread"
    pub chat_type: String,
    /// User who initiated the session
    pub user_id: Option<String>,
    /// User display name
    pub user_name: Option<String>,
    /// Thread/topic ID for threaded conversations
    pub thread_id: Option<String>,
    /// Guild/workspace ID (Discord, Slack)
    pub guild_id: Option<String>,
}

impl SessionOrigin {
    /// Create a new CLI session origin.
    pub fn cli() -> Self {
        Self {
            platform: SessionSource::Cli,
            chat_type: "dm".to_string(),
            ..Default::default()
        }
    }

    /// Human-readable description for logging/display.
    pub fn description(&self) -> String {
        match self.platform {
            SessionSource::Cli => "CLI terminal".to_string(),
            _ => format!(
                "{} ({}), chat_type={}",
                format_args!("{:?}", self.platform),
                self.chat_name.as_deref().or(self.chat_id.as_deref()).unwrap_or("unknown"),
                self.chat_type
            ),
        }
    }
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
    pub end_reason: Option<String>, // "compression", "branched", "cancelled", "reset"
    pub title: Option<String>,
    pub preview: Option<String>,    // first 60 chars of first user message
    pub message_count: usize,
    pub tool_call_count: usize,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub total_tokens: usize,
    pub estimated_cost_usd: f64,
    pub cost_status: String, // "unknown", "tracked", "estimated"
    pub handoff_state: Option<String>,
    pub handoff_platform: Option<String>,
    pub handoff_error: Option<String>,
    /// Session origin metadata for routing and context injection.
    pub origin: Option<SessionOrigin>,
    /// Token tracking: last API-reported prompt tokens (for accurate compression check).
    pub last_prompt_tokens: usize,
    /// Session lifecycle flags.
    pub is_fresh_reset: bool,     // user explicitly sent /new or /reset
    pub suspended: bool,          // next access will auto-reset (stuck loop breaker)
    pub resume_pending: bool,     // interrupted by restart, recovery expected
    pub resume_reason: Option<String>,
    pub last_resume_marked_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// In-memory access to a session's message buffer.
///
/// Both the real `SessionManager` and in-memory test doubles implement this
/// trait, letting `TurnExecutor` and `ConversationLoop` work with any session
/// store without requiring SQLite.
pub trait SessionStore {
    /// Get mutable access to a session's messages by ID.
    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session>;

    /// Get read-only access to a session by ID.
    fn session(&self, session_id: &str) -> Option<&Session>;
}

/// Full session lifecycle management trait.
///
/// Extended by `SessionManager` to provide complete session management:
/// creation, reset, suspension, resumption, listing, deletion, and search.
///
/// Methods that create/switch sessions return `&mut Session` so the caller
/// can mutate messages and metadata in one shot — no extra lookup needed.
pub trait SessionManagerExt: Send + Sync {
    /// Initialize the session store from disk.
    fn init(&mut self) -> Result<(), anyhow::Error>;

    /// Get or create a session, loading its messages into the cache.
    /// Returns `&mut Session` for in-place modification (messages, metadata).
    fn get_or_create_session(&mut self, name: &str) -> &mut Session;

    /// Create a new empty session.
    /// Returns `&mut Session` for in-place modification.
    fn create_session(&mut self, name: &str) -> &mut Session;

    /// Switch to an existing session by key or ID.
    /// Returns `&mut Session` on success.
    fn switch_session(&mut self, key: &str) -> Result<&mut Session, anyhow::Error>;

    /// Reset the current session (start fresh, keeping metadata).
    fn reset_current_session(&mut self) -> Result<(), anyhow::Error>;

    /// Reset a specific session by key or ID.
    fn reset_session(&mut self, key: &str) -> Result<(), anyhow::Error>;

    /// Suspend a session — next access will auto-create a fresh session.
    fn suspend_session(&mut self, key: &str) -> bool;

    /// Mark a session as resumable after a restart interruption.
    fn mark_resume_pending(&mut self, key: &str, reason: &str) -> bool;

    /// Clear the resume-pending flag after a successful resumed turn.
    fn clear_resume_pending(&mut self, key: &str) -> bool;

    /// List all sessions, optionally filtered by activity window.
    fn list_sessions(&self, active_minutes: Option<u64>) -> Vec<SessionListEntry>;

    /// Delete a session by key or ID.
    fn delete_session(&mut self, key: &str) -> Result<(), anyhow::Error>;

    /// Prune sessions older than `max_age_days` (exclude suspended and active).
    /// Returns the number of entries removed.
    fn prune_sessions(&mut self, max_age_days: i64) -> usize;

    /// Save pending messages to the session store.
    fn save_session(&mut self, session_id: Option<&str>) -> Result<(), anyhow::Error>;

    /// Get the active session ID.
    fn active_session_id(&self) -> Option<String>;

    /// Update token tracking for a session.
    fn update_token_tracking(
        &mut self,
        session_id: &str,
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
        estimated_cost_usd: f64,
    );

    /// Split session after compression: end parent, create child with lineage.
    ///
    /// After context compression, the parent session is marked ended with
    /// `end_reason = "compression"` and a new child session is created with
    /// `parent_session_id` pointing to the parent. The child's title is
    /// auto-numbered: "chat-123" → "chat-123 (2)".
    ///
    /// Returns the child session (which is now the active session).
    fn split_after_compression(&mut self, parent_id: &str) -> Result<Session, anyhow::Error>;

    /// Get a mutable reference to the currently active session.
    fn active_session_mut(&mut self) -> Option<&mut Session>;

    /// Get a mutable reference to a session by ID.
    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session>;

    /// Get a read reference to a session by ID.
    fn session(&self, session_id: &str) -> Option<&Session>;
}

/// Entry in the session list view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListEntry {
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub source: SessionSource,
    pub model: Option<String>,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_active: chrono::DateTime<chrono::Utc>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub end_reason: Option<String>,
    pub preview: Option<String>,
    pub parent_session_id: Option<String>,
    pub suspended: bool,
}

/// Session recap — summarize recent activity without LLM calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecap {
    /// Session identifier.
    pub session_id: String,
    /// Session title (if available).
    pub session_title: Option<String>,
    /// Human-readable summary of recent activity.
    pub summary: String,
    /// Turn counts: (user_turns, assistant_turns, tool_messages).
    pub turn_counts: (usize, usize, usize),
    /// Tool activity: sorted list of (tool_name, count).
    pub tool_activity: Vec<(String, usize)>,
    /// Recently touched files (most recent first).
    pub recent_files: Vec<String>,
    /// Last user prompt preview.
    pub last_user_prompt: Option<String>,
    /// Last assistant text preview.
    pub last_assistant_text: Option<String>,
}

/// Build a session recap from conversation history.
///
/// Inspired by Claude Code's `/recap` command. Pure local computation —
/// no LLM calls, no auxiliary model, instant and free.
///
/// Returns a human-readable summary of recent activity including:
/// - Turn counts and scope
/// - Tool activity (most used first)
/// - Recently touched files
/// - Last user prompt and assistant reply previews
pub fn build_session_recap(
    messages: &[crate::Message],
    session_title: Option<&str>,
    session_id: Option<&str>,
) -> SessionRecap {
    const RECENT_TURN_WINDOW: usize = 20;
    const PROMPT_PREVIEW_CHARS: usize = 140;
    const ASSISTANT_PREVIEW_CHARS: usize = 200;
    const MAX_FILES_LISTED: usize = 5;

    let user_turns = messages.iter().filter(|m| m.role == crate::MessageRole::User).count();
    let assistant_turns = messages.iter().filter(|m| m.role == crate::MessageRole::Assistant).count();
    let tool_messages = messages.iter().filter(|m| m.role == crate::MessageRole::Tool).count();

    // Recent window: last N user+assistant turns
    let recent_window = recent_turn_window(messages, RECENT_TURN_WINDOW);
    let rec_user_turns = recent_window.iter().filter(|m| m.role == crate::MessageRole::User).count();
    let rec_assistant_turns = recent_window.iter().filter(|m| m.role == crate::MessageRole::Assistant).count();

    // Tool activity from recent window
    let mut tool_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut file_paths: Vec<String> = Vec::new();
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();

    for msg in recent_window.iter().rev() {
        if msg.role == crate::MessageRole::Assistant {
            if let Some(tcs) = &msg.tool_calls {
                for tc in tcs {
                    let name = &tc.tool_name;
                    *tool_counts.entry(name.clone()).or_insert(0) += 1;

                    // Track file-editing tools
                    if let Some(path) = extract_file_path(&tc.arguments, name) {
                        if seen_files.insert(path.clone()) {
                            file_paths.push(shorten_path(&path));
                        }
                    }
                }
            }
        }
    }

    let mut tool_activity: Vec<(String, usize)> = tool_counts.into_iter().collect();
    tool_activity.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let file_paths: Vec<String> = file_paths.into_iter().take(MAX_FILES_LISTED).collect();

    // Last user prompt and assistant text
    let last_user_prompt = latest_text(messages, crate::MessageRole::User).map(|t| {
        let trimmed = collapse_whitespace(&t);
        if trimmed.len() > PROMPT_PREVIEW_CHARS {
            format!("{}...", &trimmed[..PROMPT_PREVIEW_CHARS - 1])
        } else {
            trimmed
        }
    });

    let last_assistant_text = latest_text(messages, crate::MessageRole::Assistant).map(|t| {
        let trimmed = collapse_whitespace(&t);
        if trimmed.len() > ASSISTANT_PREVIEW_CHARS {
            format!("{}...", &trimmed[..ASSISTANT_PREVIEW_CHARS - 1])
        } else {
            trimmed
        }
    });

    // Build summary line
    let mut parts = Vec::new();
    if rec_user_turns > 0 || rec_assistant_turns > 0 {
        parts.push(format!(
            "{} user turn{}, {} assistant reply{}, {} tool results",
            rec_user_turns,
            if rec_user_turns != 1 { "s" } else { "" },
            rec_assistant_turns,
            if rec_assistant_turns != 1 { "ies" } else { "y" },
            tool_messages
        ));
    }
    if !tool_activity.is_empty() {
        let top = tool_activity.iter()
            .take(5)
            .map(|(name, count)| format!("{}×{}", name, count))
            .collect::<Vec<_>>()
            .join(", ");
        let extra = tool_activity.len().saturating_sub(5);
        let top = if extra > 0 {
            format!("{} (+{} more)", top, extra)
        } else {
            top
        };
        parts.push(format!("Tools used: {}", top));
    }
    if !file_paths.is_empty() {
        let files = file_paths.join(", ");
        parts.push(format!("Files touched: {}", files));
    }
    let summary = if parts.is_empty() {
        "(no activity yet in this window)".to_string()
    } else {
        parts.join(" | ")
    };

    SessionRecap {
        session_id: session_id.unwrap_or("").to_string(),
        session_title: session_title.map(|s| s.to_string()),
        summary,
        turn_counts: (user_turns, assistant_turns, tool_messages),
        tool_activity,
        recent_files: file_paths,
        last_user_prompt,
        last_assistant_text,
    }
}

fn recent_turn_window<'a>(
    messages: &'a [crate::Message],
    window: usize,
) -> Vec<&'a crate::Message> {
    let mut count = 0;
    let mut cut = 0;
    for i in (0..messages.len()).rev() {
        if messages[i].role == crate::MessageRole::User || messages[i].role == crate::MessageRole::Assistant {
            count += 1;
            if count >= window {
                cut = i;
                break;
            }
        }
    }
    messages[cut..].iter().collect()
}

fn latest_text(messages: &[crate::Message], role: crate::MessageRole) -> Option<String> {
    messages.iter().rev().find(|m| m.role == role).map(|m| {
        match &m.content {
            crate::MessageContent::Text(t) => t.clone(),
            crate::MessageContent::Parts(parts) => {
                parts.iter()
                    .filter_map(|p| match p {
                        crate::MessagePart::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            crate::MessageContent::Image { .. } => String::new(),
        }
    })
}

fn extract_file_path(arguments: &serde_json::Value, tool_name: &str) -> Option<String> {
    let path_key = match tool_name {
        "write_file" | "patch" | "read_file" => "path",
        "skill_manage" | "skill_view" => "file_path",
        _ => return None,
    };
    arguments.get(path_key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten_path(path: &str) -> String {
    let path = path.trim_start_matches('~');
    path.to_string()
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

    /// Replace messages (used by retry, undo, compress operations).
    pub fn replace_messages(&mut self, messages: Vec<crate::Message>) {
        self.messages = messages;
        self.updated_at = chrono::Utc::now();
    }
}

// ── MessageStore: test double for SessionStore ─────────────────────────────

/// Thin wrapper so tests can construct a `SessionStore` from a plain
/// `Vec<Message>` in one line.
pub struct MessageStore {
    session: Session,
}

impl MessageStore {
    pub fn new(session_id: impl Into<String>, messages: Vec<crate::Message>) -> Self {
        let now = chrono::Utc::now();
        Self {
            session: Session {
                id: session_id.into(),
                name: "msg-store".into(),
                created_at: now,
                updated_at: now,
                messages,
                memory_context: None,
                summary_chunks: Vec::new(),
                persisted_message_count: 0,
                metadata: SessionMetadata::default(),
            },
        }
    }

    /// Returns the current session (by value).
    pub fn into_session(self) -> Session {
        self.session
    }
}

impl SessionStore for MessageStore {
    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        if self.session.id == session_id {
            Some(&mut self.session)
        } else {
            None
        }
    }

    fn session(&self, session_id: &str) -> Option<&Session> {
        if self.session.id == session_id {
            Some(&self.session)
        } else {
            None
        }
    }
}
