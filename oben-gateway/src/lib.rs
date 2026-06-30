//! Messaging gateway — serve conversations from multiple platforms.
//!
//! Maps to `gateway/` directory in Hermes.

pub mod coordinator;
pub mod dispatcher;
pub mod gateway;
pub mod platform;
pub mod qq_bot;
pub mod qq_onboard;
pub mod qq_protocol;
pub mod router;

pub use coordinator::{GatewayCoordinator, ResponseMessage};
pub use dispatcher::Dispatcher;
pub use gateway::Gateway;
pub use platform::*;
pub use qq_bot::*;
pub use qq_onboard::*;
pub use qq_protocol::*;
pub use router::ResponseRouter;
