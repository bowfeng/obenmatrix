use serde_json::Value;

use super::registry::{Tool, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_clarify_tool_def() -> ToolMeta {
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
    ToolMeta {
        name: "clarify".into(),
        description: "Ask the user for clarification on an ambiguous task. Pauses execution until user responds.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct ClarifyTool;

/// Format a question with optional suggestions for user responses.
async fn execute_clarify(args: &Value) -> anyhow::Result<ToolResult> {
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

    let mut output = format!("❓ Question: {}\n", question);
    if !options.is_empty() {
        output.push_str("\nSuggested options:\n");
        for (i, opt) in options.iter().enumerate() {
            output.push_str(&format!("  {}. {}\n", i + 1, opt));
        }
    }
    output.push_str("\nWaiting for user response...");

    Ok(ToolResult { call_id, output, error: None })
}

#[async_trait::async_trait]
impl Tool for ClarifyTool {
    fn name(&self) -> &str {
        "clarify"
    }
    fn description(&self) -> &str {
        "Ask the user for clarification on an ambiguous task"
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        execute_clarify(args).await.unwrap_or_else(|e| ToolResult {
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

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(ClarifyTool);
    registry.register_with_def(tool, make_clarify_tool_def());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        crate::clarify::register(&mut registry);
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
