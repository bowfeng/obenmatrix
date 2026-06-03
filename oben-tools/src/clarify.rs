use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};
use serde_json::Value;
/// Clarify tool — asks the user for clarification on ambiguous tasks.
///
/// Pauses execution and requests user input to resolve ambiguity.
use std::sync::Arc;

use super::registry::{SelfRegisteringTool, ToolHandler};

/// Clarify tool definition
fn make_clarify_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "question".into(),
            description: "The question to ask the user for clarification.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "options".into(),
            description: "Optional list of suggested options for the user to choose from.".into(),
            parameter_type: "array".into(),
            required: false,
        },
    ];
    Tool {
        name: "clarify".into(),
        description: "Ask the user for clarification on an ambiguous task. Pauses execution until user responds.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

/// Clarify tool handler
fn make_clarify_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let question = args
                .get("question")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'question' argument"))?;

            let options = args
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Format the question with options if provided
            let mut output = format!("❓ Question: {}\n", question);
            if !options.is_empty() {
                output.push_str("\nSuggested options:\n");
                for (i, opt) in options.iter().enumerate() {
                    output.push_str(&format!("  {}. {}\n", i + 1, opt));
                }
            }

            output.push_str("\nWaiting for user response...");

            Ok(ToolResult {
                call_id,
                output,
                error: None,
            })
        })
    })
}

/// Self-registration
pub struct ClarifyTool;

impl SelfRegisteringTool for ClarifyTool {
    fn tool() -> Tool {
        make_clarify_tool()
    }

    fn handler() -> ToolHandler {
        make_clarify_handler()
    }
}

/// Register this module into the given registry.
/// Called automatically by `discover_builtin_tools`.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    ClarifyTool::register_self(registry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        ClarifyTool::register_self(&mut registry);
        registry
    }

    #[tokio::test]
    async fn asks_question() {
        let registry = make_registry();
        let result = registry
            .execute(
                "clarify",
                &json!({
                    "question": "Which programming language should I use?",
                    "call_id": "test-1",
                }),
            )
            .await;

        assert!(result.error.is_none());
        assert!(result.output.contains("Question:"));
        assert!(result.output.contains("programming language"));
    }

    #[tokio::test]
    async fn includes_options() {
        let registry = make_registry();
        let result = registry
            .execute(
                "clarify",
                &json!({
                    "question": "What format for output?",
                    "options": ["JSON", "Markdown", "CSV"],
                    "call_id": "test-2",
                }),
            )
            .await;

        assert!(result.error.is_none());
        assert!(result.output.contains("JSON"));
        assert!(result.output.contains("Markdown"));
        assert!(result.output.contains("CSV"));
    }

    #[tokio::test]
    async fn handles_missing_question() {
        let registry = make_registry();
        let result = registry
            .execute(
                "clarify",
                &json!({
                    "call_id": "test-3",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result
            .error
            .as_ref()
            .unwrap()
            .contains("Missing 'question'"));
    }
}
