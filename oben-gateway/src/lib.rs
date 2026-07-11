//! Messaging gateway — serve conversations from multiple platforms.
//!
//! Maps to `gateway/` directory in Hermes.

pub mod alerting;
pub mod circuit_breaker;
pub mod coordinator;
pub mod delivery;
pub mod diagnostics;
pub mod dispatcher;
pub mod fallback;
pub mod gateway;
pub mod health_check;
pub mod logging;
pub mod memory_monitor;
pub mod metrics;
pub mod pairing;
pub mod platform;
pub mod qq_bot;
pub mod qq_onboard;
pub mod rate_limiter;
pub mod router;
pub mod slash_commands;
pub mod state_persistence;
pub mod sticker_cache;
pub mod webhooks;

#[cfg(feature = "discord")]
pub mod discord_adapter;

#[cfg(feature = "telegram")]
pub mod telegram_adapter;

#[cfg(feature = "whatsapp")]
pub mod whatsapp_adapter;

#[cfg(feature = "slack")]
pub mod slack_adapter;

// Re-export shared platform SDK types into gateway's public API
pub use oben_platform_sdk::*;

pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerManager, CircuitState};
pub use coordinator::{GatewayCoordinator, ResponseMessage};
pub use delivery::{DeliveryResult, DeliveryRouter, DeliveryTarget, DeadTargetRegistry};
pub use diagnostics::{DiagnosticProvider, GatewayDiagnostic, PlatformDiagnostic};
pub use dispatcher::Dispatcher;
pub use fallback::{FallbackManager, FallbackResult, FallbackStrategy};
pub use gateway::{Gateway, RetryConfig, RetryExecutor, RetryResult};
pub use health_check::{HealthChecker, HealthCheckHandler, HealthCheckResult, HealthCheckSummary, HealthStatus};
pub use alerting::{Alert, AlertConfig, AlertManager, AlertSeverity, AlertType};
pub use logging::{EventTracker, GatewayEvent, GatewayEventLogger};
pub use memory_monitor::{is_running, log_memory_usage, start_memory_monitoring, stop_memory_monitoring};
pub use metrics::{GatewayMetrics, MetricsRecorder, PlatformMetrics};
pub use pairing::{ApprovedUser, PairingManager, PendingEntryInfo};
pub use platform::*; // PlatformAdapterRegistry, PlatformEntry, QQBotFactory, PlatformFactory, PlatformHandle, PlatformRegistry
pub use qq_bot::*;
pub use qq_onboard::*;
pub use rate_limiter::{RateLimitAlgorithm, RateLimitConfig, RateLimitManager, RateLimiter, RateLimitState};
pub use router::ResponseRouter;
pub use slash_commands::{
    HelpHandler, PauseHandler, ResetHandler, ResumeHandler, SlashCommandHandler, SlashCommandRouter, StatusHandler,
};
pub use state_persistence::{GatewayState, GatewayStatePersister};
pub use sticker_cache::{StickerCache, StickerDescription};
pub use webhooks::{WebhookEvent, WebhookHandler, WebhookManager};

#[cfg(feature = "discord")]
pub use discord_adapter::*;

#[cfg(feature = "telegram")]
pub use telegram_adapter::*;

#[cfg(feature = "whatsapp")]
pub use whatsapp_adapter::*;

#[cfg(feature = "slack")]
pub use slack_adapter::*;
