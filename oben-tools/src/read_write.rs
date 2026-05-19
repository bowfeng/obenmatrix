/// File read/write tools.
///
/// Self-registers via `SelfRegisteringTool` trait.

use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;
use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool};
use oben_utils::path_security::is_path_safe;

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_read_file_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "path".into(),
            description: "Path to the file".into(),
            parameter_type: "string".into(),
            required: true,
        },
    ];
    Tool {
        name: "read_file".into(),
        description: "Read the contents of a file".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_write_file_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "path".into(),
            description: "Path to write to".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "content".into(),
            description: "Content to write".into(),
            parameter_type: "string".into(),
            required: true,
        },
    ];
    Tool {
        name: "write_file".into(),
        description: "Write content to a file".into(),
        parameters: ToolParameters::Flat(params),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn make_read_file_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let path = args.get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

            if !is_path_safe(std::path::Path::new(path)) {
                return Ok(ToolResult {
                    call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    output: String::new(),
                    error: Some("Unsafe file path".to_string()),
                });
            }

            let content = tokio::fs::read_to_string(path).await.map_err(|e| {
                anyhow::anyhow!("Failed to read {}: {}", path, e)
            })?;

            let content = if content.len() > 100_000 {
                format!("{}... (truncated, {} chars total)", &content[..100_000], content.len())
            } else {
                content
            };

            Ok(ToolResult {
                call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                output: content,
                error: None,
            })
        })
    })
}

fn make_write_file_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let path = args.get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

            let content = args.get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

            if !is_path_safe(std::path::Path::new(path)) {
                return Ok(ToolResult {
                    call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    output: String::new(),
                    error: Some("Unsafe file path".to_string()),
                });
            }

            let path_buf = PathBuf::from(path);
            let dir = path_buf.parent().unwrap_or(std::path::Path::new("."));
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(path, content).await?;

            Ok(ToolResult {
                call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                output: format!("Wrote {} bytes to {}", content.len(), path),
                error: None,
            })
        })
    })
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct FileTools;

impl SelfRegisteringTool for FileTools {
    fn tool() -> Tool {
        // Return a combined representation — we'll register both tools
        // under different names. For this trait we return read_file as
        // the primary tool and register write_file separately.
        make_read_file_tool()
    }

    fn handler() -> ToolHandler {
        make_read_file_handler()
    }
}

/// Manually register all file-related tools.
pub fn register_file_tools(registry: &mut super::ToolRegistry) {
    registry.register(make_read_file_tool(), make_read_file_handler());
    registry.register(make_write_file_tool(), make_write_file_handler());
}
