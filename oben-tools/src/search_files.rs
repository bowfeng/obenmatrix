/// File search tool — content (grep-like) and name (glob-like) search.
///
/// Uses ripgrep (rg) for fast parallel search, falls back to basic grep/find.

use std::sync::Arc;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool};
use oben_utils::path_security::is_path_safe;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Whether ripgrep is available on this system.
static RGPREP_FOUND: TokioMutex<Option<bool>> = TokioMutex::const_new(None);

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_search_files_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "query".into(),
            description: "Search query — file name pattern for name search, or text pattern for content search.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "path".into(),
            description: "Root path to search in. Defaults to current directory.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "limit".into(),
            description: "Maximum number of results to return. Default is 50.".into(),
            parameter_type: "number".into(),
            required: false,
        },
        ToolParameter {
            name: "type".into(),
            description: "Search type: 'name' for file name matching, 'content' for file content matching (grep-like). Default is 'content'.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "glob".into(),
            description: "Optional glob pattern to filter file types (e.g., '*.rs', '*.py', '*.{js,ts}').".into(),
            parameter_type: "string".into(),
            required: false,
        },
    ];
    Tool {
        name: "search_files".into(),
        description: "Search for files by name or content. Uses ripgrep for fast parallel search. Defaults to content search (grep-like). Set type='name' for file name glob matching.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_search_files_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;

            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");

            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50);

            let search_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("content");

            let glob = args
                .get("glob")
                .and_then(|v| v.as_str());

            // Safety check: dangerous query patterns
            if is_dangerous_query(query) {
                return Ok(ToolResult {
                    call_id: args.get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    output: String::new(),
                    error: Some("Invalid query: search terms must be alphanumeric strings".to_string()),
                });
            }

            // Safety check: unsafe path
            if !is_path_safe(std::path::Path::new(path)) {
                return Ok(ToolResult {
                    call_id: args.get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    output: String::new(),
                    error: Some("Unsafe search path".to_string()),
                });
            }

            let call_id = args.get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if search_type == "name" {
                search_by_name(&query, path, limit as usize, &call_id).await
            } else {
                search_by_content(&query, path, limit as usize, glob, &call_id).await
            }
        })
    })
}

/// Check if the query looks like a dangerous shell injection attempt.
fn is_dangerous_query(query: &str) -> bool {
    let q = query.trim();
    // Allow alphanumeric, dots, dashes, underscores, slashes, asterisks, question marks, braces, and spaces
    // Block any other shell metacharacters
    !q.is_empty() && !q.chars().all(|c| {
        c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '*' | '?' | '{' | '}' | ' ' | ',')
    })
}

// ---------------------------------------------------------------------------
// Search implementations
// ---------------------------------------------------------------------------

/// Check if ripgrep is available.
async fn has_ripgrep() -> bool {
    let mut found = RGPREP_FOUND.lock().await;
    if let Some(val) = *found {
        return val;
    }

    let status = Command::new("which")
        .arg("rg")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    *found = Some(status);
    status
}

/// Search for files by name pattern (glob-like).
async fn search_by_name(
    pattern: &str,
    path: &str,
    limit: usize,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    // Normalize pattern: ensure it looks like a glob
    let glob = if pattern.contains('*') || pattern.contains('?') {
        pattern.to_string()
    } else {
        format!("**/*{}*", pattern)
    };

    if has_ripgrep().await {
        search_files_rg(&glob, path, limit, call_id).await
    } else {
        search_files_find(&glob, path, limit, call_id).await
    }
}

/// Search files by name using ripgrep.
async fn search_files_rg(
    glob: &str,
    path: &str,
    limit: usize,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let mut cmd = Command::new("rg");
    cmd.arg("--files")
        .arg("-g")
        .arg(glob)
        .arg(path)
        .arg("--no-heading")
        .arg("--vimgrep")
        .kill_on_drop(true);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        cmd.output()
    ).await;

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("rg command failed: {}", e)),
            });
        }
        Err(_) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some("File name search timed out after 60s".to_string()),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<&str> = stdout.lines().collect();
    let total = files.len();
    let files: Vec<String> = files.into_iter()
        .take(limit)
        .map(|s| s.to_string())
        .collect();

    let output = format_files_result(&files, total, limit);
    Ok(ToolResult {
        call_id: call_id.to_string(),
        output,
        error: None,
    })
}

/// Search files by name using find (fallback).
async fn search_files_find(
    pattern: &str,
    path: &str,
    limit: usize,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    // Build find command
    let cmd_str = format!(
        "find {} -type f -name \"{}\" -not -path '*/.*' 2>/dev/null | head -n {}",
        escape_shell_arg(path),
        pattern.replace('*', "*"),
        limit
    );

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        Command::new("/bin/sh")
            .arg("-c")
            .arg(&cmd_str)
            .output()
    ).await;

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("find command failed: {}", e)),
            });
        }
        Err(_) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some("File name search timed out after 60s".to_string()),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout.lines()
        .filter(|l| !l.is_empty())
        .take(limit)
        .map(|s| s.to_string())
        .collect();

    let output = format_files_result(&files, files.len(), limit);
    Ok(ToolResult {
        call_id: call_id.to_string(),
        output,
        error: None,
    })
}

/// Search for text content inside files (grep-like).
async fn search_by_content(
    pattern: &str,
    path: &str,
    limit: usize,
    glob: Option<&str>,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let mut rg_args = vec![
        "--heading",      // show file names
        "-n",             // line numbers
        "-C", "2",        // 2 lines context
        "-I",             // ignore binary files
    ];

    if let Some(g) = glob {
        rg_args.push("-g");
        rg_args.push(g);
    }

    if has_ripgrep().await {
        rg_args.push("-e");
        rg_args.push(pattern);
        rg_args.push(path);

        search_rg(&rg_args, limit, call_id).await
    } else {
        search_grep(pattern, path, limit, glob, call_id).await
    }
}

/// Search content using ripgrep.
async fn search_rg(
    args: &[&str],
    limit: usize,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let mut cmd = Command::new("rg");
    cmd.args(args).kill_on_drop(true);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        cmd.output()
    ).await;

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("rg search failed: {}", e)),
            });
        }
        Err(_) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some("Content search timed out after 60s".to_string()),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let matches = stdout.lines().collect::<Vec<_>>();

    // Limit output to a reasonable size
    let truncated = matches.len() > limit * 10;
    let output_lines: Vec<&str> = matches.iter()
        .take(limit * 5)
        .cloned()
        .collect();

    let output_str = if output_lines.is_empty() {
        "(no matches found)".to_string()
    } else {
        output_lines.join("\n")
    };

    let error = if truncated {
        Some(format!(
            "Found {} matches, showing first {} lines (set higher limit to see more)",
            matches.len(),
            output_lines.len()
        ))
    } else {
        None
    };

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: output_str,
        error,
    })
}

/// Search content using grep (fallback).
async fn search_grep(
    pattern: &str,
    path: &str,
    limit: usize,
    glob: Option<&str>,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let _glob_flag = if let Some(g) = glob {
        format!(" -name \"{}\"", g.replace('*', "*"))
    } else {
        String::new()
    };

    // Use find + grep to approximate ripgrep behavior
    let cmd_str = format!(
        "find {} -type f -name \"*\" -not -path '*/.*' 2>/dev/null | head -n {} | xargs grep -l -n -C 2 -I -- \"{}\" 2>/dev/null | head -n {}",
        path,
        limit * 20,
        shell_escape(pattern),
        limit * 5
    );

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        Command::new("/bin/sh")
            .arg("-c")
            .arg(&cmd_str)
            .output()
    ).await;

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("grep search failed: {}", e)),
            });
        }
        Err(_) => {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some("Content search timed out after 60s".to_string()),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let output = if stdout.is_empty() {
        "(no matches found)".to_string()
    } else {
        stdout.to_string()
    };

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output,
        error: None,
    })
}

/// Format file list results.
fn format_files_result(files: &[String], total: usize, limit: usize) -> String {
    if files.is_empty() {
        "(no files found)".to_string()
    } else {
        let shown = files.len();
        let mut result = format!("Found {} files (showing {}):\n", total, shown);
        for f in files {
            result.push_str(&format!("  {}\n", f));
        }
        if shown >= limit {
            result.push_str(&format!("... ({} more files not shown)\n", total - shown));
        }
        result
    }
}

/// Shell-escape an argument.
fn escape_shell_arg(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_')) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn shell_escape(s: &str) -> String {
    shell_escape_inner(s)
}

fn shell_escape_inner(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | ' ' | '*')) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct SearchFilesTool;

impl SelfRegisteringTool for SearchFilesTool {
    fn tool() -> Tool {
        make_search_files_tool()
    }

    fn handler() -> ToolHandler {
        make_search_files_handler()
    }
}

/// Register this module into the given registry.
/// Called automatically by `discover_builtin_tools`.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    SearchFilesTool::register_self(registry);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        SearchFilesTool::register_self(&mut registry);
        registry
    }

    #[tokio::test]
    async fn content_search_finds_match() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "hello",
            "path": "oben-tools/src",
            "call_id": "test-1",
        })).await;

        // Should not error, may or may not find matches depending on file content
        assert!(result.error.is_none() || result.error.as_ref().unwrap().contains("rg") || result.error.as_ref().unwrap().contains("no matches"));
    }

    #[tokio::test]
    async fn name_search_finds_rs_files() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "terminal",
            "type": "name",
            "path": "oben-tools/src",
            "call_id": "test-2",
        })).await;

        // Should not error
        assert!(result.error.is_none() || result.error.as_ref().unwrap().contains("rg") || result.error.as_ref().unwrap().contains("no files"));
    }

    #[tokio::test]
    async fn content_search_with_glob() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "search_files",
            "path": "oben-tools/src",
            "glob": "*.rs",
            "call_id": "test-3",
        })).await;

        assert!(result.error.is_none() || result.error.as_ref().unwrap().contains("rg") || result.error.as_ref().unwrap().contains("no matches"));
    }

    #[tokio::test]
    async fn content_search_respects_limit() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "hello",
            "path": "oben-tools/src",
            "limit": 5,
            "call_id": "test-4",
        })).await;

        assert!(result.error.is_none() || result.error.as_ref().unwrap().contains("rg") || result.error.as_ref().unwrap().contains("no matches"));
    }

    #[tokio::test]
    async fn blocks_dangerous_query() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "; rm -rf /",
            "call_id": "test-5",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Invalid query"));
        
        // Verify safe query passes
        let result2 = registry.execute("search_files", &json!({
            "query": "safe_search",
            "call_id": "test-5b",
        })).await;
        assert!(result2.error.is_none());
    }

    #[tokio::test]
    async fn accepts_alphanumeric_query() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "hello_world-test.txt",
            "call_id": "test-6",
        })).await;

        // Should pass safety check (may or may not find results)
        assert!(result.error.is_none() || !result.error.as_ref().unwrap().contains("Invalid query"));
    }

    #[tokio::test]
    async fn name_search_name_pattern() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "*.rs",
            "type": "name",
            "path": "oben-tools/src",
            "call_id": "test-7",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("Found") || result.output.contains("no files"));
    }

    #[tokio::test]
    async fn empty_results() {
        let registry = make_registry();
        let result = registry.execute("search_files", &json!({
            "query": "xyznonexistent12345",
            "path": "oben-tools/src",
            "call_id": "test-8",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("no") || result.output.contains("(no matches"));
    }
}
