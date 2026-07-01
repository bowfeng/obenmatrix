/// Shared platform adapter SDK for messaging services.
///
/// Each platform (Telegram, Discord, Slack, QQ) implements the `PlatformAdapter` trait.
/// This crate contains only the shared platform types so future adapters can depend
/// on it without importing gateway internals.

mod platform;
pub mod common;
pub mod qq_protocol;

pub use platform::*;
