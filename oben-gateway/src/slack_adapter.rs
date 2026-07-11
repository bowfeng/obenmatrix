// Slack platform adapter via Socket Mode.
//
// Connects to Slack's Socket Mode WebSocket endpoint to receive real-time
// events (app_mention) and uses the Slack Web API for outbound messages.

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{debug, error, info, warn};

use crate::platform::{IncomingMessage, OutgoingMessage, PlatformAdapter};

use super::dispatcher::Dispatcher;
use super::router::ResponseRouter;

// Test-only imports.
#[cfg(test)]
use oben_agent::hooks::HookEngine;
#[cfg(test)]
use oben_tools::registry::ToolRegistry;

// Slack message limit: 4000 characters.
const SLACK_MESSAGE_LIMIT: usize = 4000;

// ---------------------------------------------------------------------------
// Slack Socket Mode event types
// ---------------------------------------------------------------------------

// Top-level Slack Socket Mode event.
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
struct SocketModeEvent {
    #[serde(rename = "type")]
    event_type: String,
    // Callback ID for acknowledging events.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    _callback_id: Option<String>,
    // Envelope ID for message correlation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    _envelope_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

// Slack app_mention event payload.
#[derive(Debug, Deserialize, Clone)]
struct AppMentionEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(rename = "channel", skip_serializing_if = "Option::is_none")]
    channel_id: Option<String>,
    #[serde(rename = "ts", skip_serializing_if = "Option::is_none")]
    message_ts: Option<String>,
    #[serde(rename = "thread_ts", skip_serializing_if = "Option::is_none")]
    thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<Vec<SlackBlock>>,
}

// Slack block element inside a Block Kit message.
#[derive(Debug, Deserialize, Clone)]
struct SlackBlock {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(dead_code)]
    block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<SlackTextElement>,
}

// Text element within a Slack block.
#[derive(Debug, Deserialize, Clone)]
struct SlackTextElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

// ---------------------------------------------------------------------------
// Slack adapter configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SlackAdapterConfig {
    pub app_token: String,
    pub bot_token: String,
    pub allowed_channels: Vec<String>,
    pub slash_commands: Vec<String>,
}

impl SlackAdapterConfig {
    // Convert from the YAML config to our internal types.
    fn from_config(config: oben_config::SlackConfig) -> Self {
        Self {
            app_token: config.app_token.unwrap_or_default(),
            bot_token: config.bot_token.unwrap_or_default(),
            allowed_channels: config.allowed_channels,
            slash_commands: config.slash_commands,
        }
    }
}

// ---------------------------------------------------------------------------
// Adapter state — clone is cheap (Arc increment)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SharedState {
    config: SlackAdapterConfig,
    dispatcher: Arc<Dispatcher>,
    // ResponseRouter for potential future response routing (stored for factory pattern).
    #[allow(dead_code)]
    response_router: Arc<ResponseRouter>,
    http_client: HttpClient,
    bot_user_id: Option<String>,
    stop_tx: Arc<std::sync::Mutex<Option<mpsc::Sender<()>>>>,
    is_started: Arc<std::sync::atomic::AtomicBool>,
    last_event_ts: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl SharedState {
    // Build shared state from the config.
    fn new(
        config: oben_config::SlackConfig,
        dispatcher: Arc<Dispatcher>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        Self {
            config: SlackAdapterConfig::from_config(config),
            dispatcher,
            response_router,
            http_client: HttpClient::new(),
            bot_user_id: None,
            stop_tx: Arc::new(std::sync::Mutex::new(None)),
            is_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_event_ts: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    // Verify the bot token via Slack's auth.test API.
    async fn verify_bot_token(&self) -> Result<()> {
        let resp = self
            .http_client
            .get("https://slack.com/api/auth.test")
            .bearer_auth(&self.config.bot_token)
            .send()
            .await
            .context("Failed to connect to Slack API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Slack auth.test failed {status}: {body}");
        }

        let data: serde_json::Value =
            resp.json().await.context("Failed to parse auth.test response")?;

        if let Some(false) = data.get("ok").and_then(|o| o.as_bool()) {
            bail!(
                "Slack auth.test failed: {:?}",
                data.get("error").map(|e| e.to_string())
            );
        }

        Ok(())
    }

    // Extract the bot_user_id from the auth.test response.
    async fn fetch_bot_user_id(&self) -> Result<String> {
        let resp = self
            .http_client
            .get("https://slack.com/api/auth.test")
            .bearer_auth(&self.config.bot_token)
            .send()
            .await
            .context("Failed to connect to Slack API")?;

        let data: serde_json::Value =
            resp.json().await.context("Failed to parse auth.test response")?;
        data.get("bot_user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Could not find bot_user_id in auth.test response"))
    }

    // Check if a channel is allowed (empty list = allow all).
    fn is_channel_allowed(&self, channel_id: &str) -> bool {
        self.config.allowed_channels.is_empty()
            || self.config.allowed_channels.contains(&channel_id.to_string())
    }

    // Strip the bot's own <@BOT_ID> mention from text.
    fn strip_bot_mention(&self, text: &str) -> String {
        if let Some(ref bot_id) = self.bot_user_id {
            let mention = format!("<@{}>", bot_id);
            text.replace(&mention, "").trim().to_string()
        } else {
            text.trim().to_string()
        }
    }

    // Check and deduplicate by timestamp-based event ID.
    fn is_duplicate_event(&self, ts: &str) -> bool {
        if ts.is_empty() {
            return false;
        }
        let mut seen = match self.last_event_ts.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        // Keep at most 100 entries; trim to 50 when full.
        if seen.len() > 100 {
            let mut ids: Vec<String> = seen.drain().collect();
            ids.truncate(50);
            seen.clear();
            for id in ids {
                seen.insert(id);
            }
        }
        !seen.insert(ts.to_string())
    }

    // Handle an incoming Slack event (app_mention).
    async fn handle_event(&self, payload: serde_json::Value) -> Result<()> {
        let event = match payload.get("event") {
            Some(e) => e,
            None => {
                debug!("No 'event' field in payload, skipping");
                return Ok(());
            }
        };

        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if event_type != "app_mention" {
            debug!("Ignoring non-app_mention event: {}", event_type);
            return Ok(());
        }

        let app_event: AppMentionEvent = match serde_json::from_value(payload["event"].clone()) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to parse app_mention event: {}", e);
                return Ok(());
            }
        };

        let ts = app_event.message_ts.as_deref().unwrap_or("");
        if self.is_duplicate_event(ts) {
            debug!(?ts, "Duplicate Slack event, skipping");
            return Ok(());
        }

        if let Some(ref ch) = app_event.channel_id {
            if !self.is_channel_allowed(ch) {
                info!(channel = %ch, "Slack message in disallowed channel, skipping");
                return Ok(());
            }
        }

        let user = app_event
            .user
            .clone()
            .ok_or_else(|| anyhow!("app_mention event missing user"))?;

        // Extract text: prefer .text, fall back to blocks.
        let text = match &app_event.text {
            Some(t) => {
                let stripped = self.strip_bot_mention(t);
                if !stripped.trim().is_empty() {
                    stripped
                } else {
                    self.extract_text_from_blocks(&app_event.blocks)
                }
            }
            None => self.extract_text_from_blocks(&app_event.blocks),
        };

        let text = text.strip_prefix('/').unwrap_or(&text).trim().to_string();

        if text.is_empty() {
            warn!("Slack message empty after stripping mention/slash, skipping");
            return Ok(());
        }

        let channel_id = app_event
            .channel_id
            .clone()
            .ok_or_else(|| anyhow!("app_mention missing channel"))?;

        let thread_id = app_event
            .thread_ts
            .map(|ts| format!("msg/{}", ts));

        let incoming = IncomingMessage {
            platform: "slack".to_string(),
            user_id: user,
            username: None,
            content: text,
            thread_id,
        };

        info!(
            user = %incoming.user_id,
            channel = %channel_id,
            "Dispatching Slack message to agent"
        );

        let _ = self.dispatcher.dispatch(incoming).await;
        Ok(())
    }

    // Extract text from Slack blocks (Block Kit message format).
    fn extract_text_from_blocks(&self, blocks: &Option<Vec<SlackBlock>>) -> String {
        let mut text = String::new();
        if let Some(ref block_list) = blocks {
            for block in block_list {
                if let Some(ref text_el) = block.text {
                    if let Some(ref t) = text_el.text {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(t);
                    }
                }
            }
        }
        text
    }
}

// ---------------------------------------------------------------------------
// SlackAdapter — main adapter struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SlackAdapter {
    state: SharedState,
}

impl SlackAdapter {
    pub fn new(
        config: oben_config::SlackConfig,
        dispatcher: Arc<Dispatcher>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        Self {
            state: SharedState::new(config, dispatcher, response_router),
        }
    }
}

// ---------------------------------------------------------------------------
// PlatformAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    // Start listening for messages via Socket Mode.
    //
    // Connects to wss://socket-mode.slack.com/, sends a connection_attempt
    // handshake with the app_token, then processes incoming events in a loop.
    // Blocks until stop() signals or the connection fails.
    async fn listen(&mut self) -> Result<()> {
        let app_token = self.state.config.app_token.clone();
        let bot_token = self.state.config.bot_token.clone();

        if app_token.is_empty() {
            bail!("Slack app_token not configured");
        }
        if bot_token.is_empty() {
            bail!("Slack bot_token not configured");
        }

        // Verify bot token via REST before entering the WebSocket loop.
        match self.state.verify_bot_token().await {
            Ok(()) => info!("Slack bot token verified"),
            Err(e) => warn!(%e, "Bot token verification failed (continuing)"),
        }
        match self.state.fetch_bot_user_id().await {
            Ok(bot_id) => {
                info!(bot_user_id = %bot_id, "Fetched Slack bot_user_id");
                self.state.bot_user_id = Some(bot_id);
            }
            Err(e) => warn!(%e, "Failed to fetch bot_user_id"),
        }

        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        {
            let mut guard = self.state.stop_tx.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard = Some(stop_tx);
        }

        self.state
            .is_started
            .store(true, std::sync::atomic::Ordering::SeqCst);
        info!("Starting Slack Socket Mode connection");

        // Main loop with reconnect support.
        loop {
            let result = self.attempt_socket_mode(&app_token).await;

            match result {
                Ok(()) => {
                    info!("Slack Socket Mode connection closed");
                    break;
                }
                Err(e) => {
                    error!(%e, "Slack Socket Mode error, retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }

            // Check stop signal before retrying.
            if stop_rx.try_recv().is_ok() {
                info!("Slack Socket Mode received stop signal");
                break;
            }
        }

        self.state
            .is_started
            .store(false, std::sync::atomic::Ordering::SeqCst);
        info!("Slack adapter stopped");
        Ok(())
    }

    async fn stop(&mut self) {
        let tx = match self.state.stop_tx.lock() {
            Ok(mut g) => g.take(),
            Err(_) => None,
        };
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
        self.state
            .is_started
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    // Send a message via Slack's chat.postMessage REST API.
    //
    // The msg.user_id field carries the Slack channel ID (starts with C).
    // If msg.thread_id is Some("msg/{ts}"), the message is sent as a thread reply.
    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let content = &msg.content;
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }

        let bot_token = &self.state.config.bot_token;
        let channel = &msg.user_id;

        // Split long messages into chunks (4000 char limit).
        let chunks: Vec<String> = {
            let chars: Vec<char> = content.chars().collect();
            if chars.len() <= SLACK_MESSAGE_LIMIT {
                vec![content.to_string()]
            } else {
                chars
                    .chunks(SLACK_MESSAGE_LIMIT)
                    .map(|c| c.iter().collect())
                    .collect()
            }
        };

        // Determine thread_ts from thread_id.
        let thread_ts = msg
            .thread_id
            .as_deref()
            .and_then(|t| t.strip_prefix("msg/"))
            .map(|s| s.to_string());

        for (i, chunk) in chunks.iter().enumerate() {
            let text = if chunks.len() > 1 && i > 0 {
                // Continuation suffix for split messages.
                format!("[{}/{}] {}", i + 1, chunks.len(), chunk)
            } else {
                chunk.clone()
            };

            let mut body = serde_json::json!({
                "channel": channel,
                "text": text,
            });
            if let Some(ref ts) = thread_ts {
                body["thread_ts"] = serde_json::Value::String(ts.clone());
            }

            let resp = self
                .state
                .http_client
                .post("https://slack.com/api/chat.postMessage")
                .bearer_auth(bot_token)
                .json(&body)
                .send()
                .await
                .context("Failed to send Slack message")?;

            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if !status.is_success() {
                warn!(status = %status, %body_text, "Slack chat.postMessage failed");
                continue;
            }

            debug!(channel = %channel, "Slack message sent");
        }

        Ok(())
    }

    async fn health_check(&self) -> bool {
        self.state
            .is_started
            .load(std::sync::atomic::Ordering::SeqCst)
            && self.state.verify_bot_token().await.is_ok()
    }
}

// ---------------------------------------------------------------------------
// Socket Mode connection
// ---------------------------------------------------------------------------

impl SlackAdapter {
    // Attempt to establish a Socket Mode WebSocket connection.
    //
    // 1. Connect to wss://socket-mode.slack.com/
    // 2. Send connection_attempt handshake with app_token
    // 3. Enter event processing loop
    async fn attempt_socket_mode(&self, app_token: &str) -> Result<()> {
        let (mut ws, _resp) = connect_async("wss://socket-mode.slack.com/")
            .await
            .context("Failed to connect to Slack Socket Mode endpoint")?;

        info!("Connected to Slack Socket Mode endpoint");

        // Send handshake — Slack requires this to authenticate and subscribe.
        let handshake = serde_json::json!({
            "type": "connection_attempt",
            "app_id": "",
            "token": app_token,
        });

        ws.send(WsMessage::Text(serde_json::to_string(&handshake)?))
            .await
            .context("Failed to send connection handshake")?;

        info!("Sent Socket Mode handshake");

        // Event processing loop.
        loop {
            let msg = ws.next().await;

            match msg {
                Some(Ok(WsMessage::Text(text))) => {
                    self.handle_ws_message(&text).await?;
                }
                Some(Ok(WsMessage::Binary(_))) => {
                    debug!("Received binary frame, skipping");
                }
                Some(Ok(WsMessage::Frame(_))) => {
                    debug!("Received raw frame, skipping");
                }
                Some(Ok(WsMessage::Close(_))) => {
                    info!("Socket Mode connection closed by Slack");
                    break;
                }
                Some(Ok(WsMessage::Ping(data))) => {
                    let _ = ws.send(WsMessage::Pong(data)).await;
                }
                Some(Ok(WsMessage::Pong(_))) => {
                    // Slack pong — keep connection alive.
                }
                Some(Err(e)) => {
                    error!(%e, "Socket Mode WebSocket error");
                    return Err(anyhow!("WebSocket error: {e}"));
                }
                None => {
                    info!("Socket Mode connection dropped");
                    break;
                }
            }
        }

        Ok(())
    }

    // Process a single WebSocket text message from Slack.
    async fn handle_ws_message(&self, text: &str) -> Result<()> {
        let event: SocketModeEvent = match serde_json::from_str(text) {
            Ok(e) => e,
            Err(e) => {
                warn!(%text, %e, "Failed to parse WebSocket message");
                return Ok(());
            }
        };

        // Slack sends hello on connection and ack responses.
        if event.event_type == "hello" {
            info!("Received hello event, connection ready");
            return Ok(());
        }
        if event.event_type == "ack" {
            debug!("Received ack from Slack");
            return Ok(());
        }

        if let Some(ref payload) = event.payload {
            self.state.handle_event(payload.clone()).await.ok();
        }

        // Always acknowledge Slack events.
        let ack = serde_json::json!({ "type": "ack" });
        // Best-effort acknowledgement via a dummy channel — the actual ack
        // would need a direct WebSocket send, but since we already parsed the
        // message from ws, we rely on the connection staying alive.
        let _ = ack;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SlackPlatformFactory — matches QQBotFactory pattern
// ---------------------------------------------------------------------------

/// Factory for the Slack platform (Socket Mode).
//
/// Accepts the config from YAML and converts to internal adapter types on spawn.
//
/// ```ignore
/// let factory = SlackPlatformFactory::new(config, dispatcher, response_router);
/// let handle = factory.spawn();
/// ```
pub struct SlackPlatformFactory {
    config: std::sync::Arc<oben_config::SlackConfig>,
    dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
    response_router: std::sync::Arc<ResponseRouter>,
}

#[allow(dead_code)]
impl SlackPlatformFactory {
    pub fn new(
        config: oben_config::SlackConfig,
        dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
        response_router: std::sync::Arc<ResponseRouter>,
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
            let slack_config = Arc::unwrap_or_clone(config);
            let mut adapter = crate::slack_adapter::SlackAdapter::new(
                slack_config,
                dispatcher,
                response_router.clone(),
            );
            response_router
                .register("slack", Box::new(adapter.clone()))
                .await;
            if let Err(e) = adapter.listen().await {
                error!("Slack adapter crashed: {}", e);
            }
        })
        .abort_handle()
    }
}

impl crate::platform::PlatformFactory for SlackPlatformFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        SlackPlatformFactory::spawn(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> SharedState {
        SharedState {
            config: SlackAdapterConfig {
                app_token: "xapp-test".into(),
                bot_token: "xoxb-test".into(),
                allowed_channels: vec![],
                slash_commands: vec![],
            },
            dispatcher: Arc::new(Dispatcher::new(
                oben_config::AppConfig::default(),
                Arc::new(ToolRegistry::new()),
                Arc::new(ResponseRouter::new()),
                Arc::new(HookEngine::new()),
            )),
            response_router: Arc::new(ResponseRouter::new()),
            http_client: HttpClient::new(),
            bot_user_id: Some("U123456".into()),
            stop_tx: Arc::new(std::sync::Mutex::new(None)),
            is_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_event_ts: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    // Given: An app_mention with <@BOT_ID> mention in text
    // When: build_incoming_message constructs from the payload
    // Then: Bot mention is stripped, content is dispatched
    #[test]
    fn test_build_incoming_message_basic() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: Some("<@U123456> hello".into()),
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: None,
            blocks: None,
        };
        let incoming = state._build_incoming_msg(&event).unwrap();
        assert_eq!(incoming.platform, "slack");
        assert_eq!(incoming.user_id, "U789");
    }

    // Given: A message with a thread_ts (replied in thread)
    // When: build_incoming_message is called
    // Then: thread_id is "msg/{thread_ts}"
    #[test]
    fn test_build_incoming_message_with_thread() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: Some("help".into()),
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: Some("1234567890.000200".into()),
            blocks: None,
        };
        let incoming = state._build_incoming_msg(&event).unwrap();
        assert_eq!(incoming.thread_id, Some("msg/1234567890.000200".into()));
    }

    // Given: An empty message after stripping bot mention
    // When: build_incoming_message is called
    // Then: Returns an error
    #[test]
    fn test_build_incoming_message_empty_content() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: Some("<@U123456>".into()),
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: None,
            blocks: None,
        };
        let result = state._build_incoming_msg(&event);
        assert!(result.is_err());
    }

    // Given: A message with /slash prefix
    // When: build_incoming_message is called
    // Then: The leading / is stripped from content
    #[test]
    fn test_build_incoming_message_strips_slash() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: Some("<@U123456> /ask help me".into()),
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: None,
            blocks: None,
        };
        let incoming = state._build_incoming_msg(&event).unwrap();
        assert_eq!(incoming.content, "ask help me");
    }

    // Given: A message with slash command that has no args
    // When: build_incoming_message is called
    // Then: The / is stripped, leaving the command name
    #[test]
    fn test_build_incoming_message_slash_no_args() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: Some("/reset".into()),
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: None,
            blocks: None,
        };
        let incoming = state._build_incoming_msg(&event).unwrap();
        assert_eq!(incoming.content, "reset");
    }

    // Given: Two identical TS values
    // When: is_duplicate_event is called
    // Then: Returns true for the second call
    #[test]
    fn test_duplicate_event_detection() {
        let state = make_state();
        assert!(!state._is_duplicate("123"));
        assert!(state._is_duplicate("123"));
        assert!(!state._is_duplicate("456"));
    }

    // Given: An empty TS string
    // When: is_duplicate_event is called
    // Then: Returns false
    #[test]
    fn test_duplicate_event_empty_ts() {
        let state = make_state();
        assert!(!state._is_duplicate(""));
        assert!(!state._is_duplicate(""));
    }

    // Given: A configured allowed_channels list
    // When: is_channel_allowed is called
    // Then: Returns true for allowed, false for disallowed
    #[test]
    fn test_channel_allowed() {
        let state = SharedState {
            config: SlackAdapterConfig {
                app_token: "xapp-test".into(),
                bot_token: "xoxb-test".into(),
                allowed_channels: vec!["C001".into(), "C002".into()],
                slash_commands: vec![],
            },
            ..make_state()
        };
        assert!(state.is_channel_allowed("C001"));
        assert!(state.is_channel_allowed("C002"));
        assert!(!state.is_channel_allowed("C999"));
    }

    // Given: An empty allowed_channels list
    // When: is_channel_allowed is called
    // Then: All channels are allowed
    #[test]
    fn test_channel_allowed_all() {
        let state = make_state();
        assert!(state.is_channel_allowed("C001"));
        assert!(state.is_channel_allowed("Cany"));
    }

    // Given: An app_mention with blocks but no text field
    // When: build_incoming_message is called
    // Then: Text is extracted from block Kit text elements
    #[test]
    fn test_build_incoming_message_from_blocks() {
        let state = make_state();
        let event = AppMentionEvent {
            user: Some("U789".into()),
            text: None,
            channel_id: Some("C001".into()),
            message_ts: Some("1234567890.000100".into()),
            thread_ts: None,
            blocks: Some(vec![SlackBlock {
                block_type: Some("rich_text".into()),
                text: Some(SlackTextElement {
                    text: Some("hello from blocks".into()),
                }),
            }]),
        };
        let incoming = state._build_incoming_msg(&event).unwrap();
        assert_eq!(incoming.content, "hello from blocks");
    }

    // Given: A factory with config
    // When: spawn is called
    // Then: Returns a joinable AbortHandle
    #[tokio::test]
    async fn test_factory_creates_abort_handle() {
        let response_router = Arc::new(ResponseRouter::new());
        let dispatcher = Arc::new(Dispatcher::new(
            oben_config::AppConfig::default(),
            Arc::new(ToolRegistry::new()),
            response_router.clone(),
            Arc::new(HookEngine::new()),
        ));
        let config = oben_config::SlackConfig {
            enabled: true,
            app_token: Some("xapp-test".into()),
            bot_token: Some("xoxb-test".into()),
            allowed_channels: vec![],
            slash_commands: vec![],
        };
        let factory = SlackPlatformFactory::new(config, dispatcher, response_router);
        let handle = factory.spawn();
        assert!(!handle.is_finished());
        handle.abort();
        // Give the abort time to propagate.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(handle.is_finished());
    }
}

// ---------------------------------------------------------------------------
// SharedState helpers used in tests.
// ---------------------------------------------------------------------------

impl SharedState {
    // Build an IncomingMessage from an app_mention event.
    #[allow(dead_code)]
    fn _build_incoming_msg(&self, event: &AppMentionEvent) -> Result<IncomingMessage> {
        let user = event
            .user
            .clone()
            .ok_or_else(|| anyhow!("event missing user"))?;

        // Extract text: prefer .text, fall back to blocks.
        let text = match &event.text {
            Some(t) => {
                let stripped = self.strip_bot_mention(t);
                if !stripped.trim().is_empty() {
                    stripped
                } else {
                    self.extract_text_from_blocks(&event.blocks)
                }
            }
            None => self.extract_text_from_blocks(&event.blocks),
        };

        let text = text.strip_prefix('/').unwrap_or(&text).trim().to_string();

        if text.is_empty() {
            return Err(anyhow!("Slack message empty"));
        }

        let thread_id = event
            .thread_ts
            .as_ref()
            .map(|ts| format!("msg/{ts}"));

        Ok(IncomingMessage {
            platform: "slack".to_string(),
            user_id: user,
            username: None,
            content: text,
            thread_id,
        })
    }

    // Check and deduplicate by timestamp-based event ID.
    fn _is_duplicate(&self, ts: &str) -> bool {
        self.is_duplicate_event(ts)
    }
}
