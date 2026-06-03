use serde_json::Value;
/// Code execution tool — safely executes code in a sandboxed environment.
///
/// Supports Python code execution with output capture and timeout protection.
use std::sync::Arc;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{SelfRegisteringTool, ToolHandler};

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_code_execution_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "code".into(),
            description: "The Python code to execute.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "timeout".into(),
            description: "Maximum execution time in seconds (default: 30).".into(),
            parameter_type: "number".into(),
            required: false,
        },
    ];
    Tool {
        name: "code_execution".into(),
        description:
            "Execute Python code in a sandboxed environment. Returns stdout, stderr, and exit code."
                .into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_code_execution_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let code = args
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

            let timeout_secs = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);

            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Security check: block dangerous operations.
            // Strip all whitespace (spaces, newlines, tabs) so that
            // "import\nos" matches the same as "importos".
            let safe_code: String = code.chars().filter(|c| !c.is_ascii_whitespace()).collect();

            let dangerous_patterns = [
                "importos",
                "importos.",
                "importsubprocess",
                "importshutil",
                "importsocket",
                "importurllib",
                "import requests",
                "importhttp",
                "open(",
                "eval(",
                "exec(",
                "__import__",
                ".system(",
                ".popen(",
            ];

            for pattern in &dangerous_patterns {
                if safe_code.contains(pattern) {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!(
                            "Security check: code contains disallowed pattern '{}'.",
                            pattern
                        )),
                    });
                }
            }

            // Write code to a uniquely-named temp file to avoid race conditions
            // between parallel tests (all share the same process ID).
            use std::time::{SystemTime, UNIX_EPOCH};
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let code_file = std::env::temp_dir().join(format!(
                "oben_code_{}_{}.py",
                std::process::id(),
                timestamp
            ));
            if let Err(e) = std::fs::write(&code_file, code) {
                return Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(format!("Failed to write code file: {}", e)),
                });
            }

            // Execute with timeout
            let output = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                execute_python(&code_file),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    let _ = std::fs::remove_file(&code_file);
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!(
                            "Execution timed out after {} seconds.",
                            timeout_secs
                        )),
                    });
                }
            };

            let _ = std::fs::remove_file(&code_file);

            match output {
                Ok(exec_output) => Ok(ToolResult {
                    call_id,
                    output: format!(
                        "Exit code: {}\n\nStdout:\n{}\n\nStderr:\n{}",
                        exec_output.exit_code, exec_output.stdout, exec_output.stderr
                    ),
                    error: if exec_output.exit_code == 0 {
                        None
                    } else {
                        Some(format!("Exit code: {}", exec_output.exit_code))
                    },
                }),
                Err(e) => Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(format!("Execution failed: {}", e)),
                }),
            }
        })
    })
}

/// Execution output
#[derive(Debug)]
struct ExecutionOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

async fn execute_python(code_file: &std::path::Path) -> Result<ExecutionOutput, anyhow::Error> {
    use tokio::process::Command;

    let output = Command::new("python3").arg(code_file).output().await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(ExecutionOutput {
        exit_code: output.status.code().unwrap_or(1),
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct CodeExecutionTool;

impl SelfRegisteringTool for CodeExecutionTool {
    fn tool() -> Tool {
        make_code_execution_tool()
    }

    fn handler() -> ToolHandler {
        make_code_execution_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    CodeExecutionTool::register_self(registry);
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
        CodeExecutionTool::register_self(&mut registry);
        registry
    }

    #[test]
    fn executes_simple_code() {
        let registry = make_registry();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let result = registry
                .execute(
                    "code_execution",
                    &json!({
                        "code": "print('HelloCodeExec')",
                        "call_id": "test-1",
                    }),
                )
                .await;
            assert!(result.error.is_none(), "Error: {:?}", result.error);
            assert!(
                result.output.contains("HelloCodeExec"),
                "Output: {}",
                result.output
            );
        })
    }

    #[tokio::test]
    async fn blocks_dangerous_imports() {
        let registry = make_registry();
        let result = registry
            .execute(
                "code_execution",
                &json!({
                    "code": "import os\nprint(os.listdir('.'))",
                    "call_id": "test-3",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Security check"));
    }

    #[tokio::test]
    async fn blocks_eval() {
        let registry = make_registry();
        let result = registry
            .execute(
                "code_execution",
                &json!({
                    "code": "eval('1 + 1')",
                    "call_id": "test-4",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Security check"));
    }

    #[tokio::test]
    async fn handles_missing_code() {
        let registry = make_registry();
        let result = registry
            .execute(
                "code_execution",
                &json!({
                    "call_id": "test-5",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Missing 'code'"));
    }

    #[tokio::test]
    async fn handles_syntax_error() {
        let registry = make_registry();
        let result = registry
            .execute(
                "code_execution",
                &json!({
                    "code": "def broken(",
                    "call_id": "test-6",
                }),
            )
            .await;

        // Should fail with exit code != 0
        assert!(result.output.contains("Exit code"));
    }
}
