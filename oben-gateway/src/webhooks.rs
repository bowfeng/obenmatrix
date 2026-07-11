//! Gateway webhooks - webhook handling for gateway events
//!
//! WebhookHandler manages webhook registration and event delivery

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Webhook event types
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WebhookEvent {
    /// Platform started
    PlatformStarted { platform: String },
    /// Platform stopped
    PlatformStopped { platform: String },
    /// Platform error
    PlatformError { platform: String, error: String },
    /// Message received
    MessageReceived { platform: String, user_id: String, content: String },
    /// Message sent
    MessageSent { platform: String, user_id: String, content: String },
    /// Health status change
    HealthStatusChange { platform: String, status: String },
}

/// Webhook target configuration
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookConfig {
    /// URL to send webhook to
    pub url: String,
    /// HTTP method (POST, PUT, etc.)
    pub method: String,
    /// Headers to include
    pub headers: HashMap<String, String>,
    /// Timeout in seconds
    pub timeout_seconds: u64,
}

impl WebhookConfig {
    /// Create new webhook config
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            method: "POST".to_string(),
            headers: HashMap::new(),
            timeout_seconds: 30,
        }
    }

    /// Set HTTP method
    pub fn with_method(mut self, method: &str) -> Self {
        self.method = method.to_string();
        self
    }

    /// Add a header
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self::new("http://localhost:8080/webhook")
    }
}

/// Webhook handler - processes and sends webhooks
pub struct WebhookHandler {
    /// Registered webhook URLs
    webhook_urls: Vec<String>,
    /// Configurations
    configs: HashMap<String, WebhookConfig>,
    /// HTTP client
    client: reqwest::Client,
}

impl WebhookHandler {
    /// Create new webhook handler
    pub fn new() -> Self {
        Self {
            webhook_urls: Vec::new(),
            configs: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    /// Register a webhook URL
    pub fn register_url(&mut self, url: &str) {
        self.webhook_urls.push(url.to_string());
    }

    /// Register a webhook with config
    pub fn register_with_config(&mut self, url: &str, config: WebhookConfig) {
        self.webhook_urls.push(url.to_string());
        self.configs.insert(url.to_string(), config);
    }

    /// Send a webhook event
    pub async fn send_event(&self, event: &WebhookEvent) -> Result<()> {
        let event_json = serde_json::to_string(event)?;

        for url in &self.webhook_urls {
            let config = self.configs.get(url);
            let config = self.configs.get(url);

            let mut request = self.client.post(url).body(event_json.clone());

            if let Some(cfg) = config {
                if !cfg.method.is_empty() {
                    // Note: This would need different client setup for PUT/PATCH
                }
                for (key, value) in &cfg.headers {
                    request = request.header(key, value);
                }
            }

            self.client.post(url)
                .body(event_json.clone())
                .send()
                .await?;
        }

        Ok(())
    }

    /// Get registered webhook URLs
    pub fn urls(&self) -> &[String] {
        &self.webhook_urls
    }

    /// Get count of registered webhooks
    pub fn url_count(&self) -> usize {
        self.webhook_urls.len()
    }
}

impl Default for WebhookHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Webhook manager - coordinates webhook operations
pub struct WebhookManager {
    handler: WebhookHandler,
    /// Recent events for debugging
    recent_events: Vec<WebhookEvent>,
    /// Max events to keep
    max_events: usize,
}

impl WebhookManager {
    /// Create new manager
    pub fn new() -> Self {
        Self {
            handler: WebhookHandler::new(),
            recent_events: Vec::new(),
            max_events: 100,
        }
    }

    /// Register a webhook URL
    pub fn register(&mut self, url: &str) {
        self.handler.register_url(url);
    }

    /// Register a webhook with config
    pub fn register_with_config(&mut self, url: &str, config: WebhookConfig) {
        self.handler.register_with_config(url, config);
    }

    /// Record an event (for debugging)
    pub fn record_event(&mut self, event: WebhookEvent) {
        if self.recent_events.len() >= self.max_events {
            self.recent_events.remove(0);
        }
        self.recent_events.push(event);
    }

    /// Get recent events
    pub fn recent_events(&self) -> &[WebhookEvent] {
        &self.recent_events
    }

    /// Send an event to all registered webhooks
    pub async fn send_event(&self, event: &WebhookEvent) -> Result<()> {
        self.handler.send_event(event).await
    }

    /// Get count of registered webhooks
    pub fn webhook_count(&self) -> usize {
        self.handler.url_count()
    }
}

impl Default for WebhookManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_config() {
        let config = WebhookConfig::new("http://example.com/webhook")
            .with_method("POST")
            .with_header("Content-Type", "application/json")
            .with_timeout(60);

        assert_eq!(config.url, "http://example.com/webhook");
        assert_eq!(config.method, "POST");
        assert_eq!(config.headers.get("Content-Type"), Some(&"application/json".to_string()));
        assert_eq!(config.timeout_seconds, 60);
    }

    #[test]
    fn test_webhook_handler() {
        let mut handler = WebhookHandler::new();
        
        handler.register_url("http://example.com/webhook1");
        handler.register_url("http://example.com/webhook2");
        
        assert_eq!(handler.url_count(), 2);
    }

    #[test]
    fn test_webhook_manager() {
        let mut manager = WebhookManager::new();
        
        manager.register("http://example.com/webhook");
        manager.record_event(WebhookEvent::PlatformStarted {
            platform: "telegram".to_string(),
        });
        
        assert_eq!(manager.webhook_count(), 1);
        assert_eq!(manager.recent_events().len(), 1);
    }

    #[test]
    fn test_webhook_event_serialization() {
        let event = WebhookEvent::MessageReceived {
            platform: "telegram".to_string(),
            user_id: "user-123".to_string(),
            content: "hello".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("telegram"));
        assert!(json.contains("user-123"));
    }
}
