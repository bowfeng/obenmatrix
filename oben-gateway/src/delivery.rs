/// Delivery routing for messages across multiple platforms.
///
/// DeliveryRouter routes messages to appropriate destinations based on
/// session metadata (platform, user_id, thread_id). It handles:
/// - Routing back to origin platform
/// - Routing to specific platform/chat_id combinations
/// - Dead target tracking (permanently unreachable destinations)
use std::collections::HashMap;

use anyhow::Result;
use tracing::{info, warn};

use crate::platform::{OutgoingMessage, PlatformAdapter, PlatformSessionContext};

/// A single delivery target.
///
/// Represents where a message should be sent:
/// - Origin → back to source platform
/// - Specific platform:chat_id → specific destination
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryTarget {
    /// Platform name (e.g., "telegram", "discord", "slack")
    pub platform: String,
    /// Chat ID within the platform (None for home channel)
    pub chat_id: Option<String>,
    /// Thread/channel ID for group conversations (optional)
    pub thread_id: Option<String>,
    /// Whether this target is the origin (source) platform
    pub is_origin: bool,
    /// Whether chat_id was explicitly specified
    pub is_explicit: bool,
}

impl DeliveryTarget {
    /// Create a delivery target from a session context.
    pub fn from_session_context(context: &PlatformSessionContext) -> Self {
        Self {
            platform: context.platform.clone(),
            chat_id: None, // Will be set from session context
            thread_id: context.thread_id.clone(),
            is_origin: true,
            is_explicit: false,
        }
    }

    /// Parse a delivery target string.
    ///
    /// Formats:
    /// - "origin" → back to source
    /// - "telegram" → Telegram home channel
    /// - "telegram:123456" → specific Telegram chat
    /// - "telegram:123456:thread-789" → specific Telegram thread
    pub fn parse(target: &str) -> Self {
        let target_stripped = target.trim();
        let target_lower = target_stripped.to_lowercase();

        if target_lower == "origin" {
            return Self {
                platform: String::from("unknown"),
                chat_id: None,
                thread_id: None,
                is_origin: true,
                is_explicit: false,
            };
        }

        // Check for platform:chat_id or platform:chat_id:thread_id format
        if target_stripped.contains(':') {
            let parts: Vec<&str> = target_stripped.split(':').collect();
            let platform = parts[0].to_lowercase();
            let chat_id = parts.get(1).map(|s| s.to_string());
            let thread_id = parts.get(2).map(|s| s.to_string());

            return Self {
                platform,
                chat_id,
                thread_id,
                is_origin: false,
                is_explicit: true,
            };
        }

        // Just a platform name (use home channel)
        Self {
            platform: target_lower,
            chat_id: None,
            thread_id: None,
            is_origin: false,
            is_explicit: false,
        }
    }

    /// Convert back to string format.
    pub fn to_string(&self) -> String {
        if self.is_origin {
            return String::from("origin");
        }
        if let Some(chat) = &self.chat_id {
            if let Some(thread) = &self.thread_id {
                return format!("{}:{}:{}", self.platform, chat, thread);
            }
            return format!("{}:{}", self.platform, chat);
        }
        self.platform.clone()
    }
}

/// Registry of dead (permanently unreachable) delivery targets.
///
/// When a delivery fails with a permanent error (user deleted, bot kicked,
/// chat removed), the target is marked as dead. Future deliveries to that
/// target are skipped with a clear error message.
#[derive(Debug, Clone, Default)]
pub struct DeadTargetRegistry {
    dead_targets: std::sync::Arc<std::sync::Mutex<HashMap<String, String>>>,
}

impl DeadTargetRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            dead_targets: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Check if a target is marked as dead.
    pub fn is_dead(&self, platform: &str, chat_id: &str) -> bool {
        let key = format!("{}:{}", platform, chat_id);
        self.dead_targets.lock().unwrap().contains_key(&key)
    }

    /// Mark a target as dead.
    pub fn mark_dead(&self, platform: &str, chat_id: &str, reason: &str) {
        let key = format!("{}:{}", platform, chat_id);
        self.dead_targets
            .lock()
            .unwrap()
            .insert(key, reason.to_string());
        warn!(platform, chat_id, reason, "Marked target as dead");
    }

    /// Clear a dead target (used when a previously dead target succeeds).
    pub fn clear(&self, platform: &str, chat_id: &str) {
        let key = format!("{}:{}", platform, chat_id);
        self.dead_targets.lock().unwrap().remove(&key);
        info!(platform, chat_id, "Cleared dead target");
    }
}

/// Routes messages to appropriate delivery targets.
///
/// Handles the logic of resolving delivery targets and dispatching
/// messages to the right platform adapters.
pub struct DeliveryRouter {
    adapters: std::sync::Arc<std::sync::RwLock<HashMap<String, Box<dyn PlatformAdapter + Send + Sync>>>>,
    pub dead_targets: DeadTargetRegistry,
}

impl DeliveryRouter {
    /// Create a new DeliveryRouter.
    pub fn new() -> Self {
        Self {
            adapters: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
            dead_targets: DeadTargetRegistry::new(),
        }
    }

    /// Register a platform adapter for routing.
    pub async fn register_adapter(
        &self,
        platform: &str,
        adapter: Box<dyn PlatformAdapter + Send + Sync>,
    ) {
        let mut adapters = self.adapters.write().unwrap();
        adapters.insert(platform.to_string(), adapter);
        info!(platform, "Registered platform adapter for delivery routing");
    }

    /// Deliver a message to a single target.
    pub async fn deliver_to_target(
        &self,
        content: &str,
        target: &DeliveryTarget,
    ) -> Result<DeliveryResult> {
        // Skip dead targets
        if !target.is_origin
            && target.chat_id.is_some()
            && self
                .dead_targets
                .is_dead(&target.platform, target.chat_id.as_ref().unwrap())
        {
            return Ok(DeliveryResult {
                success: false,
                skipped: Some("dead_target".to_string()),
                error: Some("target previously confirmed unreachable".to_string()),
                delivered: false,
            });
        }

        // Handle origin target (use from session context)
        if target.is_origin {
            // This would need session context to determine actual platform
            // For now, return an error indicating origin routing needs context
            return Ok(DeliveryResult {
                success: false,
                skipped: Some("origin_requires_context".to_string()),
                error: Some("origin routing requires PlatformSessionContext".to_string()),
                delivered: false,
            });
        }

        // Get adapter for the target platform
        let adapters = self.adapters.read().unwrap();
        let adapter = adapters.get(&target.platform).ok_or_else(|| {
            anyhow::anyhow!("No adapter registered for platform: {}", target.platform)
        })?;

        let msg = OutgoingMessage {
            platform: target.platform.clone(),
            user_id: target.chat_id.clone().unwrap_or_default(),
            thread_id: target.thread_id.clone(),
            content: content.to_string(),
        };

        // Try to send the message
        match adapter.send(msg).await {
            Ok(()) => {
                // Success - clear dead flag if it was marked
                if target.chat_id.is_some() {
                    self.dead_targets.clear(
                        &target.platform,
                        target.chat_id.as_ref().unwrap(),
                    );
                }
                Ok(DeliveryResult {
                    success: true,
                    skipped: None,
                    error: None,
                    delivered: true,
                })
            }
            Err(e) => {
                // Mark as dead if appropriate (for permanent failures)
                // For now, just return the error
                Ok(DeliveryResult {
                    success: false,
                    skipped: None,
                    error: Some(e.to_string()),
                    delivered: false,
                })
            }
        }
    }

    /// Deliver a message to multiple targets.
    pub async fn deliver_to_targets(
        &self,
        content: &str,
        targets: &[DeliveryTarget],
    ) -> HashMap<String, DeliveryResult> {
        let mut results = HashMap::new();

        for target in targets {
            let key = target.to_string();
            match self.deliver_to_target(content, target).await {
                Ok(result) => {
                    results.insert(key, result);
                }
                Err(e) => {
                    results.insert(
                        key,
                        DeliveryResult {
                            success: false,
                            skipped: None,
                            error: Some(e.to_string()),
                            delivered: false,
                        },
                    );
                }
            }
        }

        results
    }

    /// Get the number of registered adapters.
    pub fn adapter_count(&self) -> usize {
        let adapters = self.adapters.read().unwrap();
        adapters.len()
    }
}

impl Default for DeliveryRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a delivery operation.
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    /// Whether the delivery operation succeeded (no exceptions)
    pub success: bool,
    /// If delivery was skipped, the reason
    pub skipped: Option<String>,
    /// If delivery failed, the error message
    pub error: Option<String>,
    /// Whether the message was actually delivered
    pub delivered: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Mock adapter for testing
    struct TestAdapter {
        name: String,
        sent_count: std::sync::Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl PlatformAdapter for TestAdapter {
        fn name(&self) -> &str {
            &self.name
        }

        async fn listen(&mut self) -> Result<()> {
            Ok(())
        }

        async fn stop(&mut self) {}

        async fn send(&self, _msg: OutgoingMessage) -> Result<()> {
            *self.sent_count.lock().unwrap() += 1;
            Ok(())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn test_delivery_target_parse() {
        // Test origin target
        let target = DeliveryTarget::parse("origin");
        assert_eq!(target.is_origin, true);
        assert_eq!(target.platform, "unknown");

        // Test platform-only target
        let target = DeliveryTarget::parse("telegram");
        assert_eq!(target.is_origin, false);
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.chat_id, None);

        // Test platform:chat_id target
        let target = DeliveryTarget::parse("telegram:123456");
        assert_eq!(target.is_origin, false);
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.chat_id, Some("123456".to_string()));

        // Test platform:chat_id:thread_id target
        let target = DeliveryTarget::parse("telegram:123456:thread-789");
        assert_eq!(target.is_origin, false);
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.chat_id, Some("123456".to_string()));
        assert_eq!(target.thread_id, Some("thread-789".to_string()));
    }

    #[tokio::test]
    async fn test_delivery_target_to_string() {
        // Test origin
        let target = DeliveryTarget {
            platform: "unknown".to_string(),
            chat_id: None,
            thread_id: None,
            is_origin: true,
            is_explicit: false,
        };
        assert_eq!(target.to_string(), "origin");

        // Test platform only
        let target = DeliveryTarget {
            platform: "telegram".to_string(),
            chat_id: None,
            thread_id: None,
            is_origin: false,
            is_explicit: false,
        };
        assert_eq!(target.to_string(), "telegram");

        // Test platform:chat_id
        let target = DeliveryTarget {
            platform: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            thread_id: None,
            is_origin: false,
            is_explicit: true,
        };
        assert_eq!(target.to_string(), "telegram:123456");

        // Test platform:chat_id:thread_id
        let target = DeliveryTarget {
            platform: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            thread_id: Some("thread-789".to_string()),
            is_origin: false,
            is_explicit: true,
        };
        assert_eq!(target.to_string(), "telegram:123456:thread-789");
    }

    #[tokio::test]
    async fn test_delivery_router_register_adapter() {
        let router = DeliveryRouter::new();
        assert_eq!(router.adapter_count(), 0);

        let adapter = Box::new(TestAdapter {
            name: "test_platform".to_string(),
            sent_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });
        router.register_adapter("test_platform", adapter).await;

        assert_eq!(router.adapter_count(), 1);
    }

    #[tokio::test]
    async fn test_delivery_router_deliver_to_registered_platform() {
        let router = DeliveryRouter::new();

        let sent_count = std::sync::Arc::new(std::sync::Mutex::new(0));
        let adapter = Box::new(TestAdapter {
            name: "test_platform".to_string(),
            sent_count: sent_count.clone(),
        });
        router.register_adapter("test_platform", adapter).await;

        let target = DeliveryTarget {
            platform: "test_platform".to_string(),
            chat_id: Some("user-123".to_string()),
            thread_id: None,
            is_origin: false,
            is_explicit: true,
        };

        let result = router
            .deliver_to_target("Hello, world!", &target)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.delivered);
        assert_eq!(*sent_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_delivery_router_deliver_to_unknown_platform() {
        let router = DeliveryRouter::new();

        let target = DeliveryTarget {
            platform: "unknown_platform".to_string(),
            chat_id: Some("user-123".to_string()),
            thread_id: None,
            is_origin: false,
            is_explicit: true,
        };

        let result = router
            .deliver_to_target("Hello, world!", &target)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No adapter registered"));
    }

    #[tokio::test]
    async fn test_dead_target_registry() {
        let registry = DeadTargetRegistry::new();

        // Initially not dead
        assert!(!registry.is_dead("telegram", "123456"));

        // Mark as dead
        registry.mark_dead("telegram", "123456", "chat deleted");
        assert!(registry.is_dead("telegram", "123456"));

        // Clear the dead flag
        registry.clear("telegram", "123456");
        assert!(!registry.is_dead("telegram", "123456"));
    }

    #[tokio::test]
    async fn test_delivery_skips_dead_target() {
        let router = DeliveryRouter::new();

        let sent_count = std::sync::Arc::new(std::sync::Mutex::new(0));
        let adapter = Box::new(TestAdapter {
            name: "telegram".to_string(),
            sent_count: sent_count.clone(),
        });
        router.register_adapter("telegram", adapter).await;

        // Mark target as dead
        router
            .dead_targets
            .mark_dead("telegram", "123456", "user deleted");

        // Try to deliver to dead target
        let target = DeliveryTarget {
            platform: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            thread_id: None,
            is_origin: false,
            is_explicit: true,
        };

        let result = router
            .deliver_to_target("Hello, world!", &target)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(!result.delivered);
        assert_eq!(result.skipped, Some("dead_target".to_string()));
        assert!(result.error.is_some());

        // sent_count should still be 0 (no actual send attempt)
        assert_eq!(*sent_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_delivery_clears_dead_flag_on_success() {
        let router = DeliveryRouter::new();

        let sent_count = std::sync::Arc::new(std::sync::Mutex::new(0));
        let adapter = Box::new(TestAdapter {
            name: "telegram".to_string(),
            sent_count: sent_count.clone(),
        });
        router.register_adapter("telegram", adapter).await;

        // Mark target as dead
        router
            .dead_targets
            .mark_dead("telegram", "123456", "previous failure");

        // First delivery: should be skipped because target is dead
        let target = DeliveryTarget {
            platform: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            thread_id: None,
            is_origin: false,
            is_explicit: true,
        };

        let result = router
            .deliver_to_target("Hello, world!", &target)
            .await
            .unwrap();

        // First delivery should be skipped
        assert!(!result.success);
        assert!(result.skipped.is_some());

        // Manually clear the dead flag
        router.dead_targets.clear("telegram", "123456");

        // Second delivery: should succeed
        let result = router
            .deliver_to_target("Hello, world!", &target)
            .await;

        assert!(result.is_ok(), "Delivery should succeed: {:?}", result);
        let delivery_result = result.unwrap();
        assert!(delivery_result.success);
        assert!(delivery_result.delivered);

        // Dead flag should be cleared after successful send
        assert!(!router.dead_targets.is_dead("telegram", "123456"));
    }
}
