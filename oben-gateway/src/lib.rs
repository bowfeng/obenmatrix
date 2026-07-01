//! Messaging gateway — serve conversations from multiple platforms.
//!
//! Maps to `gateway/` directory in Hermes.

pub mod coordinator;
pub mod dispatcher;
pub mod gateway;
pub mod platform;
pub mod qq_bot;
pub mod qq_onboard;
pub mod router;

// Re-export shared platform SDK types into gateway's public API
pub use oben_platform_sdk::*;

pub use coordinator::{GatewayCoordinator, ResponseMessage};
pub use dispatcher::Dispatcher;
pub use gateway::Gateway;
pub use platform::*; // QQBotFactory, PlatformFactory, PlatformHandle, PlatformRegistry
pub use qq_bot::*;
pub use qq_onboard::*;
pub use router::ResponseRouter;
