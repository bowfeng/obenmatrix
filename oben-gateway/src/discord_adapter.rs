//! Discord platform adapter via Gateway bot.
//!
//! Uses `discordrs` for the WebSocket gateway connection and reqwest for REST API calls.
//! Pattern matches QQBotAdapter: SharedState + event handler for incoming, REST for outgoing.

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use discordrs::{Context, Interaction, Message};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::platform::{IncomingMessage, OutgoingMessage, PlatformAdapter};
use crate::router::ResponseRouter;
use oben_config::DiscordConfig as DiscordPlatformConfig;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_MESSAGE_LENGTH: usize = 2000;
const SPLIT_THRESHOLD: usize = 1900;

// ---------------------------------------------------------------------------
// Configuration wrapper (converted from oben_config::DiscordConfig)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DiscordAdapterConfig {
    pub token: String,
    pub intents: Vec<DiscordIntent>,
    pub allowed_guilds: Vec<String>,
    pub allowed_users: Vec<String>,
    pub slash_commands: bool,
    pub voice: bool,
    pub dm_role_auth_guild: Option<String>,
}

/// Local intent variants (mirrors oben_config::DiscordIntent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordIntent {
    GuildMessages,
    GuildMessageTyping,
    DirectMessages,
    DirectMessageTyping,
    GuildMessagesReactions,
    MessageContent,
}

/// Convert from the config-domain intent enum to the adapter-local enum.
impl From<oben_config::DiscordIntent> for DiscordIntent {
    fn from(intent: oben_config::DiscordIntent) -> Self {
        match intent {
            oben_config::DiscordIntent::GuildMessages => DiscordIntent::GuildMessages,
            oben_config::DiscordIntent::GuildMessageTyping => DiscordIntent::GuildMessageTyping,
            oben_config::DiscordIntent::DirectMessages => DiscordIntent::DirectMessages,
            oben_config::DiscordIntent::DirectMessageTyping => DiscordIntent::DirectMessageTyping,
            oben_config::DiscordIntent::GuildMessagesReactions => {
                DiscordIntent::GuildMessagesReactions
            }
            oben_config::DiscordIntent::MessageContent => DiscordIntent::MessageContent,
        }
    }
}

impl Default for DiscordIntent {
    fn default() -> Self {
        Self::GuildMessages
    }
}

impl DiscordIntent {
    /// Convert to a raw u64 intent bit flag.
    fn to_intent_bits(&self) -> u64 {
        match self {
            DiscordIntent::GuildMessages => 1 << 9,
            DiscordIntent::GuildMessageTyping => 1 << 11,
            DiscordIntent::DirectMessages => 1 << 12,
            DiscordIntent::DirectMessageTyping => 1 << 14,
            DiscordIntent::GuildMessagesReactions => 1 << 10,
            DiscordIntent::MessageContent => 1 << 15,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state — clone is cheap (Arc increment).
// ---------------------------------------------------------------------------

struct SharedState {
    dispatcher: Arc<super::dispatcher::Dispatcher>,
    token: String,
    intents_bits: u64,
    allowed_guilds: Vec<String>,
    allowed_users: Vec<String>,
    slash_commands: bool,
    voice: bool,
    stop_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    is_started: Arc<AtomicBool>,
    message_dedup: Arc<Mutex<HashSet<String>>>,
    response_router: Arc<RwLock<Option<Arc<ResponseRouter>>>>,
}

impl Clone for SharedState {
    fn clone(&self) -> Self {
        Self {
            dispatcher: Arc::clone(&self.dispatcher),
            token: self.token.clone(),
            intents_bits: self.intents_bits,
            allowed_guilds: self.allowed_guilds.clone(),
            allowed_users: self.allowed_users.clone(),
            slash_commands: self.slash_commands,
            voice: self.voice,
            stop_tx: Arc::clone(&self.stop_tx),
            is_started: Arc::clone(&self.is_started),
            message_dedup: Arc::clone(&self.message_dedup),
            response_router: Arc::clone(&self.response_router),
        }
    }
}

impl SharedState {
    fn new(
        config: DiscordAdapterConfig,
        dispatcher: Arc<super::dispatcher::Dispatcher>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        // Combine intent flags into a single u64 bitfield
        let mut intents_bits: u64 = 0;
        for intent in &config.intents {
            intents_bits |= intent.to_intent_bits();
        }

        Self {
            dispatcher,
            token: config.token,
            intents_bits,
            allowed_guilds: config.allowed_guilds,
            allowed_users: config.allowed_users,
            slash_commands: config.slash_commands,
            voice: config.voice,
            stop_tx: Arc::new(Mutex::new(None)),
            is_started: Arc::new(AtomicBool::new(false)),
            message_dedup: Arc::new(Mutex::new(HashSet::new())),
            response_router: Arc::new(RwLock::new(Some(response_router))),
        }
    }

    /// Register a handler with the response router.
    async fn register_response(&self) {
        let router = {
            let guard = self.response_router.read().await;
            guard.clone()
        };
        if let Some(router) = router {
            if let Some(handler) = self.response_handler() {
                router.register("discord", Box::new(handler)).await;
            }
        }
    }

    fn response_handler(&self) -> Option<DiscordResponseHandler> {
        Some(DiscordResponseHandler {
            token: self.token.clone(),
            allowed_guilds: self.allowed_guilds.clone(),
            allowed_users: self.allowed_users.clone(),
        })
    }
}

/// Lightweight response handler registered with the `ResponseRouter` so outbound
/// replies can be delivered back through the Discord adapter.  This struct only
/// holds the fields needed for `PlatformAdapter::send`.  It never touches the
/// gateway connection.
struct DiscordResponseHandler {
    token: String,
    allowed_guilds: Vec<String>,
    allowed_users: Vec<String>,
}

impl Clone for DiscordResponseHandler {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            allowed_guilds: self.allowed_guilds.clone(),
            allowed_users: self.allowed_users.clone(),
        }
    }
}

#[async_trait]
impl PlatformAdapter for DiscordResponseHandler {
    fn name(&self) -> &str {
        "discord"
    }

    async fn listen(&mut self) -> Result<()> {
        Ok(())
    }

    async fn stop(&mut self) {}

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        // Determine target channel
        let channel_id = match &msg.thread_id {
            Some(thread) => thread.clone(),
            None => msg.user_id.clone(),
        };
        let content = msg.content;
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }
        self.rest_send_message(&channel_id, &content).await
    }

    async fn health_check(&self) -> bool {
        !self.token.is_empty()
    }
}

impl DiscordResponseHandler {
    async fn rest_send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let payload = serde_json::json!({ "content": content });
        let client = reqwest::Client::new();
        match client
            .post(format!("https://discord.com/api/v10/channels/{}/messages", channel_id))
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    debug!("REST message sent to channel {}", channel_id);
                    Ok(())
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        channel = %channel_id,
                        status = %status,
                        "Discord REST send failed: {}",
                        body
                    );
                    Err(anyhow!("Discord send failed {}: {}", status, body))
                }
            }
            Err(e) => {
                warn!(channel = %channel_id, "Discord REST request failed: {}", e);
                Err(anyhow!("Discord REST request failed: {}", e))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Message splitting (UTF-8 safe — uses .chars() not byte slicing)
// ---------------------------------------------------------------------------

/// Split a message into chunks of at most MAX_MESSAGE_LENGTH characters,
/// splitting at the nearest whitespace boundary. UTF-8 safe via .chars().
fn split_message(content: &str) -> Vec<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    let max_chars: usize = MAX_MESSAGE_LENGTH;
    let threshold_chars: usize = SPLIT_THRESHOLD;

    let mut chunks = Vec::new();
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();
    let mut start = 0;

    while start < total {
        let remaining = total - start;
        if remaining <= max_chars {
            chunks.push(chars[start..].iter().collect());
            break;
        }

        // Find split point near threshold
        let end = (start + threshold_chars).min(total);
        let chunk_chars: Vec<char> = chars[start..end].to_vec();
        if let Some(pos) = chunk_chars.iter().rev().position(|c| c.is_whitespace()) {
            let split_idx = end - pos;
            chunks.push(chars[start..split_idx].iter().collect::<String>().trim().to_string());
            start = split_idx;
            // Skip leading whitespace on next chunk
            while start < total && chars[start].is_whitespace() {
                start += 1;
            }
        } else {
            // No whitespace found, hard truncate at char boundary
            chunks.push(chars[start..end].iter().collect::<String>().trim().to_string());
            start = end;
        }
    }

    // Ensure no chunk exceeds the limit even after trimming
    for c in &mut chunks {
        if c.len() > max_chars {
            let truncated: String = c.chars().take(max_chars).collect();
            *c = truncated;
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// DiscordAdapter
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DiscordAdapter {
    state: Arc<SharedState>,
}

impl DiscordAdapter {
    pub fn new(
        config: DiscordPlatformConfig,
        dispatcher: Arc<super::dispatcher::Dispatcher>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        let adapter_config = DiscordAdapterConfig {
            token: config.token.unwrap_or_default(),
            intents: config.intents.into_iter().map(DiscordIntent::from).collect(),
            allowed_guilds: config.allowed_guilds.clone(),
            allowed_users: config.allowed_users.clone(),
            slash_commands: config.slash_commands,
            voice: config.voice,
            dm_role_auth_guild: config.dm_role_auth_guild.clone(),
        };

        Self {
            state: Arc::new(SharedState::new(adapter_config, dispatcher, response_router)),
        }
    }

    /// Spawn the gateway listener loop in a background task.
    fn spawn_loop(&self) {
        let state = Arc::clone(&self.state);

        tokio::spawn(async move {
            let (done_tx, mut done_rx) = mpsc::channel::<()>(1);

            loop {
                let (stop_tx, stop_rx) = mpsc::channel::<()>(1);
                {
                    let mut guard = state.stop_tx.lock().await;
                    *guard = Some(stop_tx);
                }
                state.is_started.store(true, Ordering::SeqCst);

                match Self::gateway_run(&state, stop_rx).await {
                    Ok(()) => {
                        info!("Gateway loop finished (clean)");
                        break;
                    }
                    Err(e) => {
                        warn!("Gateway loop error, reconnecting: {}", e);
                    }
                }

                // Backoff before reconnecting
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }

            let _ = done_tx.send(()).await;
            let _ = done_rx.try_recv(); // drain to avoid blocking drop
        });
    }

    /// Connect to the Discord gateway and run the event loop.
    async fn gateway_run(
        state: &Arc<SharedState>,
        mut stop_rx: mpsc::Receiver<()>,
    ) -> Result<()> {
        if state.token.is_empty() {
            return Err(anyhow!("Discord token is empty"));
        }

        info!("Connecting to Discord gateway");

        // Build event handler
        let handler = DiscordEventHandler {
            state: Arc::clone(state),
        };

        // Start the gateway using Client::builder(token, intents).event_handler(handler).start()
        // start() returns () in discordrs 2.0.2 — no Handle/wait/stop methods
        discordrs::Client::builder(&state.token, state.intents_bits)
            .event_handler(handler)
            .start()
            .await
            .context("Failed to start Discord gateway")?;

        // Wait for stop signal (gateway is blocking, stop wakes rx when signaled)
        let _ = stop_rx.recv().await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Event handler — processes incoming Discord events
// ---------------------------------------------------------------------------

struct DiscordEventHandler {
    state: Arc<SharedState>,
}

#[async_trait::async_trait]
impl discordrs::EventHandler for DiscordEventHandler {
    /// Handle incoming guild/personal messages.
    async fn message_create(&self, _ctx: Context, message: Message) {
        self.process_message(&message).await;
    }

    /// Handle slash commands and other interactions.
    async fn interaction_create(
        &self,
        _ctx: Context,
        interaction: Interaction,
    ) {
        self.process_interaction(&interaction).await;
    }
}

// ---------------------------------------------------------------------------
// PlatformAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        "discord"
    }

    async fn listen(&mut self) -> Result<()> {
        info!("Discord adapter starting with dispatcher");

        // Register with response router
        self.state.register_response().await;

        // Start the gateway listener loop
        self.spawn_loop();

        // Hold open until stop signal
        let (tx, mut rx) = mpsc::channel::<()>(1);
        {
            let mut guard = self.state.stop_tx.lock().await;
            *guard = Some(tx);
        }
        let _ = rx.recv().await;
        info!("Discord adapter stopping");
        Ok(())
    }

    async fn stop(&mut self) {
        let tx = {
            let mut guard = self.state.stop_tx.lock().await;
            guard.take()
        };
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let content = &msg.content;
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }

        // Determine target channel: thread_id (guild/channel) or user_id (DM)
        let channel_id = match &msg.thread_id {
            Some(thread) => thread.clone(),
            None => msg.user_id.clone(),
        };

        // Split long messages (Discord limit: 2000 chars)
        let chunks = split_message(content);
        if chunks.is_empty() {
            return Ok(());
        }

        for (i, chunk) in chunks.iter().enumerate() {
            if let Err(e) = self.rest_send_message(&channel_id, chunk).await {
                warn!(chunk_index = i, "Failed to send message chunk: {}", e);
                // Continue trying remaining chunks
            }
        }

        info!(chunks = chunks.len(), channel = %channel_id, "Discord message dispatch complete");
        Ok(())
    }

    async fn health_check(&self) -> bool {
        !self.state.token.is_empty()
    }
}

impl DiscordAdapter {
    /// Send a message via Discord REST API.
    async fn rest_send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let payload = serde_json::json!({ "content": content });
        let client = reqwest::Client::new();

        match client
            .post(format!("https://discord.com/api/v10/channels/{}/messages", channel_id))
            .header("Authorization", format!("Bot {}", self.state.token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    debug!("REST message sent to channel {}", channel_id);
                    Ok(())
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        channel = %channel_id,
                        status = %status,
                        "Discord REST send failed: {}",
                        body
                    );
                    Err(anyhow!("Discord send failed {}: {}", status, body))
                }
            }
            Err(e) => {
                warn!(channel = %channel_id, "Discord REST request failed: {}", e);
                Err(anyhow!("Discord REST request failed: {}", e))
            }
        }
    }
}

// DiscordEventHandler implementation — processes incoming Discord events
impl DiscordEventHandler {
    /// Process an individual Message event.
    async fn process_message(&self, msg: &Message) {
        let msg_id = msg.id.to_string();

        // Dedup: skip if we've already processed this message ID
        {
            let mut dedup = self.state.message_dedup.lock().await;
            if dedup.contains(&msg_id) {
                debug!(msg_id = %msg_id, "Skipping duplicate message");
                return;
            }
            dedup.insert(msg_id.clone());
            if dedup.len() > 10000 {
                dedup.clear();
            }
        }

        // Filter: skip messages from bots
        let author = match msg.author {
            Some(ref a) => a,
            None => return,
        };
        if author.bot.unwrap_or(false) {
            debug!(user_id = %author.id, "Skipping bot message");
            return;
        }

        // Determine if this is a DM
        let _is_dm = msg.channel_id.to_string().starts_with("dm/")
            || msg.guild_id.is_none();

        // For simplicity, treat messages with no guild as potential DMs
        // Filter DMs from non-allowed users
        if !msg.guild_id.is_some() && !self.state.allowed_users.is_empty() {
            let user_id = author.id.to_string();
            if !self.state.allowed_users.iter().any(|a| a == &user_id) {
                debug!(user_id = %user_id, "DM user not in allowed_users, skipping");
                return;
            }
        }

        // For guild messages, check if guild is allowed
        if msg.guild_id.is_some() && !self.state.allowed_guilds.is_empty() {
            if let Some(guild_id) = &msg.guild_id {
                let guild_str = guild_id.to_string();
                if !self.state.allowed_guilds.iter().any(|a| a == &guild_str) {
                    debug!(guild_id = %guild_str, "Guild not in allowed_guilds, skipping");
                    return;
                }
            }
        }

        // Strip bot mention from content
        let mention_strings: Vec<String> = msg
            .mentions
            .iter()
            .filter(|m| m.bot.unwrap_or(false))
            .map(|m| vec![format!("<@!{}>", m.id), format!("<@{}>", m.id)])
            .flatten()
            .collect();

        let mut content = msg.content.clone();
        for mention in &mention_strings {
            content = content.replace(mention, "").trim().to_string();
        }

        // Build thread_id from guild + channel for grouping
        let thread_id = msg
            .guild_id.as_ref()
            .map(|g| format!("{}/{}", g, msg.channel_id))
            .or_else(|| Some(format!("dm/{}", author.id)));

        // Build username (handle discriminator — now Option<String>)
        let username = if let Some(disc) = &author.discriminator {
            Some(format!("{}#{}", author.username, disc))
        } else {
            Some(author.username.clone())
        };

        let incoming = IncomingMessage {
            platform: "discord".to_string(),
            user_id: author.id.to_string(),
            username,
            thread_id,
            content,
        };

        debug!(
            platform = %incoming.platform,
            user_id = %incoming.user_id,
            username = ?incoming.username,
            "Incoming Discord message"
        );

        if let Err(e) = self.state.dispatcher.dispatch(incoming).await {
            error!("Dispatcher error: {}", e);
        }
    }

    /// Process an Interaction event.
    async fn process_interaction(&self, interaction: &Interaction) {
        // Only handle application command interactions
        let Interaction::ChatInputCommand(cmd) = interaction else {
            return;
        };

        let name = match &cmd.data.name {
            Some(n) => n.as_str(),
            None => return,
        };


        if name != "ask" && name != "reset" && name != "status" && name != "stop" {
            warn!(name = name, "Unknown slash command");
            return;
        }

        debug!(name = name, "Received Discord slash command");

        let user_id = match &cmd.context.user {
            Some(u) => u.id.to_string(),
            None => return,
        };

        if !self.state.allowed_users.is_empty() && !self.state.allowed_users.iter().any(|a| a == &user_id) {
            warn!(user_id = %user_id, "Unauthorized slash command");
            return;
        }

        let channel_id = match &cmd.context.channel_id {
            Some(id) => id.to_string(),
            None => return,
        };

        match name {
            "ask" => {
                let text = cmd
                    .data
                    .options
                    .iter()
                    .find(|o| o.name == "text")
                    .and_then(|o| o.value.as_ref())
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if text.is_empty() {
                    let _ = self
                        .state
                        .rest_send_message_to(&channel_id, "Usage: /ask <text>\nSend a message to the agent directly in this channel.")
                        .await;
                    return;
                }

                let thread_id = match (&cmd.context.guild_id, &cmd.context.channel_id) {
                    (Some(g), Some(c)) => Some(format!("{}/{}", g, c)),
                    _ => None,
                };

                let incoming = IncomingMessage {
                    platform: "discord".to_string(),
                    user_id,
                    username: None,
                    thread_id,
                    content: text.to_string(),
                };

                if let Err(e) = self.state.dispatcher.dispatch(incoming).await {
                    error!("Dispatcher error for slash command: {}", e);
                }

                if let Err(e) = self
                    .state
                    .rest_send_message_to(&channel_id, "Got it, processing your request...")
                    .await
                {
                    warn!("Failed to acknowledge slash command: {}", e);
                }
            }
            "reset" => {
                if let Err(e) = self
                    .state
                    .rest_send_message_to(&channel_id, "Session reset.")
                    .await
                {
                    warn!("Failed to respond to /reset: {}", e);
                }
            }
            "status" => {
                let running = self.state.is_started.load(Ordering::SeqCst);
                let status_text = if running { "Online and running" } else { "Offline" };
                if let Err(e) = self
                    .state
                    .rest_send_message_to(&channel_id, status_text)
                    .await
                {
                    warn!("Failed to respond to /status: {}", e);
                }
            }
            "stop" => {
                if let Err(e) = self
                    .state
                    .rest_send_message_to(&channel_id, "Stopping current task...")
                    .await
                {
                    warn!("Failed to respond to /stop: {}", e);
                }
                let tx = {
                    let mut guard = self.state.stop_tx.lock().await;
                    guard.take()
                };
                if let Some(tx) = tx {
                    let _ = tx.send(()).await;
                }
            }
            _ => {
                warn!(name = name, "Unknown slash command");
            }
        }
    }
}

impl SharedState {
    /// REST helper for use from the event handler.  Creates a fresh reqwest
    /// client per call so we don't hold a long-lived `Arc<reqwest::Client>`.
    async fn rest_send_message_to(
        &self,
        channel_id: &str,
        content: &str,
    ) -> Result<()> {
        let payload = serde_json::json!({ "content": content });
        match reqwest::Client::new()
            .post(format!("https://discord.com/api/v10/channels/{}/messages", channel_id))
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    Ok(())
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        channel = %channel_id,
                        status = %status,
                        "Discord REST send failed: {}",
                        body
                    );
                    Err(anyhow!("Discord send failed {}: {}", status, body))
                }
            }
            Err(e) => {
                warn!(channel = %channel_id, "Discord REST request failed: {}", e);
                Err(anyhow!("Discord REST request failed: {}", e))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DiscordPlatformFactory for PlatformFactory trait
// ---------------------------------------------------------------------------

/// Factory for the Discord platform (Gateway bot).
pub struct DiscordPlatformFactory {
    config: std::sync::Arc<DiscordPlatformConfig>,
    dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
    response_router: std::sync::Arc<crate::router::ResponseRouter>,
}

impl DiscordPlatformFactory {
    pub fn new(
        config: DiscordPlatformConfig,
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
            let c = Arc::unwrap_or_clone(config);
            // Clone response_router before moving it into DiscordAdapter::new()
            let response_router_for_register = Arc::clone(&response_router);
            let mut adapter = crate::discord_adapter::DiscordAdapter::new(
                c,
                dispatcher,
                response_router,
            );

            // Register a clone with the response router so outbound replies can find it.
            response_router_for_register.register("discord", Box::new(adapter.clone())).await;

            // Start listen on the original adapter instance.
            if let Err(e) = adapter.listen().await {
                tracing::error!("Discord adapter crashed: {e}");
            }
        })
        .abort_handle()
    }
}

impl crate::platform::PlatformFactory for DiscordPlatformFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        DiscordPlatformFactory::spawn(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- split_message tests --

    #[test]
    fn test_split_empty() {
        let chunks = split_message("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello world");
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn test_split_within_limit() {
        let msg = "a".repeat(MAX_MESSAGE_LENGTH / 2);
        let chunks = split_message(&msg);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].len() <= MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn test_split_long_message() {
        let msg = "hello world ".repeat(300);
        let chunks = split_message(&msg);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_no_whitespace_hard_break() {
        // All chars are "abcde" — no whitespace, should hard-truncate
        let msg = "abcde".repeat(600);
        let chunks = split_message(&msg);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_exact_limit() {
        let msg = "a".repeat(MAX_MESSAGE_LENGTH);
        let chunks = split_message(&msg);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_split_just_over_limit() {
        let msg = "a".repeat(MAX_MESSAGE_LENGTH + 10);
        let chunks = split_message(&msg);
        assert!(chunks.len() >= 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_preserves_word_boundaries() {
        let sentences = vec![
            "first sentence.",
            "second sentence.",
            "third sentence.",
        ];
        let long = sentences.repeat(50).join(" ");
        let chunks = split_message(&long);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // Chunks shouldn't end mid-word (shouldn't end with a letter)
            if !chunk.is_empty() {
                let last_char = chunk.chars().last().unwrap();
                assert!(last_char.is_whitespace() || !last_char.is_alphanumeric(),
                    "Chunk ends mid-word: {:?}", chunk);
            }
        }
    }

    #[test]
    fn test_split_multibyte_utf8() {
        // Multi-byte chars: each Chinese char is 3 bytes in UTF-8
        // But we split by .chars() which counts characters, not bytes
        let msg = "你好世界".repeat(800); // 4 chars each = 3200 chars total
        let chunks = split_message(&msg);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= MAX_MESSAGE_LENGTH);
            // Ensure no truncated multi-byte sequences
            assert!(chunk.chars().next_back().map_or(true, |c| !c.is_ascii()),
                "Potential truncation in Chinese text");
        }
    }

    #[test]
    fn test_split_preserves_emojis() {
        // Emoji are multi-char sequences in Rust (surrogate pairs)
        let msg = "Hello 🦀 world ".repeat(200);
        let chunks = split_message(&msg);
        assert!(chunks.len() > 0);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= MAX_MESSAGE_LENGTH);
            // Crabs should be preserved
            assert!(chunk.contains("🦀") || chunks.last() == Some(chunk));
        }
    }

    #[test]
    fn test_split_single_chunk_no_wrap() {
        let msg = "short";
        let chunks = split_message(msg);
        assert_eq!(chunks, vec!["short"]);
    }

    // -- dedup tests --

    #[test]
    fn test_split_handles_whitespace_only() {
        let chunks = split_message("   ");
        assert!(chunks.is_empty() || chunks.iter().all(|c| c.is_empty()));
    }

    #[test]
    fn test_split_handles_tabs_and_newlines() {
        let msg = "word1\tword2\nword3\tword4";
        let chunks = split_message(msg);
        assert!(chunks.len() >= 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_very_long_single_word() {
        let word = "x".repeat(5000);
        let chunks = split_message(&word);
        // Should produce chunks all under the limit
        assert!(chunks.iter().all(|c| c.len() <= MAX_MESSAGE_LENGTH));
    }
}
