//! Telegram platform adapter via long polling.
//!
//! Uses `reqwest` for Telegram Bot API HTTP calls and polls for new updates.
//! Default mode is long polling; webhook mode requires a public HTTPS endpoint.

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::platform::{IncomingMessage, OutgoingMessage, PlatformAdapter};

// ---------------------------------------------------------------------------
// Telegram Bot API types
// ---------------------------------------------------------------------------

/// Telegram Bot API base URL.
const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// Telegram message limit: 4096 UTF-16 code units.
const TELEGRAM_MESSAGE_LIMIT: usize = 4096;

/// Telegram update polling timeout in seconds.
const POLL_TIMEOUT: u64 = 30;

// ---------------------------------------------------------------------------
// Telegram API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    result: Vec<Update>,
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse {
    ok: bool,
    description: Option<String>,
    #[serde(default)]
    error_code: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: u64,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    message_id: u64,
    from: Option<User>,
    chat: Chat,
    text: Option<String>,
    caption: Option<String>,
    reply_to_message: Option<Box<Message>>,
    message_thread_id: Option<u64>,
    #[serde(rename = "is_forum")]
    is_forum: Option<bool>,
    entity: Option<Vec<MessageEntity>>,
    photo: Option<Vec<PhotoSize>>,
    document: Option<Document>,
    audio: Option<Audio>,
    voice: Option<Voice>,
    video_note: Option<VideoNote>,
}

#[derive(Debug, Deserialize)]
struct User {
    id: u64,
    first_name: Option<String>,
    username: Option<String>,
    is_bot: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
    r#type: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageEntity {
    r#type: String,
    offset: u32,
    length: u32,
}

#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
}

#[derive(Debug, Deserialize)]
struct Document {
    file_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Audio {
    file_id: String,
    title: Option<String>,
    performer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Voice {
    file_id: String,
    duration: u32,
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VideoNote {
    file_id: String,
    duration: u32,
}

// ---------------------------------------------------------------------------
// Telegram API request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SendChatActionRequest<'a> {
    chat_id: i64,
    action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SendPhotoRequest<'a> {
    chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    photo: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SendDocumentRequest<'a> {
    chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    document: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SendVoiceRequest<'a> {
    chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<u64>,
}

// ---------------------------------------------------------------------------
// TelegramConfig (re-exported convenience)
// ---------------------------------------------------------------------------

pub use oben_config::TelegramConfig;

// ---------------------------------------------------------------------------
// MarkdownV2 escaping
// ---------------------------------------------------------------------------

/// Escape special characters for Telegram MarkdownV2 parse mode.
///
/// Special characters: `_`, `*`, `[`, `]`, `(`, `)`, `~`, `` ` ``, `>`, `#`,
/// `+`, `-`, `=`, `|`, `{`, `}`, `.`, `!`
pub fn escape_markdown_v2(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#'
            | '+' | '-' | '=' | '|' | '{' | '}' | '.' | '!' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

// ---------------------------------------------------------------------------
// UTF-16 aware text splitting for Telegram's 4096 char limit
// ---------------------------------------------------------------------------

/// Split `content` into segments of at most `max_len` UTF-16 code units.
///
/// Returns a Vec of segments, each safe to send as a single Telegram message.
/// Uses binary search over character positions to find boundaries that stay
/// within the UTF-16 limit.
fn split_message_utf16(content: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 || content.is_empty() {
        return vec![];
    }

    let mut segments = Vec::new();
    let mut remaining = content;

    while !remaining.is_empty() {
        let utf16_len = remaining.encode_utf16().count();

        if utf16_len <= max_len {
            segments.push(remaining.to_string());
            break;
        }

        // Binary search for the largest prefix with <= max_len UTF-16 units
        let char_positions: Vec<(usize, char)> = remaining.char_indices().collect();
        let mut lo: usize = 0;
        let mut hi: usize = char_positions.len().saturating_sub(1);
        let mut best = 0usize;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let cut_pos = char_positions[mid].0;
            let prefix = &remaining[..cut_pos];
            if prefix.encode_utf16().count() <= max_len {
                best = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }

        let char_start = char_positions[best].0;
        let char_end = char_start + char_positions[best].1.len_utf8();
        let prefix_with_char = &remaining[..char_end];
        let prefix_utf16 = prefix_with_char.encode_utf16().count();
        
        let cut_idx = if best == 0 {
            remaining.chars().next().unwrap().len_utf8()
        } else if prefix_utf16 <= max_len {
            char_end
        } else {
            char_start
        };

        segments.push(remaining[..cut_idx].to_string());
        remaining = &remaining[cut_idx..];
    }

    segments
}

// ---------------------------------------------------------------------------
// Allowed-users/chats filtering
// ---------------------------------------------------------------------------

fn is_allowed(
    user_id: &str,
    chat_id: &str,
    allowed_users: &[String],
    allowed_chats: &[String],
) -> bool {
    let users_empty = allowed_users.is_empty();
    let chats_empty = allowed_chats.is_empty();

    if users_empty && chats_empty {
        return true;
    }

    if !users_empty && !allowed_users.contains(&user_id.to_string()) {
        return false;
    }

    if !chats_empty && !allowed_chats.contains(&chat_id.to_string()) {
        return false;
    }

    true
}

// ---------------------------------------------------------------------------
// Message extraction from Telegram Update
// ---------------------------------------------------------------------------

/// Extract text content from a Telegram message.
///
/// Priority: text > caption. If both exist, prefer text.
fn extract_message_content(msg: &Message) -> Option<String> {
    if let Some(text) = &msg.text {
        if !text.trim().is_empty() {
            return Some(text.clone());
        }
    }
    if let Some(caption) = &msg.caption {
        if !caption.trim().is_empty() {
            return Some(caption.clone());
        }
    }
    None
}

/// Extract username from a Telegram message's `from` field.
fn extract_username(msg: &Message) -> Option<String> {
    msg.from
        .as_ref()
        .and_then(|u| u.username.clone())
        .or_else(|| msg.from.as_ref().and_then(|u| u.first_name.clone()))
}

/// Convert a Telegram chat ID to a platform routing key.
///
/// For groups: "chat/{id}" → acts as guild key.
/// For DMs: "user/{id}" → acts as DM key.
fn chat_to_thread_key(chat: &Chat) -> String {
    format!("chat/{}", chat.id)
}

// ---------------------------------------------------------------------------
// TelegramAdapter
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TelegramAdapter {
    state: SharedState,
}

/// Shared state — clone is cheap (Arc increment).
#[derive(Clone)]
struct SharedState {
    client: Client,
    token: String,
    dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>,
    webhook_url: Option<String>,
    webhook_secret: Option<String>,
    allowed_users: Vec<String>,
    allowed_chats: Vec<String>,
    forum_topics: bool,
    stop_tx: std::sync::Arc<std::sync::Mutex<Option<mpsc::Sender<()>>>>,
    is_started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_offset: std::sync::Arc<std::sync::Mutex<i64>>,
}

impl SharedState {
    fn new(
        config: TelegramConfig,
        dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest Client should always build"),
            token: config
                .token
                .clone()
                .expect("Telegram bot token must be configured"),
            dispatcher,
            webhook_url: config.webhook_url,
            webhook_secret: config.webhook_secret,
            allowed_users: config.allowed_users,
            allowed_chats: config.allowed_chats,
            forum_topics: config.forum_topics,
            stop_tx: std::sync::Arc::new(std::sync::Mutex::new(None)),
            is_started: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_offset: std::sync::Arc::new(std::sync::Mutex::new(-1)),
        }
    }

    /// Build the Bot API URL for a given method.
    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API_BASE, self.token, method)
    }

    /// Poll for updates using long polling.
    ///
    /// Returns Ok(()) when stopped, Err(e) on fatal errors.
    async fn poll_loop(&self) -> Result<()> {
        self.is_started.store(true, std::sync::atomic::Ordering::SeqCst);
        info!(
            token_set = !self.token.is_empty(),
            "Starting Telegram long-polling loop"
        );

        loop {
            // Check for stop signal via a non-blocking try_recv on stop_tx
            let stop_tx = {
                let guard = self.stop_tx.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                guard.clone()
            };

            // Await the poll (self.do_poll()) directly; the outer stop() mechanism
            // handles lifecycle via the stop_tx channel.
            let _ = self.do_poll().await;

            if self.is_shutdown(stop_tx.as_ref()) {
                info!("Telegram adapter shutting down");
                return Ok(());
            }
        }
    }

    /// Check if a stop signal was received.
    fn is_shutdown(&self, stop_tx: Option<&mpsc::Sender<()>>) -> bool {
        stop_tx.is_none()
    }

    /// Single poll attempt — fetches updates and dispatches them.
    async fn do_poll(&self) -> Result<()> {
        let offset = {
            let mut guard = self.last_offset.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };

        let url = format!(
            "{}/bot{}/getUpdates?timeout={}&offset={}",
            TELEGRAM_API_BASE, self.token, POLL_TIMEOUT, offset
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Telegram Bot API")?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            warn!(status = %status, body = %body, "Telegram API error on getUpdates");

            // Handle 429 (rate limit) and 5xx (server errors)
            if status.is_server_error() || status.as_u16() == 429 {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            return Err(anyhow!("Telegram API error {}: {}", status, body));
        }

        let update_resp: GetUpdatesResponse =
            serde_json::from_str(&body).context("Failed to parse getUpdates response")?;

        if !update_resp.ok {
            warn!("Telegram API returned ok=false: {}", body);
            return Ok(());
        }

        if update_resp.result.is_empty() {
            return Ok(());
        }

        debug!(count = update_resp.result.len(), "Received {} updates", update_resp.result.len());

        // Process updates in order
        let mut max_update_id: u64 = if offset >= 0 { offset as u64 } else { 0 };
        for update in &update_resp.result {
            if update.update_id > max_update_id {
                max_update_id = update.update_id;
            }

            if let Some(ref msg) = update.message {
                // Skip bot messages (in case we're in a group with our own bot)
                if let Some(ref from) = msg.from {
                    if from.is_bot == Some(true) {
                        debug!(message_id = msg.message_id, "Skipping own bot message");
                        continue;
                    }
                }

                // Extract content
                let Some(content) = extract_message_content(msg) else {
                    debug!(message_id = msg.message_id, "No text content, skipping");
                    continue;
                };

                // Extract user info
                let user_name = extract_username(msg);
                let from = msg.from.as_ref();

                // Get user ID
                let user_id = from
                    .map(|u| u.id.to_string())
                    .unwrap_or_else(|| msg.message_id.to_string());

                // Check allowed users/chats
                let chat_id_str = msg.chat.id.to_string();
                if !is_allowed(
                    &user_id,
                    &chat_id_str,
                    &self.allowed_users,
                    &self.allowed_chats,
                ) {
                    info!(
                        user_id = %user_id,
                        chat_id = %chat_id_str,
                        "Message from non-allowed user/chat, ignoring"
                    );
                    continue;
                }

                // Handle commands
                if content.starts_with('/') {
                    if let Ok(cmd_msg) = self.handle_command(&user_id, &user_name, &msg.chat, &content, msg).await {
                        if let Err(e) = self.dispatcher.dispatch(cmd_msg).await {
                            warn!("Dispatcher error for command: {}", e);
                        }
                    }
                    continue;
                }

                // Chat ID for session grouping
                let thread_id = if self.forum_topics && msg.chat.r#type == "private" {
                    // Forum topics: DMs on forum groups create topics
                    // For regular private chats, still use user-based routing
                    chat_to_thread_key(&msg.chat)
                } else if let Some(topic_id) = msg.message_thread_id {
                    // Forum topic or reply thread
                    format!("topic/{}", topic_id)
                } else {
                    chat_to_thread_key(&msg.chat)
                };

                let incoming = IncomingMessage {
                    platform: "telegram".into(),
                    user_id,
                    username: user_name,
                    content: content.clone(),
                    thread_id: Some(thread_id),
                };

                debug!(
                    platform = %incoming.platform,
                    user_id = %incoming.user_id,
                    content_preview = %content.chars().take(50).collect::<String>(),
                    "Dispatching incoming Telegram message"
                );

                if let Err(e) = self.dispatcher.dispatch(incoming).await {
                    warn!("Dispatcher error: {}", e);
                }
            }

            // Update last offset
            if update.update_id >= max_update_id {
                let mut guard = self.last_offset.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                *guard = (update.update_id as i64) + 1;
            }
        }

        Ok(())
    }

    /// Handle Telegram commands.
    async fn handle_command(
        &self,
        user_id: &str,
        username: &Option<String>,
        chat: &Chat,
        command: &str,
        msg: &Message,
    ) -> Result<IncomingMessage> {
        // Telegram uses @botname after command: e.g. /ask@mybot "text"
        // Parse to get command base and argument
        let arg_start = if let Some(at_pos) = command.find('@') {
            // Part before @ is the command
            match command[..at_pos].split_once(' ') {
                Some((cmd, arg)) => (cmd, Some(arg.trim().to_string())),
                None => (&command[..at_pos], Option::<String>::None),
            }
        } else {
            match command.split_once(' ') {
                Some((cmd, arg)) => (cmd, Some(arg.trim().to_string())),
                None => (command, Option::<String>::None),
            }
        };

        let cmd = arg_start.0.trim_start_matches('/');
        let _arg = &arg_start.1;

        let thread_id = if let Some(topic_id) = msg.message_thread_id {
            format!("topic/{}", topic_id)
        } else {
            chat_to_thread_key(chat)
        };

        let display_name = username
            .as_deref()
            .unwrap_or_else(|| user_id);

        info!(command = cmd, user = %display_name, "Telegram command received");

        // For /start, /help, /status, /stop — respond directly
        match cmd {
            "start" => {
                let welcome = format!(
                    "Welcome! I'm an AI assistant. I can help you with:\n\
                     • Ask questions\n\
                     • Run commands\n\
                     • Answer general questions\n\n\
                     Try /help for all commands."
                );
                self.send_response(chat.id, thread_id.as_str(), &welcome, msg).await?;
            }
            "help" => {
                let help = format!(
                    "**Available commands:**\n\n\
                     • /start — Start the bot\n\
                     • /help — Show this help\n\
                     • /ask <query> — Ask a question\n\
                     • /reset — Clear conversation history\n\
                     • /status — Show bot status\n\
                     • /stop — Stop current task"
                );
                self.send_response(chat.id, thread_id.as_str(), &help, msg).await?;
            }
            "ask" => {
                // Re-dispatch with the query as-is for agent processing
                let query = _arg.clone().unwrap_or_default();
                return Ok(IncomingMessage {
                    platform: "telegram".into(),
                    user_id: user_id.to_string(),
                    username: username.clone(),
                    content: query,
                    thread_id: Some(thread_id),
                });
            }
            "reset" => {
                // Send a placeholder to trigger session reset on the agent side
                return Ok(IncomingMessage {
                    platform: "telegram".into(),
                    user_id: user_id.to_string(),
                    username: username.clone(),
                    content: "\x00reset_session\x00".to_string(),
                    thread_id: Some(thread_id),
                });
            }
            "status" => {
                let is_running = self.is_started.load(std::sync::atomic::Ordering::SeqCst);
                let status = format!(
                    "**Bot Status:**\n• Mode: Long Polling\n• Status: {}\n\n\
                     Use /ask <question> to interact.",
                    if is_running { "Online" } else { "Offline" }
                );
                self.send_response(chat.id, thread_id.as_str(), &status, msg).await?;
            }
            "stop" => {
                self.send_response(
                    chat.id,
                    thread_id.as_str(),
                    "Stopped. Ready for new input.",
                    msg,
                ).await?;
            }
            _ => {
                return Ok(IncomingMessage {
                    platform: "telegram".into(),
                    user_id: user_id.to_string(),
                    username: username.clone(),
                    content: command.to_string(),
                    thread_id: Some(thread_id),
                });
            }
        }

        // For commands that already sent responses, return a no-op message
        Ok(IncomingMessage {
            platform: "telegram".into(),
            user_id: user_id.to_string(),
            username: username.clone(),
            content: String::new(),
            thread_id: Some(thread_id),
        })
    }

    /// Send a direct response to a Telegram chat (for built-in commands etc.).
    async fn send_response(
        &self,
        chat_id: i64,
        thread_id: &str,
        text: &str,
        _source_msg: &Message,
    ) -> Result<()> {
        self.send_text(chat_id, thread_id, text).await
    }

    /// Send text to a Telegram chat.
    async fn send_text(
        &self,
        chat_id: i64,
        thread_id: &str,
        text: &str,
    ) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        // Show typing indicator
        let _ = self.show_typing(chat_id, thread_id).await;

        // Split if needed (Telegram limit is 4096 UTF-16 code units)
        let chunks = split_message_utf16(text, TELEGRAM_MESSAGE_LIMIT);

        for (i, chunk) in chunks.iter().enumerate() {
            self.send_single_chat(chat_id, thread_id, chunk, i == 0).await?;
        }

        Ok(())
    }

    /// Send a single text message to a Telegram chat.
    async fn send_single_chat(
        &self,
        chat_id: i64,
        thread_id: &str,
        text: &str,
        _first_chunk: bool,
    ) -> Result<()> {
        // Escape MarkdownV2 special characters
        let escaped = escape_markdown_v2(text);

        let mut thread_id_val: Option<u64> = None;
        if !thread_id.is_empty() && thread_id != "global" {
            if thread_id.starts_with("topic/") {
                thread_id_val = thread_id["topic/".len()..]
                    .parse::<u64>()
                    .ok();
            } else if let Ok(id) = thread_id.parse::<u64>() {
                thread_id_val = Some(id);
            }
        }

        let request = SendMessageRequest {
            chat_id,
            text: &escaped,
            parse_mode: Some("MarkdownV2"),
            message_thread_id: thread_id_val,
        };

        let url = self.api_url("sendMessage");
        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send message to Telegram")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            warn!(status = %status, body = %body, "Telegram sendMessage failed");
            return Err(anyhow!("Telegram send failed {}: {}", status, body));
        }

        debug!(chat_id, "Sent message to Telegram");
        Ok(())
    }

    /// Show typing indicator to a Telegram chat.
    async fn show_typing(&self, chat_id: i64, thread_id: &str) -> Result<()> {
        let mut thread_id_val: Option<u64> = None;
        if !thread_id.is_empty() && thread_id != "global" {
            if thread_id.starts_with("topic/") {
                thread_id_val = thread_id["topic/".len()..]
                    .parse::<u64>()
                    .ok();
            } else if let Ok(id) = thread_id.parse::<u64>() {
                thread_id_val = Some(id);
            }
        }

        let request = SendChatActionRequest {
            chat_id,
            action: "typing",
            message_thread_id: thread_id_val,
        };

        let url = self.api_url("sendChatAction");
        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send typing action to Telegram")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(body = %body, "Failed to send typing indicator to Telegram");
        }

        // Typing indicator is fire-and-forget
        Ok(())
    }

    async fn verify_bot(&self) -> Result<()> {
        let url = self.api_url("getMe");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Telegram")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("Bot verification failed {status}: {body}");
        }

        Ok(())
    }
}

impl TelegramAdapter {
    pub fn new(
        config: TelegramConfig,
        dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>,
    ) -> Self {
        Self {
            state: SharedState::new(config, dispatcher),
        }
    }

    fn spawn_loop(&self) {
        let state = self.state.clone();

        tokio::spawn(async move {
            info!("Telegram adapter starting");

            // Verify bot info on startup
            if let Err(e) = state.verify_bot().await {
                error!("Telegram bot verification failed: {}", e);
                // Don't crash on verification failure — keep polling
            }

            loop {
                let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
                state.stop_tx.lock().map_err(|_| anyhow!("Mutex poisoned")).unwrap()
                    .replace(stop_tx);

                state.is_started.store(true, std::sync::atomic::Ordering::SeqCst);

                let poll_fut = state.poll_loop();
                tokio::pin!(poll_fut);

                tokio::select! {
                    result = &mut poll_fut => {
                        match result {
                            Ok(()) => {
                                info!("Telegram poll loop finished cleanly");
                                break;
                            }
                            Err(e) => {
                                warn!("Telegram poll error, retrying in 5s: {}", e);
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                    _ = stop_rx.recv() => {
                        info!("Telegram stop signal received");
                        break;
                    }
                }
            }

            state.is_started.store(false, std::sync::atomic::Ordering::SeqCst);
            info!("Telegram adapter stopped");
        });
    }

    /// Verify the bot token works by fetching bot info.
    async fn verify_bot(&self) -> Result<()> {
        let url = self.state.api_url("getMe");
        let resp = self
            .state
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Telegram")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("Bot verification failed {status}: {body}");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PlatformAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn listen(&mut self) -> Result<()> {
        info!("Telegram adapter starting with dispatcher");
        self.spawn_loop();
        let (tx, mut rx) = mpsc::channel::<()>(1);
        *self.state.stop_tx.lock().map_err(|_| anyhow!("Mutex poisoned"))? = Some(tx);
        let _ = rx.recv().await;
        info!("Telegram adapter stopping");
        Ok(())
    }

    async fn stop(&mut self) {
        let tx = {
            let mut guard = self.state.stop_tx.lock().map_err(|_| anyhow!("Mutex poisoned")).unwrap();
            guard.take()
        };
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
        self.state.is_started.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let content = &msg.content;
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }

        // Parse chat_id from the user_id field — Telegram uses numeric IDs
        let chat_id: i64 = msg
            .user_id
            .parse()
            .map_err(|_| anyhow!("Invalid Telegram chat_id: {}", msg.user_id))?;

        // Determine thread_id (topic ID) from msg.thread_id
        let thread_id = msg
            .thread_id
            .as_deref()
            .unwrap_or("");

        self.state.send_text(chat_id, thread_id, content).await
    }

    async fn health_check(&self) -> bool {
        self.state.is_started.load(std::sync::atomic::Ordering::SeqCst)
            && self.state.verify_bot().await.is_ok()
    }
}

// ---------------------------------------------------------------------------
// TelegramPlatformFactory for PlatformFactory trait
// ---------------------------------------------------------------------------

/// Factory for the Telegram platform.
pub struct TelegramPlatformFactory {
    config: std::sync::Arc<oben_config::TelegramConfig>,
    dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
    response_router: std::sync::Arc<crate::router::ResponseRouter>,
}

impl TelegramPlatformFactory {
    pub fn new(
        config: oben_config::TelegramConfig,
        dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
        response_router: std::sync::Arc<crate::router::ResponseRouter>,
    ) -> Self {
        Self {
            config: std::sync::Arc::new(config),
            dispatcher,
            response_router,
        }
    }

    pub fn spawn(&self) -> tokio::task::AbortHandle {
        let config = std::sync::Arc::clone(&self.config);
        let dispatcher = std::sync::Arc::clone(&self.dispatcher);
        let response_router = std::sync::Arc::clone(&self.response_router);
        tokio::spawn(async move {
            if config.token.is_none() {
                error!("Telegram bot token is not configured");
                return;
            }

            let mut adapter = crate::telegram_adapter::TelegramAdapter::new(
                (*config).clone(),
                dispatcher,
            );

            // Register a clone with the response router so outbound replies can find it.
            response_router.register("telegram", Box::new(adapter.clone())).await;

            // Start listen on the original adapter instance.
            if let Err(e) = adapter.listen().await {
                tracing::error!("Telegram adapter crashed: {e}");
            }
        })
        .abort_handle()
    }
}

impl crate::platform::PlatformFactory for TelegramPlatformFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        TelegramPlatformFactory::spawn(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- MarkdownV2 escaping ---

    /// Given: A string with no special characters
    /// When: escape_markdown_v2 is called
    /// Then: Returns the original string unchanged
    #[test]
    fn test_escape_markdown_v2_no_special() {
        assert_eq!(escape_markdown_v2("Hello World"), "Hello World");
    }

    /// Given: A string with underscore
    /// When: escape_markdown_v2 is called
    /// Then: Returns the string with underscore escaped
    #[test]
    fn test_escape_markdown_v2_underscore() {
        assert_eq!(escape_markdown_v2("Hello_World"), "Hello\\_World");
    }

    /// Given: A string with multiple MarkdownV2 special characters
    /// When: escape_markdown_v2 is called
    /// Then: All special characters are escaped
    #[test]
    fn test_escape_markdown_v2_multiple() {
        let input = "[link](url)";
        let result = escape_markdown_v2(input);
        assert_eq!(result, "\\[link\\]\\(url\\)");
    }

    /// Given: A string with backslash
    /// When: escape_markdown_v2 is called
    /// Then: Backslash itself is escaped
    #[test]
    fn test_escape_markdown_v2_backslash() {
        assert_eq!(escape_markdown_v2(r"\test"), "\\\\test");
    }

    /// Given: An empty string
    /// When: escape_markdown_v2 is called
    /// Then: Returns empty string
    #[test]
    fn test_escape_markdown_v2_empty() {
        assert_eq!(escape_markdown_v2(""), "");
    }

    /// Given: A CJK string (no MarkdownV2 special chars in CJK range)
    /// When: escape_markdown_v2 is called
    /// Then: Returns the original string unchanged
    #[test]
    fn test_escape_markdown_v2_cjk() {
        assert_eq!(escape_markdown_v2("你好世界"), "你好世界");
    }

    /// Given: A mixed string with CJK and MarkdownV2 special chars
    /// When: escape_markdown_v2 is called
    /// Then: Only ASCII special chars are escaped, CJK passed through
    #[test]
    fn test_escape_markdown_v2_mixed() {
        assert_eq!(escape_markdown_v2("你好_World"), "你好\\_World");
    }

    // --- UTF-16 text splitting ---

    /// Given: An empty string with any max_len
    /// When: split_message_utf16 is called
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_utf16_empty() {
        assert!(split_message_utf16("", 100).is_empty());
    }

    /// Given: A string under the UTF-16 limit
    /// When: split_message_utf16 is called
    /// Then: Returns the original string as a single element
    #[test]
    fn test_split_utf16_under_limit() {
        let result = split_message_utf16("hello", 100);
        assert_eq!(result, vec!["hello"]);
    }

    /// Given: A string exactly at the limit
    /// When: split_message_utf16 is called with the string's length as max_len
    /// Then: Returns the original string as a single element
    #[test]
    fn test_split_utf16_exact_limit() {
        let s = "hello";
        let result = split_message_utf16(s, 5);
        assert_eq!(result, vec!["hello"]);
    }

    /// Given: An ASCII string exceeding the UTF-16 limit
    /// When: split_message_utf16 is called with max_len = 10
    /// Then: Returns segments each with <= 10 UTF-16 code units
    #[test]
    fn test_split_utf16_ascii() {
        let result = split_message_utf16("Hello, World! This is a test.", 10);
        for chunk in &result {
            assert!(chunk.encode_utf16().count() <= 10, "Chunk '{}' exceeds limit", chunk);
        }
    }

    /// Given: A string with emoji that encode as 2 UTF-16 code units each
    /// When: split_message_utf16 is called with a tight limit
    /// Then: Respects UTF-16 code unit boundaries, not char boundaries
    #[test]
    fn test_split_utf16_with_emoji() {
        let with_emoji = "Hi👋Hello, World!";
        let result = split_message_utf16(with_emoji, 5);
        for chunk in &result {
            let utf16_count = chunk.encode_utf16().count();
            assert!(utf16_count <= 5, "Chunk '{}' has {} UTF-16 units", chunk, utf16_count);
        }
    }

    /// Given: A CJK string
    /// When: split_message_utf16 is called with max_len = 2
    /// Then: Splits correctly (CJK chars are 1 UTF-16 unit each)
    #[test]
    fn test_split_utf16_cjk() {
        let cjk = "你好世界今天天气如何";
        let result = split_message_utf16(cjk, 2);
        assert_eq!(result[0], "你好");
        assert_eq!(result[1], "世界");
        for chunk in &result {
            assert!(chunk.encode_utf16().count() <= 2);
        }
    }

    /// Given: max_len = 0
    /// When: split_message_utf16 is called
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_utf16_zero_limit() {
        assert!(split_message_utf16("test", 0).is_empty());
    }

    // --- Allowed users/chats filtering ---

    /// Given: Empty allowed_users and allowed_chats lists
    /// When: is_allowed is called
    /// Then: Returns true for any user/chat
    #[test]
    fn test_filter_no_restrictions() {
        assert!(is_allowed("user1", "chat1", &[], &[]));
        assert!(is_allowed("user99", "chat99", &[], &[]));
    }

    /// Given: A list of allowed users that doesn't include the target
    /// When: is_allowed is called with that user
    /// Then: Returns false
    #[test]
    fn test_filter_blocked_user() {
        let allowed = vec!["user1".to_string(), "user2".to_string()];
        assert!(!is_allowed("user3", "chat1", &allowed, &[]));
    }

    /// Given: A list of allowed users that does include the target
    /// When: is_allowed is called with that user
    /// Then: Returns true
    #[test]
    fn test_filter_allowed_user() {
        let allowed = vec!["user1".to_string(), "user2".to_string()];
        assert!(is_allowed("user1", "chat1", &allowed, &[]));
    }

    /// Given: A list of allowed chats that doesn't include the target
    /// When: is_allowed is called with that chat
    /// Then: Returns false
    #[test]
    fn test_filter_blocked_chat() {
        let allowed = vec!["chat1".to_string(), "chat2".to_string()];
        assert!(!is_allowed("user1", "chat3", &[], &allowed));
    }

    /// Given: Both allowed_users and allowed_chats lists
    /// When: is_allowed is called with a valid user and valid chat
    /// Then: Returns true
    #[test]
    fn test_filter_both_allowed() {
        let users = vec!["user1".to_string()];
        let chats = vec!["chat1".to_string()];
        assert!(is_allowed("user1", "chat1", &users, &chats));
    }

    /// Given: Both allowed_users and allowed_chats lists
    /// When: is_allowed is called with valid user but non-allowed chat
    /// Then: Returns false
    #[test]
    fn test_filter_user_allowed_chat_blocked() {
        let users = vec!["user1".to_string()];
        let chats = vec!["chat2".to_string()];
        assert!(!is_allowed("user1", "chat1", &users, &chats));
    }

    // --- Message content extraction ---

    /// Given: A message with text field
    /// When: extract_message_content is called
    /// Then: Returns the text
    #[test]
    fn test_extract_message_content_text() {
        let msg = Message {
            message_id: 1,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: Some("Hello".into()),
            caption: None,
            reply_to_message: None,
            message_thread_id: None,
            is_forum: None,
            entity: None,
            photo: None,
            document: None,
            audio: None,
            voice: None,
            video_note: None,
        };
        assert_eq!(extract_message_content(&msg), Some("Hello".to_string()));
    }

    /// Given: A message with only caption, no text
    /// When: extract_message_content is called
    /// Then: Returns the caption
    #[test]
    fn test_extract_message_content_caption() {
        let msg = Message {
            message_id: 2,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: None,
            caption: Some("Photo caption".into()),
            reply_to_message: None,
            message_thread_id: None,
            is_forum: None,
            entity: None,
            photo: None,
            document: None,
            audio: None,
            voice: None,
            video_note: None,
        };
        assert_eq!(extract_message_content(&msg), Some("Photo caption".to_string()));
    }

    /// Given: A message with both caption but text takes priority
    /// When: extract_message_content is called
    /// Then: Returns the text, not the caption
    #[test]
    fn test_extract_message_content_text_priority() {
        let msg = Message {
            message_id: 3,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: Some("Message text".into()),
            caption: Some("Caption text".into()),
            reply_to_message: None,
            message_thread_id: None,
            is_forum: None,
            entity: None,
            photo: None,
            document: None,
            audio: None,
            voice: None,
            video_note: None,
        };
        assert_eq!(extract_message_content(&msg), Some("Message text".to_string()));
    }

    /// Given: A message with neither text nor caption
    /// When: extract_message_content is called
    /// Then: Returns None
    #[test]
    fn test_extract_message_content_none() {
        let msg = Message {
            message_id: 4,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: None,
            caption: None,
            reply_to_message: None,
            message_thread_id: None,
            is_forum: None,
            entity: None,
            photo: None,
            document: None,
            audio: None,
            voice: None,
            video_note: None,
        };
        assert_eq!(extract_message_content(&msg), None);
    }

    /// Given: A message with empty text but a caption
    /// When: extract_message_content is called
    /// Then: Returns the caption (empty text is skipped)
    #[test]
    fn test_extract_empty_text_falls_to_caption() {
        let msg = Message {
            message_id: 5,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: Some("   ".into()),
            caption: Some("Caption".into()),
            reply_to_message: None,
            message_thread_id: None,
            is_forum: None,
            entity: None,
            photo: None,
            document: None,
            audio: None,
            voice: None,
            video_note: None,
        };
        assert_eq!(extract_message_content(&msg), Some("Caption".to_string()));
    }

    // --- Username extraction ---

    /// Given: A message with both username and first_name
    /// When: extract_username is called
    /// Then: Returns the username
    #[test]
    fn test_extract_username_with_username() {
        let msg = Message {
            message_id: 1,
            from: Some(User { id: 123, first_name: Some("Alice".into()), username: Some("alice_u".into()), is_bot: Some(false) }),
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: None, caption: None, reply_to_message: None,
            message_thread_id: None, is_forum: None, entity: None,
            photo: None, document: None, audio: None, voice: None, video_note: None,
        };
        assert_eq!(extract_username(&msg), Some("alice_u".into()));
    }

    /// Given: A message with only first_name
    /// When: extract_username is called
    /// Then: Returns the first_name
    #[test]
    fn test_extract_username_first_name_fallback() {
        let msg = Message {
            message_id: 1,
            from: Some(User { id: 123, first_name: Some("Bob".into()), username: None, is_bot: None }),
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: None, caption: None, reply_to_message: None,
            message_thread_id: None, is_forum: None, entity: None,
            photo: None, document: None, audio: None, voice: None, video_note: None,
        };
        assert_eq!(extract_username(&msg), Some("Bob".into()));
    }

    /// Given: A message with no from field
    /// When: extract_username is called
    /// Then: Returns None
    #[test]
    fn test_extract_username_no_from() {
        let msg = Message {
            message_id: 1,
            from: None,
            chat: Chat { id: 123, r#type: "private".into(), title: None },
            text: None, caption: None, reply_to_message: None,
            message_thread_id: None, is_forum: None, entity: None,
            photo: None, document: None, audio: None, voice: None, video_note: None,
        };
        assert_eq!(extract_username(&msg), None);
    }

    // --- Chat threading key ---

    /// Given: A private chat
    /// When: chat_to_thread_key is called
    /// Then: Returns "user/{id}" format
    #[test]
    fn test_chat_thread_key_private() {
        let chat = Chat { id: 987, r#type: "private".into(), title: None };
        assert_eq!(chat_to_thread_key(&chat), "chat/987");
    }

    /// Given: A group chat
    /// When: chat_to_thread_key is called
    /// Then: Returns "chat/{id}" format
    #[test]
    fn test_chat_thread_key_group() {
        let chat = Chat { id: -1001234567890, r#type: "group".into(), title: Some("My Group".into()) };
        assert_eq!(chat_to_thread_key(&chat), "chat/-1001234567890");
    }
}
