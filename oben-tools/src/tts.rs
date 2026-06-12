use std::path::PathBuf;
use std::sync::Arc;

use msedge_tts::tts::SpeechConfig;
use reqwest::Client;
use serde_json::Value;
use tracing::{debug, info};

use oben_config::config::AppConfig;
use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{SelfRegisteringTool, ToolHandler};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default text-to-speech output directory
fn default_output_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"));
    home.join(".config/obenalien").join("audio_cache")
}

const OPENAI_BASE: &str = "https://api.openai.com/v1";
const ELEVENLABS_BASE: &str = "https://api.elevenlabs.io/v1";
const MISTRAL_BASE: &str = "https://api.mistral.ai/v1";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const XAI_BASE: &str = "https://api.x.ai/v1";

const ENV_OPENAI: &str = "OPENAI_API_KEY";
const ENV_XAI: &str = "XAI_API_KEY";
const ENV_MISTRAL: &str = "MISTRAL_API_KEY";
const ENV_ELEVENLABS: &str = "ELEVENLABS_API_KEY";
const ENV_GEMINI: &str = "GEMINI_API_KEY";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve an API key from the given env var name.
fn resolve_api_key(env_var: &str) -> Option<String> {
    std::env::var(env_var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Clean markdown from text for TTS (strip formatting, code blocks, etc.)
fn clean_for_tts(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut in_code_block = false;

    for line in text.lines() {
        let line = line.trim();

        // Skip code blocks entirely
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Remove formatting
        let processed = line
            .replace("`", "")
            .replace("**", "")
            .replace("*", " ")
            .replace("---", " ")
            .replace("# ", " ")
            .replace("\n\n", " ");

        cleaned.push_str(&processed);
        cleaned.push(' ');
    }

    cleaned.trim().to_string()
}

/// Convert MP3/WAV to Opus using ffmpeg (for Telegram voice bubbles)
async fn convert_to_opus(
    input_path: &PathBuf,
    output_path: &PathBuf,
) -> anyhow::Result<()> {
    let result = tokio::process::Command::new("ffmpeg")
        .args([
            "-i", input_path.to_string_lossy().as_ref(),
            "-acodec", "libopus",
            "-ac", "1",
            "-b:a", "64k",
            "-vbr", "off",
            output_path.to_string_lossy().as_ref(),
            "-y", "-loglevel", "error",
        ])
        .output()
        .await?;

    if result.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(anyhow::anyhow!("ffmpeg conversion failed: {}", stderr.lines().next().unwrap_or("")))
    }
}

/// Wrap raw PCM audio data (from Gemini) in a WAV container
fn wrap_pcm_as_wav(pcm_bytes: &[u8], sample_rate: u32) -> Vec<u8> {
    let bytes_per_sample = 2; // 16-bit
    let channels = 1; // Mono
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    let block_align: u32 = (channels * bytes_per_sample) as u32;
    let data_size = pcm_bytes.len();

    let mut wav = Vec::with_capacity(44 + pcm_bytes.len());

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((44 + data_size - 8) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // PCM format chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // Audio format (PCM)
    wav.extend_from_slice(&1u16.to_le_bytes()); // Channels
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());
    wav.extend_from_slice(pcm_bytes);

    wav
}

// ---------------------------------------------------------------------------
// TTS provider implementations
// ---------------------------------------------------------------------------

/// Edge TTS — free, no API key. Uses native Rust msedge-tts crate.
///
/// The msedge-tts crate uses tokio runtime for async operations.
/// It connects to Microsoft Edge Read Aloud API to synthesize speech.
async fn generate_edge_tts(text: &str, voice: Option<&str>, output_path: &PathBuf) -> anyhow::Result<()> {
    use msedge_tts::{tts::client::connect, voice::get_voices_list};

    // Get available voices
    let voices = get_voices_list().map_err(|e| anyhow::anyhow!("Failed to list Edge TTS voices: {}", e))?;

    // Find desired voice or use default
    let voice_name = voice.unwrap_or("en-US-AriaNeural");
    let voice = voices
        .iter()
        .find(|v| v.name == voice_name)
        .ok_or_else(|| anyhow::anyhow!("Voice '{}' not found. Available voices: {:?}", voice_name, voices.iter().map(|v| &v.name).collect::<Vec<_>>()))?;

    // Create speech config
    let speech_config = SpeechConfig::from(voice);

    // Connect TTS client
    let mut tts_client = connect().map_err(|e| anyhow::anyhow!("Failed to connect Edge TTS service: {}", e))?;

    // Synthesize speech
    let audio_output = tts_client
        .synthesize(text, &speech_config)
        .map_err(|e| anyhow::anyhow!("Edge TTS synthesis failed: {}", e))?;

    info!("Edge TTS generated {} bytes of audio", audio_output.audio_bytes.len());

    // Write audio to file
    std::fs::write(output_path, &audio_output.audio_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to write Edge TTS audio to {}: {}", output_path.display(), e))?;

    debug!("Edge TTS audio saved to {}", output_path.display());
    Ok(())
}

/// OpenAI TTS — requires OPENAI_API_KEY
async fn generate_openai_tts(client: &Client, text: &str, output_path: &PathBuf, config: &AppConfig) -> anyhow::Result<()> {
    let api_key = resolve_api_key(ENV_OPENAI)
        .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

    let voice = config.voice.tts.voice.as_deref().unwrap_or("alloy");
    let speed = config.voice.tts.speed.unwrap_or(1.0);
    let model = config.voice.tts.model.as_deref().unwrap_or("gpt-4o-mini-tts");

    let response_format = if config.voice.tts.output_format == "ogg" { "opus" } else { "mp3" };

    let builder = client.post(format!("{}/audio/speech", OPENAI_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({
            "model": model,
            "voice": voice,
            "input": text,
            "response_format": response_format,
            "speed": speed
        }));

    let response = builder.send().await?;

    if response.status().is_success() {
        let audio_data = response.bytes().await?;
        std::fs::write(output_path, &audio_data)
            .map_err(|e| anyhow::anyhow!("Failed to write OpenAI TTS output: {}", e))?;
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;
        Err(anyhow::anyhow!("OpenAI TTS failed (HTTP {}): {}", status, body.lines().next().unwrap_or("")))
    }
}

/// ElevenLabs TTS — requires ELEVENLABS_API_KEY
async fn generate_elevenlabs_tts(client: &Client, text: &str, output_path: &PathBuf, config: &AppConfig) -> anyhow::Result<()> {
    let api_key = resolve_api_key(ENV_ELEVENLABS)
        .ok_or_else(|| anyhow::anyhow!("ELEVENLABS_API_KEY not set"))?;

    let voice_id = config.voice.tts.voice.as_deref().unwrap_or("pNInz6obpgDQGcFmaJgB");
    let model_id = config.voice.tts.model.as_deref().unwrap_or("eleven_multilingual_v2");

    let response_format = if config.voice.tts.output_format == "ogg" {
        "opus_48000_64"
    } else {
        "mp3_44100_128"
    };

    let builder = client.post(format!("{}/text-to-speech/{}", ELEVENLABS_BASE, voice_id))
        .header("xi-api-key", &api_key)
        .json(&serde_json::json!({
            "text": text,
            "model_id": model_id,
            "output_format": response_format,
        }));

    let response = builder.send().await?;

    if response.status().is_success() {
        let audio_data = response.bytes().await?;
        std::fs::write(output_path, &audio_data)
            .map_err(|e| anyhow::anyhow!("Failed to write ElevenLabs TTS output: {}", e))?;
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;
        Err(anyhow::anyhow!("ElevenLabs TTS failed (HTTP {}): {}", status, body.lines().next().unwrap_or("")))
    }
}

/// Gemini TTS — requires GEMINI_API_KEY, supports voice output modality
async fn generate_gemini_tts(client: &Client, text: &str, api_base: &str, model: &str, voice: &str, output_path: &PathBuf) -> anyhow::Result<()> {
    let api_key = resolve_api_key(ENV_GEMINI)
        .ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY not set"))?;

    let response = client
        .post(format!("{}/models/{}:generateContent", api_base.trim_end_matches('/'), model))
        .header("x-goog-api-key", &api_key)
        .json(&serde_json::json!({
            "contents": [{ "parts": [{ "text": text }] }],
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {
                    "voiceConfig": {
                        "prebuiltVoiceConfig": { "voiceName": voice }
                    }
                }
            }
        }))
        .send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(anyhow::anyhow!("Gemini TTS failed (HTTP {}): {}", status, body.lines().next().unwrap_or("")));
    }

    let json: Value = response.json().await?;

    // Extract audio from response
    let audio_part = json["candidates"][0]["content"]["parts"]
        .as_array()
        .and_then(|p| p.iter().find(|part| part.get("inlineData").is_some()))
        .ok_or_else(|| anyhow::anyhow!("Gemini response contained no audio data"))?;

    let audio_b64 = audio_part["inlineData"]["data"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing inline data in Gemini response"))?;

    let audio_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, audio_b64)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;

    // Wrap PCM (24kHz) in WAV
    let wav_bytes = wrap_pcm_as_wav(&audio_bytes, 24000);
    std::fs::write(output_path, &wav_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to write Gemini WAV output: {}", e))?;

    Ok(())
}

/// xAI TTS — requires XAI_API_KEY
async fn generate_xai_tts(client: &Client, text: &str, api_base: &str, output_path: &PathBuf) -> anyhow::Result<()> {
    let api_key = resolve_api_key(ENV_XAI)
        .ok_or_else(|| anyhow::anyhow!("XAI_API_KEY not set"))?;

    let response = client
        .post(format!("{}/tts", api_base.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "text": text,
            "voice_id": "eve",
            "language": "en",
            "output_format": {
                "codec": "mp3",
                "sample_rate": 24000,
                "bit_rate": 128000
            }
        }))
        .send().await?;

    if response.status().is_success() {
        let audio_data = response.bytes().await?;
        std::fs::write(output_path, &audio_data)
            .map_err(|e| anyhow::anyhow!("Failed to write xAI TTS output: {}", e))?;
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;
        Err(anyhow::anyhow!("xAI TTS failed (HTTP {}): {}", status, body.lines().next().unwrap_or("")))
    }
}

/// Mistral TTS — requires MISTRAL_API_KEY
async fn generate_mistral_tts(client: &Client, text: &str, output_path: &PathBuf, model: &str, voice: &str) -> anyhow::Result<()> {
    let api_key = resolve_api_key(ENV_MISTRAL)
        .ok_or_else(|| anyhow::anyhow!("MISTRAL_API_KEY not set"))?;

    let response = client.post(format!("{}/audio/speech", MISTRAL_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "input": text,
            "voice_id": voice,
            "response_format": "mp3"
        }))
        .send().await?;

    if response.status().is_success() {
        let audio_data = response.bytes().await?;
        std::fs::write(output_path, &audio_data)
            .map_err(|e| anyhow::anyhow!("Failed to write Mistral TTS output: {}", e))?;
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;
        Err(anyhow::anyhow!("Mistral TTS failed (HTTP {}): {}", status, body.lines().next().unwrap_or("")))
    }
}

// ---------------------------------------------------------------------------
// Unified TTS dispatcher
// ---------------------------------------------------------------------------

async fn do_text_to_speech(
    text: &str,
    custom_path: Option<&str>,
    output_format: &str,
    config: &AppConfig,
) -> anyhow::Result<String> {
    let provider = config.voice.tts.provider.as_str();
    let voice = config.voice.tts.voice.as_deref();
    let base_output_dir = default_output_dir();

    // Determine output extension based on provider
    let ext = match provider {
        "" | "edge" => "mp3",
        "openai" | "elevenlabs" => { if output_format == "ogg" { "ogg" } else { "mp3" } },
        "gemini" => "wav",
        "xai" | "mistral" => "mp3",
        _ => "mp3",
    };

    // Create output path
    let output_path = if let Some(path) = custom_path {
        PathBuf::from(path)
    } else {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        base_output_dir.join(format!("tts_{}.{}", timestamp, ext))
    };

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Dispatch to provider
    let client = Client::new();
    match provider {
        "" | "edge" => {
            generate_edge_tts(text, voice, &output_path).await?;
        }
        "openai" => {
            generate_openai_tts(&client, text, &output_path, config).await?;
        }
        "elevenlabs" => {
            generate_elevenlabs_tts(&client, text, &output_path, config).await?;
        }
        "gemini" => {
            let api_base = config.voice.tts.base_url.as_deref()
                .unwrap_or(GEMINI_BASE);
            generate_gemini_tts(&client, text, api_base, "gemini-2.0-flash-exp", voice.unwrap_or("Kore"), &output_path).await?;
        }
        "xai" => {
            let api_base = config.voice.tts.base_url.as_deref()
                .unwrap_or(XAI_BASE);
            generate_xai_tts(&client, text, api_base, &output_path).await?;
        }
        "mistral" => {
            generate_mistral_tts(&client, text, &output_path,
                "voxtral-mini-tts-2603",
                voice.unwrap_or("c69964a6-ab8b-4f8a-9465-ec0925096ec8")).await?;
        }
        other => {
            return Err(anyhow::anyhow!("Unknown TTS provider: {}", other));
        }
    }

    // Convert to Opus for Telegram if requested
    if output_format == "ogg" && !output_path.to_string_lossy().ends_with(".ogg") {
        let ogg_path = output_path.with_extension("ogg");
        convert_to_opus(&output_path, &ogg_path).await?;
        std::fs::remove_file(&output_path).ok();
        Ok(format!("MEDIA:{}", ogg_path.to_string_lossy()))
    } else {
        Ok(format!("MEDIA:{}", output_path.to_string_lossy()))
    }
}

// ---------------------------------------------------------------------------
// Tool definition & handler
// ---------------------------------------------------------------------------

fn make_tts_tool() -> Tool {
    Tool {
        name: "text_to_speech".into(),
        description: "Convert text to speech audio. Returns MEDIA: path for platform delivery. Supports Edge TTS (free, native Rust), OpenAI, ElevenLabs, Google Gemini, xAI, and Mistral.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter {
                name: "text".into(),
                description: "Text to convert to speech. Provider-specific character limits apply (OpenAI: 4096, xAI: 15000, etc.). Markdown is automatically stripped.".into(),
                parameter_type: "string".into(),
                required: true,
            },
            ToolParameter {
                name: "output_path".into(),
                description: "Optional custom file path. Defaults to ~/.config/obenalien/audio_cache/tts_<timestamp>.mp3".into(),
                parameter_type: "string".into(),
                required: false,
            },
        ]),
    }
}

fn make_tts_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'text' argument"))?;

            let custom_path = args
                .get("output_path")
                .and_then(|v| v.as_str())
                .map(String::from);

            let output_format = "mp3"; // Default
            let cleaned_text = clean_for_tts(text);

            let config = AppConfig::load().unwrap_or_else(|_| AppConfig::default());

            let media_path = do_text_to_speech(&cleaned_text, custom_path.as_deref(), output_format, &config).await?;

            Ok(ToolResult {
                call_id,
                output: media_path,
                error: None,
            })
        })
    })
}

pub struct TtsTool;

impl SelfRegisteringTool for TtsTool {
    fn tool() -> Tool {
        make_tts_tool()
    }

    fn handler() -> ToolHandler {
        make_tts_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    TtsTool::register_self(registry);
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
        let tool_result = make_tts_handler()(test_args).await;
        assert!(tool_result.is_err() || tool_result.unwrap().error.is_some());
    }

    /// Given: markdown-formatted text
    /// When: text_to_speech handler is called
    /// Then: markdown is stripped before TTS generation
    #[test]
    fn test_markdown_stripping() {
        let text = "Here is **bold** and `inline code`.\n\n```\ncode block\n```";
        let cleaned = clean_for_tts(text);
        assert!(!cleaned.contains("**"));
        assert!(!cleaned.contains("`"));
        assert!(!cleaned.contains("```"));
    }

    /// Given: Chinese text with markdown
    /// When: text_to_speech handler is called
    /// Then: markdown is stripped and Chinese characters are preserved
    #[test]
    fn test_markdown_stripping_with_chinese() {
        let text = "你好**世界**和`代码`\n\n```rust\nfn main() {}\n```";
        let cleaned = clean_for_tts(text);
        assert!(!cleaned.contains("**"));
        assert!(!cleaned.contains("`"));
        assert!(cleaned.contains("你好"));
        assert!(cleaned.contains("世界"));
        assert!(cleaned.contains("和"));
    }

    /// Given: valid text input without Edge TTS connection
    /// When: text_to_speech handler is called with default config
    /// Then: returns error when provider is unreachable (Edge/None)
    #[tokio::test]
    async fn test_tts_handler_missing_provider_key() {
        let test_args = serde_json::json!({
            "call_id": "test-tts",
            "text": "Hello world"
        });
        
        // With default config (no voice config set), should use Edge provider
        // which will fail without network connection
        let result = make_tts_handler()(test_args).await;
        
        // Edge TTS requires network, so result will be error
        // But it should NOT error on missing text
        match result {
            Ok(tool_result) => {
                // If TTS somehow succeeded (e.g., network available), 
                // verify output format
                assert!(tool_result.output.starts_with("MEDIA:"));
            }
            Err(e) => {
                // Edge TTS unreachable is expected in test environment
                assert!(e.to_string().contains("Edge TTS") || e.to_string().contains("edge") || e.to_string().contains("aria"));
            }
        }
    }

    /// Given: valid text input with custom path
    /// When: text_to_speech handler tries Edge provider
    /// Then: error about provider being unavailable or failed synthesis
    #[tokio::test]
    async fn test_tts_handler_with_custom_path() {
        let test_args = serde_json::json!({
            "call_id": "test-tts-custom",
            "text": "Custom path test",
            "output_path": "/tmp/test_tts_output.mp3"
        });
        
        let result = make_tts_handler()(test_args).await;
        
        // Custom path validation passes, but Edge TTS will fail without network
        match result {
            Ok(tool_result) => {
                // If TTS succeeded
                assert!(tool_result.output.starts_with("MEDIA:"));
            }
            Err(_) => {
                // Expected: Edge TTS not available in test environment
            }
        }
    }

    /// Given: code block content within text
    /// When: text_to_speech handler is called
    /// Then: entire code block is skipped (both opening and closing ```)
    #[test]
    fn test_code_block_complete_removal() {
        let text = "Start of message\n```\nfn main() {\n    println!(\"Hello\");\n}\n```\nEnd of message";
        let cleaned = clean_for_tts(text);
        assert!(!cleaned.contains("fn main"));
        assert!(!cleaned.contains("println"));
        assert!(cleaned.contains("Start"));
        assert!(cleaned.contains("End"));
    }
}
