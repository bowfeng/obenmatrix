//! WhatsApp platform adapter via Meta Cloud API.
//!
//! Receives messages through HTTP webhooks (port 8000) and sends
//! messages via the Cloud API REST endpoint.

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::platform::{IncomingMessage, OutgoingMessage, PlatformAdapter};
use crate::router::ResponseRouter;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    pub access_token: String,
    pub phone_number_id: String,
    pub business_account_id: String,
    pub webhook_verify_token: String,
    pub api_version: String,
    pub allowed_numbers: Vec<String>,
    pub default_language: String,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------
// HTTP 1.1 status strings (no `http` crate dependency)
const HTTP_OK: &str = "200 OK";
const HTTP_BAD_REQUEST: &str = "400 Bad Request";
const HTTP_FORBIDDEN: &str = "403 Forbidden";
const HTTP_NOT_FOUND: &str = "404 Not Found";

#[derive(Clone)]
struct SharedState {
    dispatcher: Arc<crate::dispatcher::Dispatcher>,
    #[allow(dead_code)]
    response_router: Arc<ResponseRouter>,
    token: String,
    phone_number_id: String,
    #[allow(dead_code)]
    business_account_id: String,
    webhook_verify_token: String,
    api_version: String,
    allowed_numbers: Vec<String>,
    #[allow(dead_code)]
    default_language: String,
    http_server_port: u16,
    server_abort: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    is_started: Arc<AtomicBool>,
    deduplicator: Arc<MessageDeduplicator>,
}

impl SharedState {
    fn new(
        config: WhatsAppConfig,
        dispatcher: Arc<crate::dispatcher::Dispatcher>,
        response_router: Arc<ResponseRouter>,
        deduplicator: Arc<MessageDeduplicator>,
    ) -> Self {
        Self {
            dispatcher,
            response_router,
            token: config.access_token,
            phone_number_id: config.phone_number_id,
            business_account_id: config.business_account_id,
            webhook_verify_token: config.webhook_verify_token,
            api_version: config.api_version,
            allowed_numbers: config.allowed_numbers,
            default_language: config.default_language,
            http_server_port: 8000,
            server_abort: Arc::new(Mutex::new(None)),
            is_started: Arc::new(AtomicBool::new(false)),
            deduplicator,
        }
    }
}

// ---------------------------------------------------------------------------
// Message deduplication
// ---------------------------------------------------------------------------

struct MessageDeduplicator {
    seen: Mutex<serde_json::Map<String, serde_json::Value>>,
}

impl Clone for MessageDeduplicator {
    fn clone(&self) -> Self {
        Self {
            seen: Mutex::new(serde_json::Map::new()),
        }
    }
}

impl MessageDeduplicator {
    fn new() -> Self {
        Self {
            seen: Mutex::new(serde_json::Map::new()),
        }
    }

    async fn try_insert(&self, id: &str) -> bool {
        let mut seen = self.seen.lock().await;
        if seen.contains_key(id) {
            false
        } else {
            seen.insert(id.to_string(), serde_json::Value::Null);
            seen.retain(|_, v| {
                if v.is_null() {
                    return true;
                }
                true
            });
            if seen.len() > 1000 {
                let first = seen.keys().next().cloned();
                if let Some(k) = first {
                    seen.remove(&k);
                }
            }
            true
        }
    }
}

// ---------------------------------------------------------------------------
// WhatsApp adapter
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WhatsAppAdapter {
    state: SharedState,
}

impl WhatsAppAdapter {
    pub fn new(
        config: WhatsAppConfig,
        dispatcher: Arc<crate::dispatcher::Dispatcher>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        let dedup = Arc::new(MessageDeduplicator::new());
        Self {
            state: SharedState::new(config, dispatcher, response_router, dedup),
        }
    }

    async fn spawn_server(&self) -> Result<()> {
        let state = self.state.clone();
        // Extract Arc fields before moving state into the spawn closure
        let server_abort = Arc::clone(&state.server_abort);
        let is_started = Arc::clone(&state.is_started);

        let addr: SocketAddr = ([127, 0, 0, 1], state.http_server_port).into();
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| "Failed to bind WhatsApp webhook listener")?;

        info!(port = state.http_server_port, "WhatsApp webhook server listening");

        let server = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let st = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_tcp(st, stream).await {
                                debug!("Connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        debug!("Accept error: {}", e);
                    }
                }
            }
        });

        let abort_handle = server.abort_handle();
        {
            let mut guard = server_abort.lock().await;
            *guard = Some(abort_handle);
        }
        is_started.store(true, Ordering::Release);

        info!("WhatsApp webhook server started");
        Ok(())
    }

    async fn send_message(&self, phone_number: &str, content: &str, reply_to: Option<&str>) -> Result<()> {
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }

        let formatted = trancate_content(content, 4096);

        let client = reqwest::Client::new();
        let endpoint = format!(
            "https://graph.facebook.com/{}/{}",
            self.state.api_version,
            self.state.phone_number_id
        );

        let mut payload = serde_json::Map::new();
        payload.insert("messaging_product".into(), "whatsapp".into());
        payload.insert("recipient_type".into(), "individual".into());
        payload.insert("to".into(), phone_number.into());
        payload.insert("type".into(), "text".into());

        let mut text_map = serde_json::Map::new();
        if let Some(_reply_to) = reply_to {
            let reply_key = serde_json::Map::new();
            text_map.insert("preview_url".into(), false.into());
            text_map.insert("reply".into(), serde_json::to_value(&reply_key).unwrap_or_default());
        }
        text_map.insert("body".into(), formatted.into());
        payload.insert("text".into(), serde_json::Value::Object(text_map));

        let resp = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", self.state.token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .with_context(|| "Failed to send WhatsApp message via Cloud API")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            error!(status = %status, "WhatsApp send failed: {}", body);
            bail!("WhatsApp send failed {}: {}", status, body);
        }

        info!(phone_number = %phone_number, "WhatsApp message sent successfully");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PlatformAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformAdapter for WhatsAppAdapter {
    fn name(&self) -> &str {
        "whatsapp"
    }

    async fn listen(&mut self) -> Result<()> {
        info!("WhatsApp adapter starting webhook server");

        let handle = self.state.server_abort.lock().await;
        if !self.state.is_started.load(Ordering::Acquire) {
            drop(handle);
            if let Err(e) = self.spawn_server().await {
                error!("Failed to start webhook server: {}", e);
                bail!("Failed to start webhook server: {}", e);
            }
        }

        tokio::signal::ctrl_c().await?;
        info!("Ctrl+C received — shutting down WhatsApp adapter");
        Ok(())
    }

    async fn stop(&mut self) {
        let handle = {
            let mut guard = self.state.server_abort.lock().await;
            guard.take()
        };
        if let Some(handle) = handle {
            handle.abort();
            info!("WhatsApp server task aborted");
        }
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let content = &msg.content;
        if content.is_empty() {
            return Err(anyhow!("Cannot send empty message"));
        }

        let phone = normalize_phone_number(&msg.user_id);

        if let Err(e) = self.send_message(&phone, content, None).await {
            error!("Send failed: {}", e);
            return Err(e);
        }
        Ok(())
    }

    async fn health_check(&self) -> bool {
        !self.state.token.is_empty() && self.state.is_started.load(Ordering::Acquire)
    }
}

// ---------------------------------------------------------------------------
// TCP webhook handler (raw HTTP, no hyper)
// ---------------------------------------------------------------------------

async fn handle_tcp(state: SharedState, mut stream: tokio::net::TcpStream) -> Result<()> {
    // Read complete HTTP request into a buffer
    let mut raw = Vec::with_capacity(4096);
    let mut read_buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut read_buf).await?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&read_buf[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            raw.truncate(min(pos + 4, raw.len()));
            break;
        }
        if raw.len() > 65536 {
            send_response(&mut stream, HTTP_BAD_REQUEST, "text/plain", "Request too large");
            return Ok(());
        }
    }

    // Split into headers and body
    let text = String::from_utf8_lossy(&raw);
    let mut parts = text.trim_end().splitn(2, "\r\n\r\n");
    let header_section = parts.next().unwrap_or("");
    let body_str = parts.next().unwrap_or("");

    // Parse request line
    let request_line = header_section.lines().next().unwrap_or("");
    let req_parts: Vec<&str> = request_line.split_whitespace().collect();
    if req_parts.len() < 2 {
        send_response(&mut stream, HTTP_BAD_REQUEST, "text/plain", "Bad Request");
        return Ok(());
    }
    let parts_vec: Vec<&str> = request_line.split_whitespace().collect();
    let mut it = parts_vec.iter();
    let method = it.next().copied().unwrap_or_default();
    let full_path = it.next().copied().unwrap_or_default();
    let (path, query) = match full_path.find('?') {
        Some(i) => (full_path[..i].to_string(), full_path[i + 1..].to_string()),
        None => (full_path.to_string(), String::new()),
    };

    // Parse headers
    let header_map: std::collections::HashMap<String, String> = header_section
        .lines()
        .skip(1)
        .filter_map(|h| {
            let mut iter = h.splitn(2, ':');
            let key = iter.next()?.trim().to_lowercase();
            let value = iter.next()?.trim().to_string();
            Some((key, value))
        })
        .collect();

    let declared_len: usize = header_map.get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // Ensure we have the full body
    let final_body = if declared_len > 0 && body_str.len() < declared_len {
        let mut remaining = declared_len - body_str.len();
        let mut ext = body_str.as_bytes().to_vec();
        while remaining > 0 {
            let n = stream.read(&mut read_buf).await?;
            if n == 0 {
                break;
            }
            let take = n.min(remaining);
            ext.extend_from_slice(&read_buf[..take]);
            remaining -= take;
        }
        String::from_utf8_lossy(&ext[..ext.len().min(declared_len)]).into_owned()
    } else {
        body_str.to_string()
    };

    // Route the request
    if method == "GET" && path == "/webhook/whatsapp" {
        let resp = verify_challenge_response(&state, &query);
        send_response(&mut stream, &resp.0, "text/plain", &resp.1);
    } else if method == "POST" && path == "/webhook/whatsapp" {
        match handle_post(&state, deduplicator_field(&state), &final_body).await {
            Ok(_) => send_response(&mut stream, HTTP_OK, "text/plain", "OK"),
            Err(e) => send_response(&mut stream, HTTP_BAD_REQUEST, "text/plain", &e.to_string()),
        }
    } else {
        send_response(&mut stream, HTTP_NOT_FOUND, "text/plain", "Not Found");
    }

    Ok(())
}

fn send_response<W: AsyncWriteExt + Unpin>(stream: &mut W, status: &str, ct: &str, body: &str) {
    let body_len = body.as_bytes().len();
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status, ct, body_len, body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn verify_challenge_response(state: &SharedState, query: &str) -> (String, String) {
    let params = parse_url_encoded(query);
    let mode = params.get("hub.mode");
    let token = params.get("hub.verify_token");
    let challenge = params.get("hub.challenge");

    if mode != Some(&"subscribe".to_string()) {
        return (HTTP_BAD_REQUEST.to_string(), "Forbidden".to_string());
    }
    if token != Some(&state.webhook_verify_token) {
        return (HTTP_FORBIDDEN.to_string(), "Wrong verify token".to_string());
    }
    let empty = String::new();
    let c = challenge.unwrap_or(&empty);
    (HTTP_OK.to_string(), c.clone())
}

fn extract_message_content(message_type: &str, msg: &serde_json::Value, attachments: &mut Vec<(String, String)>) -> Vec<String> {
    let mut contents = vec![];

    match message_type {
        "text" => {
            if let Some(text_obj) = msg.get("text") {
                if let Some(body) = text_obj.get("body").and_then(|b| b.as_str()) {
                    contents.push(body.to_string());
                }
            }
        }
        "image" => {
            let mut has_caption = false;
            if let Some(caption) = msg.get("image").and_then(|img| img.get("caption")).and_then(|c| c.as_str()) {
                contents.push(caption.to_string());
                has_caption = true;
            };
            if let Some(id) = msg.get("image").and_then(|img| img.get("id")).and_then(|i| i.as_str()) {
                attachments.push(("image".to_string(), id.to_string()));
            }
            if !has_caption {
                attachments.push(("image".to_string(), msg.get("image").and_then(|img| img.get("id")).and_then(|s| s.as_str()).unwrap_or("<unknown>").to_string()));
            }
        }
        "document" => {
            let mut has_caption = false;
            if let Some(caption) = msg.get("document").and_then(|doc| doc.get("caption")).and_then(|c| c.as_str()) {
                contents.push(caption.to_string());
                has_caption = true;
            }
            if let Some(id) = msg.get("document").and_then(|doc| doc.get("id")).and_then(|i| i.as_str()) {
                attachments.push(("document".to_string(), id.to_string()));
            }
            if !has_caption {
                attachments.push(("document".to_string(), msg.get("document").and_then(|doc| doc.get("id")).and_then(|s| s.as_str()).unwrap_or("<unknown>").to_string()));
            }
        }
        "audio" => {
            if let Some(id) = msg.get("audio").and_then(|a| a.get("id")).and_then(|i| i.as_str()) {
                attachments.push(("audio".to_string(), id.to_string()));
            }
        }
        "video" => {
            if let Some(id) = msg.get("video").and_then(|v| v.get("id")).and_then(|i| i.as_str()) {
                attachments.push(("video".to_string(), id.to_string()));
            }
        }
        _ => {}
    }

    contents
}

async fn handle_post(state: &SharedState, dedup: Arc<MessageDeduplicator>, body: &str) -> Result<()> {
    let webhook: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse webhook payload: {}", e);
            bail!("Invalid JSON");
        }
    };

    let entries = webhook.get("entry").and_then(|e| e.as_array()).ok_or_else(|| anyhow!("No 'entry' in webhook"))?;
    for entry in entries {
        let changes = entry.get("changes").and_then(|c| c.as_array()).ok_or_else(|| anyhow!("No 'changes' in entry"))?;
        for change in changes {
            let value = change.get("value").ok_or_else(|| anyhow!("Missing 'value' in change"))?;

            let mut contacts_arr = Vec::new();
            if let Some(arr) = value.get("contacts").and_then(|c| c.as_array()) {
                contacts_arr = arr.to_vec();
            }
            if contacts_arr.is_empty() {
                continue;
            }

            let contact = &contacts_arr[0];
            let wa_id = contact.get("wa_id").and_then(|w| w.as_str()).unwrap_or("unknown");
            let username = contact.get("name").and_then(|n| n.as_str()).map(|s| s.to_string());

            let mut messages_arr = Vec::new();
            if let Some(arr) = value.get("messages").and_then(|m| m.as_array()) {
                messages_arr = arr.to_vec();
            }
            if messages_arr.is_empty() {
                continue;
            }

            let msg = &messages_arr[0];
            let message_id = msg.get("id").and_then(|id| id.as_str()).unwrap_or("").to_string();
            let message_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("text");

            let mut attachments = vec![];

            for content in extract_message_content(message_type, msg, &mut attachments) {
                if content.is_empty() {
                    continue;
                }

                if !dedup.try_insert(&message_id).await {
                    debug!("Dedup: ignoring duplicate message {}", message_id);
                    continue;
                }

                let allowed = &state.allowed_numbers;
                if !allowed.is_empty() && !allowed.iter().any(|n| wa_id.contains(n.as_str())) {
                    debug!("Filtering message from disallowed number: {}", wa_id);
                    break;
                }

                let incoming_msg = IncomingMessage {
                    platform: "whatsapp".into(),
                    user_id: wa_id.to_string(),
                    username,
                    content: content.clone(),
                    thread_id: None,
                };

                if let Err(e) = state.dispatcher.dispatch(incoming_msg).await {
                    error!("Dispatcher error: {}", e);
                }
                break;
            }
        }
    }

    Ok(())
}

// Helper: state.deduplicator access (deduplicator is in SharedState now)
fn deduplicator_field(state: &SharedState) -> Arc<MessageDeduplicator> {
    Arc::clone(&state.deduplicator)
}

fn min(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_url_encoded(query: &str) -> std::collections::HashMap<String, String> {
    let mut params = std::collections::HashMap::new();
    if query.is_empty() {
        return params;
    }
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = url_decode(kv.next().unwrap_or(""));
        let value = url_decode(kv.next().unwrap_or(""));
        params.insert(key, value);
    }
    params
}

fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                result.push(val);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            result.push(b' ');
            i += 1;
            continue;
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn normalize_phone_number(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.starts_with('+') {
        cleaned[1..].to_string()
    } else if cleaned.len() >= 10 {
        cleaned
    } else {
        raw.to_string()
    }
}

fn trancate_content(content: &str, max_len: usize) -> String {
    let chars: String = content.chars().take(max_len).collect();
    chars
}

// ---------------------------------------------------------------------------
// WhatsAppPlatformFactory for PlatformFactory trait
// ---------------------------------------------------------------------------

/// Factory for the WhatsApp platform (Meta Cloud API).
pub struct WhatsAppPlatformFactory {
    config: std::sync::Arc<oben_config::WhatsAppConfig>,
    dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
    response_router: std::sync::Arc<crate::router::ResponseRouter>,
}

impl WhatsAppPlatformFactory {
    pub fn new(
        config: oben_config::WhatsAppConfig,
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
            let wa_config = WhatsAppConfig {
                access_token: c.access_token.unwrap_or_default(),
                phone_number_id: c.phone_number_id.unwrap_or_default(),
                business_account_id: c.business_account_id.unwrap_or_default(),
                webhook_verify_token: c.webhook_verify_token.unwrap_or_default(),
                api_version: c.api_version,
                allowed_numbers: c.allowed_numbers,
                default_language: c.default_language,
            };
            let mut adapter = crate::whatsapp_adapter::WhatsAppAdapter::new(
                wa_config,
                dispatcher,
                response_router.clone(),
            );

            response_router.register("whatsapp", Box::new(adapter.clone())).await;

            if let Err(e) = adapter.listen().await {
                error!("WhatsApp adapter crashed: {e}");
            }
        })
        .abort_handle()
    }
}

impl crate::platform::PlatformFactory for WhatsAppPlatformFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        WhatsAppPlatformFactory::spawn(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_truncate_ascii() {
        let result = trancate_content("Hello, world! This is a very long message that should be truncated", 20);
        assert_eq!(result, "Hello, world! This i");
    }

    #[test]
    fn test_content_truncate_utf8() {
        let input = "こんにちは世界";
        let result = trancate_content(input, 3);
        assert_eq!(result, "こんにちは");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_normalize_phone_with_plus() {
        let result = normalize_phone_number("+1234567890");
        assert_eq!(result, "1234567890");
    }

    #[test]
    fn test_normalize_phone_without_plus() {
        let result = normalize_phone_number("1234567890");
        assert_eq!(result, "1234567890");
    }

    #[test]
    fn test_normalize_phone_with_spaces() {
        let result = normalize_phone_number(" +1 234 567 890 ");
        assert_eq!(result, "1234567890");
    }

    #[tokio::test]
    async fn test_deduplicator_first_insert() {
        let dedup = MessageDeduplicator::new();
        assert!(dedup.try_insert("msg-001").await);
    }

    #[tokio::test]
    async fn test_deduplicator_duplicate_rejected() {
        let dedup = MessageDeduplicator::new();
        assert!(dedup.try_insert("msg-001").await);
        assert!(!dedup.try_insert("msg-001").await);
    }

    #[tokio::test]
    async fn test_deduplicator_different_ids() {
        let dedup = MessageDeduplicator::new();
        assert!(dedup.try_insert("msg-001").await);
        assert!(dedup.try_insert("msg-002").await);
        assert!(dedup.try_insert("msg-003").await);
    }

    #[test]
    fn test_parse_text_message() {
        let msg = serde_json::json!({
            "from": "628123456789",
            "id": "wamid.HBgLMTIzNDU2Nzg5AAI=",
            "timestamp": "1673944740",
            "text": { "body": "Hello bot" },
            "type": "text"
        });

        let mut attachments = vec![];
        let contents = extract_message_content("text", &msg, &mut attachments);
        assert_eq!(contents, vec!["Hello bot"]);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_image_message_extract_caption_and_attachment() {
        let msg = serde_json::json!({
            "from": "628123456789",
            "id": "wamid.HBgLMTIzNDU2Nzg5AAI=",
            "timestamp": "1673944740",
            "image": {
                "id": "media-id-123",
                "caption": "Look at this"
            },
            "type": "image"
        });

        let mut attachments = vec![];
        let contents = extract_message_content("image", &msg, &mut attachments);
        assert_eq!(contents, vec!["Look at this"]);
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_document_message_without_caption() {
        let msg = serde_json::json!({
            "from": "628123456789",
            "id": "wamid.HBgLMTIzNDU2Nzg5AAI=",
            "timestamp": "1673944740",
            "document": {
                "id": "media-id-456",
                "filename": "report.pdf"
            },
            "type": "document"
        });

        let mut attachments = vec![];
        let contents = extract_message_content("document", &msg, &mut attachments);
        assert!(contents.is_empty());
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].0, "document");
    }

    #[test]
    fn test_audio_message_has_no_content() {
        let msg = serde_json::json!({
            "from": "628123456789",
            "id": "wamid.HBgLMTIzNDU2Nzg5AAI=",
            "timestamp": "1673944740",
            "audio": {
                "id": "media-id-audio"
            },
            "type": "audio"
        });

        let mut attachments = vec![];
        let contents = extract_message_content("audio", &msg, &mut attachments);
        assert!(contents.is_empty());
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].0, "audio");
    }

    #[test]
    fn test_empty_text_body_produces_no_content() {
        let msg = serde_json::json!({
            "type": "text",
            "text": { "body": "" }
        });

        let mut attachments = vec![];
        let contents = extract_message_content("text", &msg, &mut attachments);
        assert!(contents.is_empty());
    }

    #[test]
    fn test_whatsapp_config_default_fields() {
        let config = WhatsAppConfig {
            access_token: "test-token".into(),
            phone_number_id: "12345".into(),
            business_account_id: "67890".into(),
            webhook_verify_token: "my-secret".into(),
            api_version: "v17.0".into(),
            allowed_numbers: vec![],
            default_language: "en_US".into(),
        };
        assert_eq!(config.access_token, "test-token");
        assert_eq!(config.api_version, "v17.0");
        assert_eq!(config.default_language, "en_US");
    }

    #[test]
    fn test_adapter_name() {
        let mut adapter = WhatsAppAdapter::new(
            WhatsAppConfig {
                access_token: "t".into(),
                phone_number_id: "1".into(),
                business_account_id: "2".into(),
                webhook_verify_token: "v".into(),
                api_version: "v17.0".into(),
                allowed_numbers: vec![],
                default_language: "en_US".into(),
            },
            Arc::new(crate::dispatcher::Dispatcher::default()),
            Arc::new(crate::router::ResponseRouter::new()),
        );
        assert_eq!(adapter.name(), "whatsapp");
    }

    #[test]
    fn test_url_decode_basic() {
        assert_eq!(url_decode("hello"), "hello");
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("%E4%B8%AD%E6%96%87"), "%E4%B8%AD%E6%96%87"); // invalid hex, returned as-is
    }

    #[test]
    fn test_parse_url_encoded_empty() {
        let params = parse_url_encoded("");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_url_encoded_single_pair() {
        let params = parse_url_encoded("hub.mode=subscribe&hub.verify_token=test&hub.challenge=12345");
        assert_eq!(params.get("hub.mode"), Some(&"subscribe".to_string()));
        assert_eq!(params.get("hub.verify_token"), Some(&"test".to_string()));
        assert_eq!(params.get("hub.challenge"), Some(&"12345".to_string()));
    }

    #[test]
    fn test_parse_url_encoded_multiple_pairs() {
        let params = parse_url_encoded("a=1&b=2&c=3");
        assert_eq!(params.len(), 3);
        assert_eq!(params.get("a"), Some(&"1".to_string()));
        assert_eq!(params.get("c"), Some(&"3".to_string()));
    }
}
