use std::path::PathBuf;

pub use crate::hook_bridge::WasmHookError;

/// Errors specific to WASM plugin execution.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WASM file not found: {0}")]
    WasmNotFound(PathBuf),

    #[error("Platform JSON not found: {0}")]
    PlatformJsonNotFound(PathBuf),

    #[error("Invalid platform JSON: {0}")]
    InvalidPlatformJson(serde_json::Error),

    #[error("WASM compilation error: {0}")]
    Compilation(String),

    #[error("WASM instantiation error: {0}")]
    Instantiation(String),

    #[error("Execute error: {0}")]
    Execute(String),

    #[error("WIT version mismatch: {0}")]
    WitVersionMismatch(String),

    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
}

pub type Result<T> = std::result::Result<T, WasmError>;
