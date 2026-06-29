//! QQ Bot platform adapter via WebSocket Gateway.
//!
//! Uses `tokio-tungstenite` for the WebSocket connection and `reqwest` for REST endpoints.

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use futures::{SinkExt as _, StreamExt as _};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tungstenite::Message as WSMessage;

use crate::platform::{IncomingMessage, MessageHandler, OutgoingMessage, PlatformAdapter};

use super::qq_protocol::{
    HeartbeatPayload, IdentifyPayload, Intents, MsgType, Properties, SendMessageRequest,
    WsIncomingMessage,
};

pub use super::qq_protocol::{CloseCode, EventType, FileUploadResponse, FileType, OpCode};

// ---------------------------------------------------------------------------
// Token management
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TokenCache {
    token: String,
    created_at: std::time::Instant,
}

#[derive(Clone)]
struct TokenManager {
    cache: std::sync::Arc<std::sync::Mutex<Option<TokenCache>>>,
    app_id: String,
    app_secret: String,
}

impl TokenManager {
    fn new(app_id: &str, app_secret: &str, _sandbox: bool) -> Self {
        Self {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    async fn get(&self) -> Result<String> {
        let cached = {
            let guard = self.cache.lock().unwrap();
            guard.as_ref().filter(|c| { c.created_at.elapsed() < Duration::from_secs(7200 - 60) }).cloned()
        };
        if let Some(c) = cached {
            return Ok(c.token);
        }

        let token = self.fetch().await?;
        self.cache.lock().unwrap().replace(TokenCache {
            token: token.clone(),
            created_at: std::time::Instant::now(),
        });
        Ok(token)
    }

    async fn fetch(&self) -> Result<String> {
        let client = Client::new();
        let resp = client
            .post("https://bots.qq.com/app/getAppAccessToken")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "appId": self.app_id,
                "clientSecret": self.app_secret,
            }))
            .send()
            .await
            .context("Failed to connect to OAuth server")?;

        let status = resp.status();
        let status_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            bail!("Token request failed {}: {}", status, status_text);
        }

        let json: serde_json::Value = serde_json::from_str(&status_text)
            .context("Failed to parse token response JSON")?;
        json.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Missing access_token in response"))
    }
}

// ---------------------------------------------------------------------------
// Gateway URL provider
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct GatewayCache {
    url: String,
    cached_at: std::time::Instant,
}

#[derive(Clone)]
struct GatewayUrlProvider {
    base_url: String,
    cache: std::sync::Arc<std::sync::Mutex<Option<GatewayCache>>>,
}

impl GatewayUrlProvider {
    fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    async fn get(&self, token: &str, shard: bool) -> Result<String> {
        let cached = {
            let guard = self.cache.lock().unwrap();
            guard.as_ref().filter(|gc| gc.cached_at.elapsed() < Duration::from_secs(86400)).cloned()
        };
        if let Some(gc) = cached {
            return Ok(gc.url);
        }

        let endpoint = if shard { "/gateway/bot" } else { "/gateway" };
        let resp = Client::new()
            .get(format!("{}{}", self.base_url, endpoint))
            .header("Authorization", format!("QQBot {}", token))
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("Gateway URL request failed: {}", resp.text().await?);
        }

        let json: serde_json::Value = resp.json().await?;
        let url = json
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("No url in gateway response"))?;

        self.cache.lock().unwrap().replace(GatewayCache {
            url: url.to_string(),
            cached_at: std::time::Instant::now(),
        });
        Ok(url.to_string())
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct QQBotConfig {
    pub app_id: String,
    pub app_secret: String,
    pub sandbox: bool,
    pub shard: Option<[usize; 2]>,
    #[allow(dead_code)]
    pub intents: Intents,
}

/// Shared state — clone is cheap (Arc increment).
#[derive(Clone)]
struct SharedState {
    dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>,
    session_id: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    last_seq: std::sync::Arc<std::sync::Mutex<i64>>,
    stop_tx: std::sync::Arc<std::sync::Mutex<Option<mpsc::Sender<()>>>>,
    token_mgr: std::sync::Arc<TokenManager>,
    gateway_url: GatewayUrlProvider,
    intents: Intents,
    shard: Option<[usize; 2]>,
    base_url: String,
}

impl SharedState {
    fn new(config: QQBotConfig, dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>) -> Self {
        let token_mgr = std::sync::Arc::new(TokenManager::new(
            &config.app_id,
            &config.app_secret,
            config.sandbox,
        ));
        let base = if config.sandbox {
            "https://sandbox.api.sgroup.qq.com"
        } else {
            "https://api.sgroup.qq.com"
        }
        .to_string();

        Self {
            dispatcher,
            session_id: std::sync::Arc::new(std::sync::Mutex::new(None)),
            last_seq: std::sync::Arc::new(std::sync::Mutex::new(0)),
            stop_tx: std::sync::Arc::new(std::sync::Mutex::new(None)),
            token_mgr,
            gateway_url: GatewayUrlProvider::new(&base),
            intents: config.intents,
            shard: config.shard,
            base_url: base,
        }
    }

    // ── WS lifecycle helpers ──────────────────────────────────────────

    async fn run_once(
        &mut self,
        stop_rx: mpsc::Receiver<()>,
    ) -> Result<()> {
        let token = self.token_mgr.get().await?;
        let gateway_url = self.gateway_url.get(&token, self.shard.is_some()).await?;
        let ws_url = format!("{}{}", gateway_url, "/websocket");
        info!(ws_url = %ws_url, "Connecting to QQ gateway");

        // Connect
        let (ws_stream, _resp) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .context("WS connect failed")?;
        // Split into read/end halves — tungstenite::Stream yields Result<Message, tungstenite::Error>
        let (mut write, mut read) = ws_stream.split();

        // Receive Hello
        self.receive_hello(&mut read).await?;

        // Identify
        let token = self.token_mgr.get().await?;
        let identify = IdentifyPayload {
            token: format!("QQBot {}", token),
            intents: self.intents.to_u64(),
            shard: self.shard,
            properties: Properties::default(),
        };
        write
            .send(WSMessage::text(serde_json::to_string(&identify)?))
            .await?;

        // Wait for READY
        let ready_data = self.wait_for_dispatch(&mut read, "READY").await?;
        let session_id = ready_data
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        {
            let mut sid = self.session_id.lock().unwrap();
            *sid = Some(session_id);
        }
        info!("Identified successfully (session_ready)");

        // Event loop
        self.event_loop(&mut read, &mut write, stop_rx).await
    }

    async fn receive_hello<S>(&mut self, read: &mut S) -> Result<()>
    where
        S: futures::StreamExt<Item = Result<WSMessage, tungstenite::Error>> + Unpin,
    {
        loop {
            let raw = read
                .next()
                .await
                .context("WS closed before Hello")??; // Option -> Result<Message, E>
            let text = raw.into_text()?;
            if let Ok(msg) = serde_json::from_str::<WsIncomingMessage>(&text) {
                if msg.op == OpCode::Hello {
                    let interval: u64 = serde_json::from_value(msg.d)
                        .map(|h: super::qq_protocol::HelloPayload| h.heartbeat_interval)
                        .unwrap_or(45_000);
                    info!(interval_ms = interval, "Received Hello");
                    return Ok(());
                }
                debug!("Non-Hello frame on connect: {:?}", msg.op);
            }
        }
    }

    async fn wait_for_dispatch<S>(&mut self, read: &mut S, expected: &str) -> Result<serde_json::Value>
    where
        S: futures::StreamExt<Item = Result<WSMessage, tungstenite::Error>> + Unpin,
    {
        loop {
            let raw = read
                .next()
                .await
                .context("WS closed waiting for event")??;
            let text = raw.into_text()?;
            let msg = serde_json::from_str::<WsIncomingMessage>(&text).with_context(|| {
                format!("Parse error: {}", &text[..text.len().min(100)])
            })?;

            match msg.op {
                OpCode::Dispatch => {
                    let t = msg.t.as_deref().unwrap_or("");
                    if t == expected {
                        return Ok(msg.d);
                    }
                    if let Some(s) = msg.s {
                        let mut seq = self.last_seq.lock().unwrap();
                        *seq = s;
                    }
                }
                _ => {}
            }
        }
    }

    async fn event_loop<S, W>(
        &mut self,
        read: &mut S,
        mut write: W,
        mut stop_rx: mpsc::Receiver<()>,
    ) -> Result<()>
    where
        S: futures::StreamExt<Item = Result<WSMessage, tungstenite::Error>> + Unpin,
        W: futures::Sink<WSMessage, Error = tungstenite::Error> + Unpin,
    {
        let mut heartbeat = tokio::time::interval(Duration::from_millis(22_500));
        let mut last_seq: i64 = 0;

        loop {
            tokio::select! {
                biased; // prefer reading

                _ = heartbeat.tick() => {
                    let payload = serde_json::to_string(&HeartbeatPayload(Some(last_seq))).unwrap_or_default();
                    if write.send(WSMessage::text(payload)).await.is_err() {
                        break;
                    }
                }

                _ = stop_rx.recv() => {
                    info!("Stop signal received in event loop");
                    return Ok(());
                }

                maybe_frame = read.next() => {
                    let frame = match maybe_frame {
                        Some(Ok(f)) => f,
                        Some(Err(_)) | None => break,
                    };
                    let text = frame.into_text().with_context(|| "Non-text WS frame")?;
                    let msg = match serde_json::from_str::<WsIncomingMessage>(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            debug!("Parse error: {:?}", e);
                            continue;
                        }
                    };
                    match msg.op {
                        OpCode::Dispatch => {
                            if let Some(s) = msg.s {
                                last_seq = s;
                                let mut seq = self.last_seq.lock().unwrap();
                                *seq = s;
                            }
                            let event_type = msg.t.as_deref().unwrap_or("");
                            if let Some(en) = EventType::from_str(event_type) {
                                if en.is_message_event() {
                                    self.dispatch_message(&en, &msg.d);
                                }
                            }
                        }
                        OpCode::HeartbeatAck => {}
                        OpCode::Hello => {}
                        OpCode::Heartbeat => {}
                        OpCode::Reconnect => break,
                        OpCode::InvalidSession => bail!("Invalid session"),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn dispatch_message(&self, event_type: &EventType, data: &serde_json::Value) {
        let incoming = match event_to_incoming(event_type, data) {
            Ok(msg) => msg,
            Err(e) => {
                warn!("Failed to convert event: {}", e);
                return;
            }
        };
        if let Err(e) = self.dispatcher.dispatch(incoming) {
            error!("Dispatcher error: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// QQBotAdapter
// ---------------------------------------------------------------------------

pub struct QQBotAdapter {
    state: SharedState,
}

impl QQBotAdapter {
    pub fn new(
        app_id: &str,
        app_secret: &str,
        sandbox: bool,
        shard: Option<[usize; 2]>,
        intents: Intents,
        dispatcher: std::sync::Arc<super::dispatcher::Dispatcher>,
    ) -> Self {
        Self {
            state: SharedState::new(
                QQBotConfig {
                    app_id: app_id.to_string(),
                    app_secret: app_secret.to_string(),
                    sandbox,
                    shard,
                    intents,
                },
                dispatcher,
            ),
        }
    }

    fn spawn_loop(&self) {
        let state = self.state.clone();

        tokio::spawn(async move {
            let (done_tx, mut done_rx) = mpsc::channel::<()>(1);

            loop {
                let (stop_tx, stop_rx) = mpsc::channel::<()>(1);
                state.stop_tx.lock().unwrap().replace(stop_tx);

                let mut running = SharedState {
                    dispatcher: state.dispatcher.clone(),
                    session_id: state.session_id.clone(),
                    last_seq: state.last_seq.clone(),
                    stop_tx: state.stop_tx.clone(),
                    token_mgr: state.token_mgr.clone(),
                    gateway_url: state.gateway_url.clone(),
                    intents: state.intents,
                    shard: state.shard,
                    base_url: state.base_url.clone(),
                };

                match running.run_once(stop_rx).await {
                    Ok(_) => {
                        info!("WS loop finished (clean)");
                        break;
                    }
                    Err(e) => {
                        warn!("WS loop error, reconnecting: {}", e);
                    }
                }
            }

            // Signal external listener that we stopped
            let _ = done_tx.send(()).await;
            done_rx.close();
        });
    }
}

// ---------------------------------------------------------------------------
// PlatformAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformAdapter for QQBotAdapter {
    fn name(&self) -> &str {
        "qq_bot"
    }

    async fn listen(&mut self, _handler: Box<dyn MessageHandler>) -> Result<()> {
        info!("QQ Bot adapter starting with dispatcher");
        self.spawn_loop();
        let (tx, mut rx) = mpsc::channel::<()>(1);
        *self.state.stop_tx.lock().unwrap() = Some(tx);
        let _ = rx.recv().await;
        info!("QQ Bot adapter stopping");
        Ok(())
    }

    async fn stop(&mut self) {
        let tx = {
            let mut guard = self.state.stop_tx.lock().unwrap();
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

        let endpoint = match &msg.thread_id {
            Some(thread) if thread.starts_with("group/") => {
                let gid = &thread["group/".len()..];
                format!("{}/v2/groups/{}/messages", self.state.base_url, gid)
            }
            None => format!(
                "{}/v2/users/{}/messages",
                self.state.base_url, msg.user_id
            ),
            Some(thread) => format!(
                "{}/channels/{}/messages",
                self.state.base_url, thread
            ),
        };

        let request = SendMessageRequest {
            content: Some(content.clone()),
            msg_type: Some(MsgType::Text),
            msg_id: None,
            event_id: None,
            msg_seq: Some(1),
            markdown: None,
            reply: None,
        };

        let token = self.state.token_mgr.get().await?;
        let status = Client::new()
            .post(&endpoint)
            .header("Authorization", format!("QQBot {}", token))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .status();

        if !status.is_success() {
            warn!("Send failed {}", status);
        }
        Ok(())
    }

    async fn health_check(&self) -> bool {
        self.state.token_mgr.get().await.is_ok()
    }
}

// ---------------------------------------------------------------------------
// Message conversion
// ---------------------------------------------------------------------------

fn event_to_incoming(event_type: &EventType, data: &serde_json::Value) -> Result<IncomingMessage> {
    let sender_id = data
        .get("author")
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let username = data
        .get("author")
        .and_then(|a| a.get("username"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let (user_id, thread_id) = match event_type {
        EventType::C2cMessageCreate => (
            data.get("openid")
                .and_then(|v| v.as_str())
                .unwrap_or(sender_id)
                .to_string(),
            None,
        ),
        EventType::GroupAtMessageCreate => (
            data.get("group_openid")
                .and_then(|v| v.as_str())
                .unwrap_or(sender_id)
                .to_string(),
            None,
        ),
        EventType::AtMessageCreate | EventType::MessageCreate => {
            let guild = data.get("guild_id").and_then(|v| v.as_str()).unwrap_or("");
            let channel = data.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
            (sender_id.to_string(), Some(format!("{}/{}", guild, channel)))
        }
        EventType::DirectMessageCreate => {
            let guild = data.get("guild_id").and_then(|v| v.as_str()).unwrap_or("");
            (sender_id.to_string(), Some(format!("dm/{}", guild)))
        }
        _ => (sender_id.to_string(), None),
    };

    let clean_content = content
        .split(char::is_control)
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();

    Ok(IncomingMessage {
        platform: "qq_bot".into(),
        user_id,
        username,
        thread_id,
        content: clean_content,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_c2c_event_to_incoming() {
        let data = serde_json::json!({
            "author": { "id": "user-123", "username": "Alice" },
            "content": "Hello bot!",
            "openid": "c2c-openid-abc"
        });
        let msg = event_to_incoming(&EventType::C2cMessageCreate, &data).unwrap();
        assert_eq!(msg.platform, "qq_bot");
        assert_eq!(msg.user_id, "c2c-openid-abc");
        assert_eq!(msg.username, Some("Alice".into()));
        assert_eq!(msg.content, "Hello bot!");
    }

    #[tokio::test]
    async fn test_group_at_event_to_incoming() {
        let data = serde_json::json!({
            "author": { "id": "u1", "username": "Bob" },
            "content": "@Hi there",
            "group_openid": "group-xyz"
        });
        let msg = event_to_incoming(&EventType::GroupAtMessageCreate, &data).unwrap();
        assert_eq!(msg.user_id, "group-xyz");
        assert_eq!(msg.content, "@Hi there");
    }

    #[tokio::test]
    async fn test_at_message_event_to_incoming() {
        let data = serde_json::json!({
            "author": { "id": "u2" },
            "content": "@Bot guild-msg",
            "guild_id": "g1",
            "channel_id": "c1"
        });
        let msg = event_to_incoming(&EventType::AtMessageCreate, &data).unwrap();
        assert_eq!(msg.thread_id, Some("g1/c1".into()));
    }

    #[tokio::test]
    async fn test_content_cleaned_of_control_chars() {
        let data = serde_json::json!({
            "author": { "id": "u3" },
            "content": "Hello\x00World\u{2}!"
        });
        let msg = event_to_incoming(&EventType::C2cMessageCreate, &data).unwrap();
        assert!(!msg.content.contains('\0'));
    }

    #[tokio::test]
    async fn test_empty_content_defaults() {
        let data = serde_json::json!({ "author": {} });
        let msg = event_to_incoming(&EventType::C2cMessageCreate, &data).unwrap();
        assert_eq!(msg.user_id, "unknown");
        assert_eq!(msg.content, "");
    }
}
