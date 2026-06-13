use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_tts_tool() -> ToolMeta {
    ToolMeta {
        name: "text_to_speech".into(),
        description: "Convert text to speech audio. Returns MEDIA: path for platform delivery. Supports Edge TTS (free, native Rust), OpenAI, ElevenLabs, Google Gemini, xAI, and Mistral.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("text", "Text to convert to speech. Provider-specific character limits apply (OpenAI: 4096, xAI: 15000, etc.). Markdown is automatically stripped.", "string"),
            ToolParameter::optional("output_path", "Optional custom file path. Defaults to ~/.config/obenalien/audio_cache/tts_<timestamp>.mp3", "string"),
        ]),
    }
}

pub struct TtsTool;

/// Delegate to voice module for TTS execution.
async fn execute_tts<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    crate::voice::tts_execute(call).await
}

#[async_trait::async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str { "text_to_speech" }
    fn description(&self) -> &str { "Convert text to speech audio" }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_tts(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> { Box::new(Self) }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(TtsTool);
    registry.register_with_def(tool, make_tts_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: empty args without text
    /// When: text_to_speech handler is called
    /// Then: returns error "Missing 'text' argument"
    #[tokio::test]
    async fn test_missing_text() {
        let test_args = serde_json::json!({"call_id": "test-1"});
        let tool = TtsTool;
        let call = ToolCall::new("text_to_speech", &test_args);
        let result = tool.execute(&call).await;
        assert!(result.error.is_some());
    }
}
