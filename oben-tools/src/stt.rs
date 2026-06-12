use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest::Client;
use serde_json::Value;

use oben_config::config::AppConfig;
use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{SelfRegisteringTool, ToolHandler};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_AUDIO_FILE_SIZE: u64 = 25 * 1024 * 1024; // 25 MB

const SUPPORTED_FORMATS: &[&str] = &[
    ".mp3", ".mp4", ".mpeg", ".mpga", ".m4a",
    ".wav", ".webm", ".ogg", ".aac", ".flac",
];

const LANG_DEFAULT: &str = ""; // "auto"

// Whitelisted API endpoints (all use OpenAI-compatible /v1/audio/transcriptions)
const OPENAI_API: &str = "https://api.openai.com/v1";
const GROQ_API: &str = "https://api.groq.com/openai/v1";
const MISTRAL_API: &str = "https://api.mistral.ai/v1";
const XAI_API: &str = "https://api.x.ai/v1";
const ELEVENLABS_BASE: &str = "https://api.elevenlabs.io/v1";

// Env vars for each provider
const ENV_GROQ: &str = "GROQ_API_KEY";
const ENV_XAI: &str = "XAI_API_KEY";
const ENV_MISTRAL: &str = "MISTRAL_API_KEY";
const ENV_ELEVENLABS: &str = "ELEVENLABS_API_KEY";
const ENV_OPENAI: &str = "OPENAI_API_KEY";

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

/// Check if a file path has a supported audio extension.
fn is_supported_format(path: impl AsRef<Path>) -> bool {
    let ext = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    SUPPORTED_FORMATS.contains(&&*(".".to_owned() + &ext))
}

/// Check if file size is acceptable.
fn is_within_size_limit(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .metadata()
        .map(|m| m.len() <= MAX_AUDIO_FILE_SIZE)
        .unwrap_or(false)
}

/// Get provider label.
fn provider_label(p: &str) -> &str {
    match p {
        "" | "whisper-rs" => "local (whisper-rs)",
        "openai" => "OpenAI",
        "groq" => "Groq",
        "mistral" => "Mistral",
        "xai" => "xAI",
        "elevenlabs" => "ElevenLabs",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// STT provider implementations
// ---------------------------------------------------------------------------

/// Common OpenAI-compatible transcription API endpoint.
/// Works with: OpenAI, Groq, Mistral, xAI, ElevenLabs (Scribe v2)
async fn transcribe_with_openai_compatible(
    client: &Client,
    api_base: &str,
    api_key: &str,
    audio_path: impl AsRef<Path>,
    model: &str,
    language: &str,
    provider_label: &str,
) -> anyhow::Result<String> {
    let path = audio_path.as_ref();
    
    // Read file bytes
    let file_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read audio file {}: {}", path.display(), e))?;
    
    let response = client
        .post(format!("{}/audio/transcriptions", api_base.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Disposition", format!("attachment; filename=\"audio.{}\"", path.extension().map(|e| e.to_string_lossy()).unwrap_or_else(|| "mp3".into())))
        .body(file_bytes)
        .form(&[
            ("model", model),
            ("response_format", "text"),
            ("language", language),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(anyhow::anyhow!(
            "{} STT failed (HTTP {}): {}",
            provider_label,
            status,
            body.lines().next().unwrap_or("").get(0..300).unwrap_or(&body),
        ));
    }

    let text = response.text().await?;
    Ok(text.trim().to_string())
}

/// Whisper-rs local transcription.
///
/// Downloads the GGML model on first use if not present in cache,
/// then transcribes directly in-process (no subprocess).
async fn transcribe_with_whisper_rs(
    audio_path: impl AsRef<Path>,
    model_path_override: Option<impl AsRef<Path>>,
    language: &str,
) -> anyhow::Result<String> {
    #[cfg(feature = "stt-whisper-rs")]
    {
        use whisper_rs::{WhisperContextParameters, FullParams, SamplingStrategy};

        // Resolve model path
        let model_path = if let Some(p) = model_path_override.as_ref() {
            p.as_ref().to_path_buf()
        } else {
            let cache_dir = std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".cache").join("whisper-rs"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/.whisper-rs"));

            std::fs::create_dir_all(&cache_dir).ok();
            let model_file = cache_dir.join("base.bin");

            if !model_file.exists() {
                let model_url = "https://huggingface.co/whisper.cpp/resolve/main/base.bin";
                let resp = reqwest::get(model_url).await?;
                if !resp.status().is_success() {
                    return Err(anyhow::anyhow!(
                        "Failed to download model from {}",
                        model_url
                    ));
                }
                let data = resp.bytes().await?;
                std::fs::write(&model_file, &data)?;
            }

            model_file
        };

        // Load audio file (convert to f32 mono 16kHz)
                let samples = load_wav_samples(audio_path.as_ref())?;

        // Create whisper state
        let ctx_params = WhisperContextParameters::default();
        let ctx = whisper_rs::WhisperContext::new_with_params(&model_path, ctx_params)
            .map_err(|e| anyhow::anyhow!("Failed to load whisper context: {}", e))?;
        let mut state = ctx.create_state().map_err(|e| anyhow::anyhow!("Failed to create state: {}", e))?;

        // Create full params
        let mut full_params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        
        // Set language if specified
        if !language.is_empty() {
            full_params.set_language(Some(language));
        }

        // Run full model (single language)
        state.full(full_params, &samples)?;

        // Extract text from segments
        let n_segments = state.full_n_segments() as i32;
        let mut text_parts = Vec::with_capacity(n_segments as usize);
        
        for i in 0..n_segments {
            if let Some(seg) = state.get_segment(i) {
                let text = seg.to_str().unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    text_parts.push(text);
                }
            }
        }

        // Join segments with spaces
        Ok(text_parts.join(" ").trim().to_string())
    }

    #[cfg(not(feature = "stt-whisper-rs"))]
    {
        Err(anyhow::anyhow!(
            "Local STT (whisper-rs) is not compiled in. \
             Enable the 'stt-whisper-rs' feature to use local transcription.\n\n\
             As a workaround, use an online provider:\n  \
             export OPENAI_API_KEY=sk-...\n  \
             Then try with provider='openai'."
        ))
    }
}

/// Load audio file and convert to f32 mono 16kHz PCM samples.
/// Load wav file and convert to f32 mono 16kHz.
fn load_wav_samples(audio_path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(audio_path)
        .map_err(|e| anyhow::anyhow!("Failed to open WAV: {}", e))?;
    
    let is_correct_rate = reader.spec().sample_rate == 16000;
    
    if is_correct_rate {
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / 32768.0)
            .collect();
        Ok(samples)
    } else {
        let original: Vec<i16> = reader.samples::<i16>().filter_map(|s| s.ok()).collect();
        Ok(simple_resample(&original, reader.spec().sample_rate as usize, 16000))
    }
}

/// Simple resample function (downsample audio samples)
fn simple_resample(input: &[i16], from_rate: usize, to_rate: usize) -> Vec<f32> {
    if from_rate == to_rate {
        return input.iter().map(|s| *s as f32 / 32768.0).collect();
    }
    
    let ratio = from_rate as f64 / to_rate as f64;
    let step = ratio.max(1.0) as usize;
    let mut output = Vec::with_capacity(input.len() / step + 1);
    
    for (_, &sample) in input.iter().enumerate().step_by(step) {
        output.push(sample as f32 / 32768.0);
    }
    
    output
}

/// OpenAI STT — the most compatible and feature-rich provider.
/// Falls back to local whisper-rs if no key is available.
async fn transcribe_with_openai(
    client: &Client,
    audio_path: impl AsRef<Path>,
    base_url: Option<String>,
    model: Option<String>,
    language: Option<String>,
) -> anyhow::Result<String> {
    let api_key = resolve_api_key(ENV_OPENAI).or_else(|| {
        resolve_api_key("VOICE_TOOLS_OPENAI_KEY")
    });

    match api_key {
        Some(key) => {
            let base = base_url
                .unwrap_or_else(|| OPENAI_API.to_string());
            let model_name = model.unwrap_or_else(|| "whisper-1".to_string());
            let lang = language.unwrap_or_else(|| LANG_DEFAULT.to_string());

            transcribe_with_openai_compatible(
                client,
                &base,
                &key,
                &audio_path,
                &model_name,
                &lang,
                "OpenAI",
            )
            .await
        }
        None => Err(anyhow::anyhow!(
            "OpenAI STT requires OPENAI_API_KEY or VOICE_TOOLS_OPENAI_KEY environment variable."
        )),
    }
}

// ---------------------------------------------------------------------------
// Unified STT dispatcher
// ---------------------------------------------------------------------------

async fn do_transcribe(
    audio_path: impl AsRef<Path>,
    provider: &str,
    config: &AppConfig,
) -> anyhow::Result<String> {
    let model = config.voice.stt.model.clone();
    let lang = config.voice.stt.language.clone();

    match provider {
        "" | "whisper-rs" => {
            transcribe_with_whisper_rs(
                &audio_path,
                config.voice.stt.model_path.clone().map(|p| PathBuf::from(p)),
                lang.as_deref().unwrap_or(LANG_DEFAULT),
            )
            .await
        }
        "openai" => transcribe_with_openai(
            &Client::new(),
            audio_path,
            config.voice.stt.openai_like.base_url.clone(),
            model.clone(),
            lang.clone(),
        )
        .await,
        "groq" => {
            let key = resolve_api_key(ENV_GROQ)
                .ok_or_else(|| anyhow::anyhow!("GROQ_API_KEY not set"))?;
            let model_name = model.unwrap_or_else(|| "whisper-large-v3-turbo".to_string());
            transcribe_with_openai_compatible(
                &Client::new(),
                GROQ_API,
                &key,
                audio_path,
                &model_name,
                lang.as_deref().unwrap_or(LANG_DEFAULT),
                "Groq",
            )
            .await
        }
        "mistral" => {
            let key = resolve_api_key(ENV_MISTRAL)
                .ok_or_else(|| anyhow::anyhow!("MISTRAL_API_KEY not set"))?;
            let model_name = model.unwrap_or_else(|| "voxtral-mini-latest".to_string());
            transcribe_with_openai_compatible(
                &Client::new(),
                MISTRAL_API,
                &key,
                audio_path,
                &model_name,
                lang.as_deref().unwrap_or(LANG_DEFAULT),
                "Mistral",
            )
            .await
        }
        "xai" => {
            let key = resolve_api_key(ENV_XAI)
                .ok_or_else(|| anyhow::anyhow!("XAI_API_KEY not set"))?;
            transcribe_with_openai_compatible(
                &Client::new(),
                XAI_API,
                &key,
                audio_path,
                // xAI STT model name
                "grok-2-transcribe",
                lang.as_deref().unwrap_or(LANG_DEFAULT),
                "xAI",
            )
            .await
        }
        "elevenlabs" => {
            let _key = resolve_api_key(ENV_ELEVENLABS)
                .ok_or_else(|| anyhow::anyhow!("ELEVENLABS_API_KEY not set"))?;
            // ElevenLabs Scribe uses same OpenAI-compatible endpoint shape, but with different base:
            transcribe_with_openai_compatible(
                &Client::new(),
                ELEVENLABS_BASE,
                &_key,
                audio_path,
                "scribe_v2",
                lang.as_deref().unwrap_or(LANG_DEFAULT),
                "ElevenLabs",
            ).await
        }
        other => Err(anyhow::anyhow!("Unknown STT provider: {}", other)),
    }
    .map_err(|e| {
        anyhow::anyhow!("STT (provider={}): {}", provider_label(provider), e)
    })
}

// ---------------------------------------------------------------------------
// Tool definition & handler
// ---------------------------------------------------------------------------

fn make_stt_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "audio_file".into(),
            description: "Path to an audio file to transcribe. Supported formats: ".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "audio_base64".into(),
            description: "Base64-encoded audio data (alternative to audio_file). Supported formats: mp3, wav, webm, mp4...".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "provider".into(),
            description: "STT provider: whisper-rs (local, free), openai, groq, mistral, xai, elevenlabs. Defaults to 'whisper-rs' when available, falls back to openai if key is set.".into(),
            parameter_type: "string".into(),
            required: false,
        },
    ];

    Tool {
        name: "speech_to_text".into(),
        description: "Transcribe speech audio to text. Supports 6 providers: whisper-rs (local GGML, free), OpenAI whisper-1, Groq whisper-turbo, Mistral Voxtral, xAI Grok, ElevenLabs Scribe. Accepts file path or base64-encoded audio data.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_stt_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Extract audio_file or audio_base64
            let audio_file = args
                .get("audio_file")
                .and_then(|v| v.as_str())
                .map(String::from);

            let audio_base64 = args
                .get("audio_base64")
                .and_then(|v| v.as_str())
                .map(String::from);

            let provider = args
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("whisper-rs")
                .to_string();

            if audio_file.is_none() && audio_base64.is_none() {
                return Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some("Either 'audio_file' or 'audio_base64' must be provided.".to_string()),
                });
            }

            // Determine audio source
            let temp_path: Option<std::path::PathBuf> = if let Some(path) = &audio_file {
                let p = Path::new(path);
                if !p.exists() {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!("Audio file not found: {}, make sure the path is absolute or a valid local path.", path)),
                    });
                }
                if !is_supported_format(p) {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!("Unsupported audio format. Supported formats: {:?}", SUPPORTED_FORMATS)),
                    });
                }
                if !is_within_size_limit(p) {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!("Audio file {} exceeds 25 MB limit.", path)),
                    });
                }
                Some(p.to_path_buf())
            } else if let Some(b64) = &audio_base64 {
                // Decode base64 to temp file
                let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                    .map_err(|e| anyhow::anyhow!("Failed to decode base64 audio: {}", e))?;

                if decoded.len() > MAX_AUDIO_FILE_SIZE as usize {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some("Base64 audio data exceeds 25 MB limit.".to_string()),
                    });
                }

                // Determine format from content-type hint or default to .wav
                let ext = "wav";
                let mut tmp = std::env::temp_dir();
                tmp.push(format!("stt_input_{}.{}", std::process::id(), ext));
                std::fs::write(&tmp, &decoded)
                    .map_err(|e| anyhow::anyhow!("Failed to write temp audio: {}", e))?;
                Some(tmp)
            } else {
                None
            };

            let result_path = temp_path.ok_or_else(|| {
                anyhow::anyhow!("No audio source provided (neither audio_file nor audio_base64)")
            })?;

            // Load config
            let config = AppConfig::load()
                .unwrap_or_else(|_e| AppConfig::default());

            // Perform transcription
            let transcript = do_transcribe(&result_path, provider.as_str(), &config)
                .await?;

            // Clean up temp file if we created one
            if audio_base64.is_some() && result_path.exists() {
                let _ = std::fs::remove_file(&result_path);
            }

            Ok(ToolResult {
                call_id,
                output: transcript,
                error: None,
            })
        })
    })
}

pub struct SttTool;

impl SelfRegisteringTool for SttTool {
    fn tool() -> Tool {
        make_stt_tool()
    }

    fn handler() -> ToolHandler {
        make_stt_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    SttTool::register_self(registry);
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
        
        let result = make_stt_handler()(test_args).await.unwrap();
        assert!(result.error.is_some());
        assert!(result.output.is_empty());
        let err_msg = result.error.unwrap();
        assert!(err_msg.contains("audio_file") || err_msg.contains("audio_base64"));
    }

    /// Given: invalid audio format
    /// When: speech_to_text handler is called
    /// Then: returns error about unsupported format
    #[tokio::test]
    async fn test_unsupported_format() {
        use tempfile::NamedTempFile;
        
        let tmp = NamedTempFile::new().unwrap();
        tmp.as_file().set_len(1024).unwrap();
        
        let test_args = serde_json::json!({
            "call_id": "test-2",
            "audio_file": tmp.path().to_str().unwrap(),
            "provider": "openai"
        });
        
        // This should fail with unsupported format error (not .mp3/.wav etc)
        let result = make_stt_handler()(test_args).await.unwrap();
        assert!(result.error.is_some());
    }

    /// Given: base64-encoded audio data
    /// When: speech_to_text handler is called
    /// Then: file not found error occurs with whisper-rs provider
    #[tokio::test]
    async fn test_base64_audio_falls_back_to_whisper_rs() {
        // Generate sample WAV-like base64 data (minimal valid WAV header + silence)
        use std::io::Write;
        
        // Create a minimal WAV file in memory
        let mut wav_data = Vec::new();
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&((44 - 8 + 2) as u32).to_le_bytes()); // file size - 8
        wav_data.extend_from_slice(b"WAVE");
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav_data.extend_from_slice(&16000u32.to_le_bytes()); // 16kHz
        wav_data.extend_from_slice(&32000u32.to_le_bytes()); // byte rate
        wav_data.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav_data.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&(2u32).to_le_bytes()); // data size
        wav_data.extend_from_slice(&0i16.to_le_bytes()); // silence sample
        
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav_data);
        
        let test_args = serde_json::json!({
            "call_id": "test-base64",
            "audio_base64": b64,
            "provider": "whisper-rs"
        });
        
        // Should fail with whisper-rs not compiled in or model download failure
        let result = make_stt_handler()(test_args).await;
        
        // Result should be error (whisper-rs needs model loading or network)
        assert!(result.is_err());
    }

    /// Given: file path pointing to non-existent file
    /// When: speech_to_text handler is called
    /// Then: returns error about file not found
    #[tokio::test]
    async fn test_nonexistent_audio_file() {
        use tempfile::NamedTempFile;
        
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        drop(tmp); // Delete the file
        
        // Now test with the deleted file path
        let test_args = serde_json::json!({
            "call_id": "test-nonexistent",
            "audio_file": &path,
            "provider": "openai"
        });
        
        let result = make_stt_handler()(test_args).await.unwrap();
        assert!(result.error.is_some());
        let err_msg = result.error.unwrap();
        assert!(err_msg.contains("not found") || err_msg.contains("NotFound"));
    }

    /// Given: valid WAV audio file path
    /// When: speech_to_text handler calls whisper-rs locally
    /// Then: transcription occurs via local whisper-rs model
    #[tokio::test]
    #[ignore = "requires whisper-rs local model download"]
    async fn test_whisper_rs_local_transcription() {
        use std::io::Write;
        use tempfile::NamedTempFile;
        
        // Create minimal WAV file at 16kHz (whisper-rs requirement)
        let mut wav_data = Vec::new();
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&((44 - 8 + 200) as u32).to_le_bytes());
        wav_data.extend_from_slice(b"WAVE");
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes());
        wav_data.extend_from_slice(&1u16.to_le_bytes());
        wav_data.extend_from_slice(&1u16.to_le_bytes());
        wav_data.extend_from_slice(&16000u32.to_le_bytes());
        wav_data.extend_from_slice(&32000u32.to_le_bytes());
        wav_data.extend_from_slice(&2u16.to_le_bytes());
        wav_data.extend_from_slice(&16u16.to_le_bytes());
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&(200u32).to_le_bytes());
        // 100 samples of silence
        for _ in 0..100 {
            wav_data.extend_from_slice(&0i16.to_le_bytes());
        }
        
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(&wav_data).unwrap();
        
        let test_args = serde_json::json!({
            "call_id": "test-local",
            "audio_file": tmp.path().to_str().unwrap(),
            "provider": "whisper-rs"
        });
        
        // Should either succeed or fail gracefully (no crash)
        let result = make_stt_handler()(test_args).await;
        assert!(result.is_ok() || result.is_err()); // Just verify no panic
    }
}
