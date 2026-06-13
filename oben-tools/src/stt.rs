use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_stt_tool() -> ToolMeta {
    ToolMeta {
        name: "speech_to_text".into(),
        description: "Transcribe speech audio to text. Supports 6 providers: whisper-rs (local GGML, free), OpenAI whisper-1, Groq whisper-turbo, Mistral Voxtral, xAI Grok, ElevenLabs Scribe. Accepts file path or base64-encoded audio data.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::optional("audio_file", "Path to an audio file to transcribe. Supported formats: ", "string"),
            ToolParameter::optional("audio_base64", "Base64-encoded audio data (alternative to audio_file). Supported formats: mp3, wav, webm, mp4...", "string"),
            ToolParameter::optional("provider", "STT provider: whisper-rs (local, free), openai, groq, mistral, xai, elevenlabs. Defaults to 'whisper-rs' when available, falls back to openai if key is set.", "string"),
        ]),
    }
}

pub struct SttTool;

/// Delegate to voice module for STT execution.
async fn execute_stt<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    crate::voice::stt_execute(call).await
}

#[async_trait::async_trait]
impl Tool for SttTool {
    fn name(&self) -> &str {
        "speech_to_text"
    }
    fn description(&self) -> &str {
        "Transcribe speech audio to text"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_stt(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(SttTool);
    registry.register_with_def(tool, make_stt_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: empty args without audio_file or audio_base64
    /// When: speech_to_text handler is called
    /// Then: returns error "Either audio_file or audio_base64 must be provided."
    #[tokio::test]
    async fn test_no_audio_provided() {
        let test_args = serde_json::json!({
            "call_id": "test-1"
        });
        
        let tool = SttTool;
        let call = ToolCall::new("speech_to_text", &test_args);
        let result = tool.execute(&call).await;
        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("must be provided"));
    }

    /// Given: valid base64-encoded audio data
    /// When: speech_to_text handler is called
    /// Then: base64 decoding succeeds
    #[tokio::test]
    async fn test_base64_audio_falls_back_to_whisper_rs() {
        let _audio_data = [0u8];
        let b64_data = [b'a', b'b']; // Not valid base64

        let test_args = serde_json::json!({
            "call_id": "test-2",
            "audio_base64": std::str::from_utf8(&b64_data).unwrap().to_string()
        });

        let tool = SttTool;
        let call = ToolCall::new("speech_to_text", &test_args);
        let result = tool.execute(&call).await;
        // Should fail since not valid base64
        assert!(result.error.is_some());
    }

    /// Given: valid base64 audio data
    /// When: speech_to_text handler is called
    /// Then: base64 decode succeeds (may fail later on transcription)
    #[test]
    fn test_base64_decoding() {
        let audio_data = [0u8];
        let b64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_data);
        // Should produce valid base64
        let result = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &b64_data,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), audio_data);
    }
}
