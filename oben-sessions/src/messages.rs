//! Re-export message types from oben-models.
//!
//! Keeping these accessible from `crate::` so that session.rs and other
//! modules within `oben-sessions` can use `crate::Message`,
//! `crate::MessageRole`, etc. without depending on `oben_models` directly.

pub use oben_models::Message;
pub use oben_models::MessageContent;
pub use oben_models::MessagePart;
pub use oben_models::MessageRole;
