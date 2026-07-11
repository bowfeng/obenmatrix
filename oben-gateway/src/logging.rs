//! Gateway logging - structured event logging
//!
//! GatewayLogger handles structured logging for gateway events

use serde::{Deserialize, Serialize};
use tracing::{event, Level};

/// Gateway event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GatewayEvent {
    /// Platform started
    PlatformStarted { platform: String },
    /// Platform stopped
    PlatformStopped { platform: String },
    /// Platform error
    PlatformError { platform: String, error: String },
    /// Message received
    MessageReceived { platform: String, user_id: String, content_length: usize },
    /// Message sent
    MessageSent { platform: String, user_id: String, content_length: usize },
    /// Message failed
    MessageFailed { platform: String, user_id: String, reason: String },
    /// Health check passed
    HealthCheckPassed { platform: String },
    /// Health check failed
    HealthCheckFailed { platform: String, error: String },
    /// State saved
    StateSaved { path: String },
    /// State loaded
    StateLoaded { path: String },
}

/// Gateway event logger
pub struct GatewayEventLogger {
    enabled: bool,
}

impl GatewayEventLogger {
    /// Create new logger
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Create logger with specified enabled state
    pub fn with_enabled(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Log a gateway event
    pub fn log(&self, event: &GatewayEvent) {
        if !self.enabled {
            return;
        }

        match event {
            GatewayEvent::PlatformStarted { platform } => {
                event!(Level::INFO, "platform_started {}", platform);
            }
            GatewayEvent::PlatformStopped { platform } => {
                event!(Level::INFO, "platform_stopped {}", platform);
            }
            GatewayEvent::PlatformError { platform, error } => {
                event!(Level::ERROR, "platform_error {} {}", platform, error);
            }
            GatewayEvent::MessageReceived { platform, user_id, content_length } => {
                event!(Level::DEBUG, "message_received {} {} {}", platform, user_id, content_length);
            }
            GatewayEvent::MessageSent { platform, user_id, content_length } => {
                event!(Level::DEBUG, "message_sent {} {}", platform, user_id);
            }
            GatewayEvent::MessageFailed { platform, user_id, reason } => {
                event!(Level::WARN, "message_failed {} {} {}", platform, user_id, reason);
            }
            GatewayEvent::HealthCheckPassed { platform } => {
                event!(Level::INFO, "health_check_passed {}", platform);
            }
            GatewayEvent::HealthCheckFailed { platform, error } => {
                event!(Level::WARN, "health_check_failed {} {}", platform, error);
            }
            GatewayEvent::StateSaved { path } => {
                event!(Level::INFO, "state_saved {}", path);
            }
            GatewayEvent::StateLoaded { path } => {
                event!(Level::INFO, "state_loaded {}", path);
            }
        }
    }

    /// Log a platform starting
    pub fn platform_started(&self, platform: &str) {
        self.log(&GatewayEvent::PlatformStarted {
            platform: platform.to_string(),
        });
    }

    /// Log a platform stopping
    pub fn platform_stopped(&self, platform: &str) {
        self.log(&GatewayEvent::PlatformStopped {
            platform: platform.to_string(),
        });
    }

    /// Log a platform error
    pub fn platform_error(&self, platform: &str, error: &str) {
        self.log(&GatewayEvent::PlatformError {
            platform: platform.to_string(),
            error: error.to_string(),
        });
    }

    /// Log a message received
    pub fn message_received(&self, platform: &str, user_id: &str, content_length: usize) {
        self.log(&GatewayEvent::MessageReceived {
            platform: platform.to_string(),
            user_id: user_id.to_string(),
            content_length,
        });
    }

    /// Log a message sent
    pub fn message_sent(&self, platform: &str, user_id: &str, content_length: usize) {
        self.log(&GatewayEvent::MessageSent {
            platform: platform.to_string(),
            user_id: user_id.to_string(),
            content_length,
        });
    }

    /// Log a message failure
    pub fn message_failed(&self, platform: &str, user_id: &str, reason: &str) {
        self.log(&GatewayEvent::MessageFailed {
            platform: platform.to_string(),
            user_id: user_id.to_string(),
            reason: reason.to_string(),
        });
    }
}

impl Default for GatewayEventLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// Event tracking with history
pub struct EventTracker {
    events: Vec<GatewayEvent>,
    max_events: usize,
}

impl EventTracker {
    /// Create new tracker with default capacity
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            max_events: 1000,
        }
    }

    /// Create new tracker with specified capacity
    pub fn with_capacity(max_events: usize) -> Self {
        Self {
            events: Vec::new(),
            max_events,
        }
    }

    /// Record an event
    pub fn record(&mut self, event: GatewayEvent) {
        if self.events.len() >= self.max_events {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// Get all events
    pub fn events(&self) -> &[GatewayEvent] {
        &self.events
    }

    /// Get events for a specific platform
    pub fn platform_events(&self, platform: &str) -> Vec<&GatewayEvent> {
        self.events
            .iter()
            .filter(|e| match e {
                GatewayEvent::PlatformStarted { platform: p } if p == platform => true,
                GatewayEvent::PlatformStopped { platform: p } if p == platform => true,
                GatewayEvent::PlatformError { platform: p, .. } if p == platform => true,
                GatewayEvent::MessageReceived { platform: p, .. } if p == platform => true,
                GatewayEvent::MessageSent { platform: p, .. } if p == platform => true,
                GatewayEvent::MessageFailed { platform: p, .. } if p == platform => true,
                GatewayEvent::HealthCheckPassed { platform: p } if p == platform => true,
                GatewayEvent::HealthCheckFailed { platform: p, .. } if p == platform => true,
                _ => false,
            })
            .collect()
    }

    /// Count events of a specific type
    pub fn event_count(&self, event_type: &str) -> usize {
        self.events.iter().filter(|e| {
            let name = match e {
                GatewayEvent::PlatformStarted { .. } => "platform_started",
                GatewayEvent::PlatformStopped { .. } => "platform_stopped",
                GatewayEvent::PlatformError { .. } => "platform_error",
                GatewayEvent::MessageReceived { .. } => "message_received",
                GatewayEvent::MessageSent { .. } => "message_sent",
                GatewayEvent::MessageFailed { .. } => "message_failed",
                GatewayEvent::HealthCheckPassed { .. } => "health_check_passed",
                GatewayEvent::HealthCheckFailed { .. } => "health_check_failed",
                GatewayEvent::StateSaved { .. } => "state_saved",
                GatewayEvent::StateLoaded { .. } => "state_loaded",
            };
            name == event_type
        }).count()
    }
}

impl Default for EventTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_event_logger() {
        let logger = GatewayEventLogger::new();
        
        // Should not panic
        logger.platform_started("telegram");
        logger.message_received("telegram", "user-1", 100);
        logger.message_sent("telegram", "user-1", 50);
        logger.message_failed("telegram", "user-1", "timeout");
    }

    #[test]
    fn test_event_tracker() {
        let mut tracker = EventTracker::new();
        
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "telegram".to_string(),
        });
        tracker.record(GatewayEvent::PlatformError {
            platform: "discord".to_string(),
            error: "auth failed".to_string(),
        });
        
        assert_eq!(tracker.events().len(), 2);
        
        let telegram_events = tracker.platform_events("telegram");
        assert_eq!(telegram_events.len(), 1);
        
        let discord_events = tracker.platform_events("discord");
        assert_eq!(discord_events.len(), 1);
    }

    #[test]
    fn test_event_tracker_capacity() {
        let mut tracker = EventTracker::with_capacity(3);
        
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "platform-1".to_string(),
        });
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "platform-2".to_string(),
        });
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "platform-3".to_string(),
        });
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "platform-4".to_string(),
        });
        
        // Should have kept only the last 3
        assert_eq!(tracker.events().len(), 3);
    }

    #[test]
    fn test_event_count() {
        let mut tracker = EventTracker::new();
        
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "telegram".to_string(),
        });
        tracker.record(GatewayEvent::PlatformStarted {
            platform: "discord".to_string(),
        });
        tracker.record(GatewayEvent::PlatformError {
            platform: "telegram".to_string(),
            error: "error".to_string(),
        });
        
        assert_eq!(tracker.event_count("platform_started"), 2);
        assert_eq!(tracker.event_count("platform_error"), 1);
        assert_eq!(tracker.event_count("message_received"), 0);
    }
}
