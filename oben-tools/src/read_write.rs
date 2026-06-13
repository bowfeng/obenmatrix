use serde_json::Value;
/// File read/write tools.
///
/// Implements `Tool` trait directly.
use std::path::PathBuf;

use super::registry::{Tool, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};
use oben_utils::path_security::is_path_safe;

// ===========================================================================
// read_file tool
// ===========================================================================

fn make_read_file_tool_def() -> ToolMeta {
    let params = vec![ToolParameter {
        name: "path".into(),
        description: "Path to the file".into(),
        parameter_type: "string".into(),
        required: true,
    }];
    ToolMeta {
        name: "read_file".into(),
        description: "Read the contents of a file".into(),
        parameters: ToolParameters::Flat(params),
    }
}

pub struct ReadFileTool;

async fn execute_read_file(args: &Value) -> anyhow::Result<ToolResult> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

    if !is_path_safe(std::path::Path::new(path)) {
        return Ok(ToolResult {
            call_id: args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            output: String::new(),
            error: Some("Unsafe file path".to_string()),
        });
    }

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?;

    let content = if content.len() > 100_000 {
        let truncated: String = content.chars().take(100_000).collect();
        format!(
            "{}... (truncated, {} chars total)",
            truncated,
            content.chars().count()
        )
    } else {
        content
    };

    Ok(ToolResult {
        call_id: args
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        output: content,
        error: None,
    })
}

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file"
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        execute_read_file(args).await.unwrap_or_else(|e| ToolResult {
            call_id: args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

// ===========================================================================
// write_file tool
// ===========================================================================

fn make_write_file_tool_def() -> ToolMeta {
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
    ToolMeta {
        name: "write_file".into(),
        description: "Write content to a file".into(),
        parameters: ToolParameters::Flat(params),
    }
}

pub struct WriteFileTool;

async fn execute_write_file(args: &Value) -> anyhow::Result<ToolResult> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

    if !is_path_safe(std::path::Path::new(path)) {
        return Ok(ToolResult {
            call_id: args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            output: String::new(),
            error: Some("Unsafe file path".to_string()),
        });
    }

    let path_buf = PathBuf::from(path);
    let dir = path_buf.parent().unwrap_or(std::path::Path::new("."));
    tokio::fs::create_dir_all(dir).await?;

    // Use UTF-8 char-safe slicing — NEVER content[..N] for multi-byte chars
    let byte_limit = content.len().min(100_000);
    let written_chars: usize = content[..byte_limit].chars().count();
    tokio::fs::write(path, content).await?;

    Ok(ToolResult {
        call_id: args
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        output: format!("Wrote {} chars to {}", written_chars, path),
        error: None,
    })
}

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file"
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        execute_write_file(args).await.unwrap_or_else(|e| ToolResult {
            call_id: args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Manually register all file-related tools.
pub fn register(registry: &mut ToolRegistry) {
    let read_tool = Box::new(ReadFileTool);
    registry.register_with_def(read_tool, make_read_file_tool_def());
    let write_tool = Box::new(WriteFileTool);
    registry.register_with_def(write_tool, make_write_file_tool_def());
}
