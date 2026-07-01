/// Common utility module for platform-agnostic helpers.
///
/// Shared utilities for messaging platform adapters:
/// - `text_splitter`: Platform-specific text splitting (UTF-8 safe)
/// - `message_normalize`: Mention stripping and slash command parsing
/// - `media_types`: Media type detection by extension and magic bytes
/// - `dedup`: Thread-safe message deduplication cache

pub mod media_types;
pub mod message_normalize;
pub mod text_splitter;

/// Re-export `MediaKind` at the common module level for convenience.
pub use media_types::MediaKind;
