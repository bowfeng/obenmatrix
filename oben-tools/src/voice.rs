use std::path::{Path, PathBuf};

use msedge_tts::tts::SpeechConfig;
use reqwest::Client;
use serde_json::Value;
use tracing::{debug, info};
use oben_config::config::AppConfig;

use super::registry::{ToolCall};

// ---------------------------------------------------------------------------
// STT Constants & Helpers
// ---------------------------------------------------------------------------

/// Maximum authorized audio file size (25 MB).
pub const MAX_AUDIO_FILE_SIZE: u64 = 25 * 1024 * 1024;

/// Supported audio formats for STT.
pub const SUPPORTED_FORMATS: &[&str] = &[
    ".mp3", ".mp4", ".mpeg", ".mpga", ".m4a",
    ".wav", ".webm", ".ogg", ".aac", ".flac",
];

const LANG_DEFAULT: &str = ""; // "auto"

// STT provider endpoints
const OPENAI_API: &str = "https://api.openai.com/v1";
const GROQ_API: &str = "https://api.groq.com/openai/v1";
const MISTRAL_API: &str = "https://api.mistral.ai/v1";
const XAI_API: &str = "https://api.x.ai/v1";
const ELEVENLABS_BASE: &str = "https://api.elevenlabs.io/v1";

// Env vars for STT
const ENV_GROQ: &str = "GROQ_API_KEY";
const ENV_XAI: &str = "XAI_API_KEY";
const ENV_MISTRAL: &str = "MISTRAL_API_KEY";
const ENV_ELEVENLABS: &str = "ELEVENLABS_API_KEY";
const ENV_OPENAI: &str = "OPENAI_API_KEY";

// ---------------------------------------------------------------------------
// TTS Constants
// ---------------------------------------------------------------------------


const OPENAI_BASE: &str = "https://api.openai.com/v1";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const XAI_TTS_BASE: &str = "https://api.x.ai/v1";
const MISTRAL_BASE: &str = "https://api.mistral.ai/v1";

// Env vars for TTS
const ENV_GEMINI: &str = "GEMINI_API_KEY";

// ---------------------------------------------------------------------------
// Shared Helpers
// ---------------------------------------------------------------------------

/// Resolve an API key from the given env var name.
fn resolve_api_key(env_var: &str) -> Option<String> {
    std::env::var(env_var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Check if a file path has a supported audio extension.
pub fn is_supported_format(path: impl AsRef<Path>) -> bool {
    let ext = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    SUPPORTED_FORMATS.contains(&&*(".".to_owned() + &ext))
}

/// Check if file size is acceptable.
pub fn is_within_size_limit(path: impl AsRef<Path>) -> bool {
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

/// Get default output directory for TTS audio files.
fn default_output_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    Path::new(&home).join(".obenmatrix").join("voice").join("audio_cache")
}

/// Load audio file and convert to f32 mono 16kHz.
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

// ---------------------------------------------------------------------------
// STT Provider Implementations
// ---------------------------------------------------------------------------

/// Whisper-rs local transcription.
async fn transcribe_with_whisper_rs(
    audio_path: impl AsRef<Path>,
    model_path_override: Option<impl AsRef<Path>>,
    language: &str,
) -> anyhow::Result<String> {
    #[cfg(feature = "stt-whisper-rs")]
    {
        use whisper_rs::{WhisperContextParameters, FullParams, SamplingStrategy};

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

        let samples = load_wav_samples(audio_path.as_ref())?;

        let ctx_params = WhisperContextParameters::default();
        let ctx = whisper_rs::WhisperContext::new_with_params(&model_path, ctx_params)
            .map_err(|e| anyhow::anyhow!("Failed to load whisper context: {}", e))?;
        let mut state = ctx.create_state().map_err(|e| anyhow::anyhow!("Failed to create state: {}", e))?;

        let mut full_params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        if !language.is_empty() {
            full_params.set_language(Some(language));
        }

        state.full(full_params, &samples)?;

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

// ---------------------------------------------------------------------------
// TTS Provider Implementations
// ---------------------------------------------------------------------------

/// Edge TTS — free, no API key. Uses native Rust msedge-tts crate.
async fn generate_edge_tts(text: &str, voice: Option<&str>, output_path: &PathBuf) -> anyhow::Result<()> {
    use msedge_tts::{tts::client::connect, voice::get_voices_list};

    let voices = get_voices_list().map_err(|e| anyhow::anyhow!("Failed to list Edge TTS voices: {}", e))?;

    let voice_name = voice.unwrap_or("en-US-AriaNeural");
    let voice = voices
        .iter()
        .find(|v| v.name == voice_name)
        .ok_or_else(|| anyhow::anyhow!("Voice '{}' not found. Available voices: {:?}", voice_name, voices.iter().map(|v| &v.name).collect::<Vec<_>>()))?;

    let speech_config = SpeechConfig::from(voice);

    let mut tts_client = connect().map_err(|e| anyhow::anyhow!("Failed to connect Edge TTS service: {}", e))?;

    let audio_output = tts_client
        .synthesize(text, &speech_config)
        .map_err(|e| anyhow::anyhow!("Edge TTS synthesis failed: {}", e))?;

    info!("Edge TTS generated {} bytes of audio", audio_output.audio_bytes.len());

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

    let audio_part = json["candidates"][0]["content"]["parts"]
        .as_array()
        .and_then(|p| p.iter().find(|part| part.get("inlineData").is_some()))
        .ok_or_else(|| anyhow::anyhow!("Gemini response contained no audio data"))?;

    let audio_b64 = audio_part["inlineData"]["data"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing inline data in Gemini response"))?;

    let audio_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, audio_b64)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;

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
// Unified STT Dispatcher
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
        "openai" => {
            let api_key = resolve_api_key(ENV_OPENAI).or_else(|| resolve_api_key("VOICE_TOOLS_OPENAI_KEY"));
            match api_key {
                Some(key) => {
                    let base = config.voice.stt.openai_like.base_url.clone().unwrap_or_else(|| OPENAI_API.to_string());
                    let model_name = model.unwrap_or_else(|| "whisper-1".to_string());
                    let lang = lang.unwrap_or_else(|| LANG_DEFAULT.to_string());
                    transcribe_with_openai_compatible(
                        &Client::new(), &base, &key, &audio_path, &model_name, &lang, "OpenAI"
                    ).await
                }
                None => Err(anyhow::anyhow!(
                    "OpenAI STT requires OPENAI_API_KEY or VOICE_TOOLS_OPENAI_KEY environment variable."
                )),
            }
        }
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
                "grok-2-transcribe",
                lang.as_deref().unwrap_or(LANG_DEFAULT),
                "xAI",
            )
            .await
        }
        "elevenlabs" => {
            let key = resolve_api_key(ENV_ELEVENLABS)
                .ok_or_else(|| anyhow::anyhow!("ELEVENLABS_API_KEY not set"))?;
            transcribe_with_openai_compatible(
                &Client::new(),
                ELEVENLABS_BASE,
                &key,
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
// Unified TTS Dispatcher
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

    let ext = match provider {
        "" | "edge" => "mp3",
        "openai" | "elevenlabs" => { if output_format == "ogg" { "ogg" } else { "mp3" } },
        "gemini" => "wav",
        "xai" | "mistral" => "mp3",
        _ => "mp3",
    };

    let output_path = if let Some(path) = custom_path {
        PathBuf::from(path)
    } else {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        base_output_dir.join(format!("tts_{}.{}", timestamp, ext))
    };

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

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
                .unwrap_or(XAI_TTS_BASE);
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
// Public Entry Points
// ---------------------------------------------------------------------------

/// STT entry point: prepares audio source (file or temp from base64) and transcribes.
/// Returns the transcript text.
pub async fn execute_stt<'a>(call: &ToolCall<'a>) -> anyhow::Result<String> {
    let audio_file = call.optional_str("audio_file").map(String::from);
    let audio_base64 = call.optional_str("audio_base64").map(String::from);
    let provider = call.optional_str("provider").unwrap_or("whisper-rs");

    if audio_file.is_none() && audio_base64.is_none() {
        return Err(anyhow::anyhow!("Either 'audio_file' or 'audio_base64' must be provided."));
    }

    let temp_path: Option<PathBuf> = if let Some(path) = &audio_file {
        let p = Path::new(path);
        if !p.exists() {
            return Err(anyhow::anyhow!(
                "Audio file not found: {}, make sure the path is absolute or a valid local path.",
                path
            ));
        }
        if !is_supported_format(p) {
            return Err(anyhow::anyhow!(
                "Unsupported audio format. Supported formats: {:?}",
                SUPPORTED_FORMATS
            ));
        }
        if !is_within_size_limit(p) {
            return Err(anyhow::anyhow!("Audio file {} exceeds 25 MB limit.", path));
        }
        Some(p.to_path_buf())
    } else if let Some(b64) = &audio_base64 {
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| anyhow::anyhow!("Failed to decode base64 audio: {}", e))?;
        if decoded.len() > MAX_AUDIO_FILE_SIZE as usize {
            return Err(anyhow::anyhow!("Base64 audio data exceeds 25 MB limit."));
        }
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

    let config = AppConfig::load().unwrap_or_else(|_| AppConfig::default());
    let transcript = do_transcribe(&result_path, provider, &config).await?;

    if audio_base64.is_some() && result_path.exists() {
        let _ = std::fs::remove_file(&result_path);
    }

    Ok(transcript)
}

/// TTS entry point: extracts text from call args, cleans it, and dispatches.
/// Returns the MEDIA: path of the generated audio.
pub async fn execute_tts<'a>(call: &ToolCall<'a>) -> anyhow::Result<String> {
    let text = call.required_str("text")?;
    let custom_path = call.optional_str("output_path");
    let output_format = "mp3";
    let cleaned_text = clean_for_tts(text);

    let config = AppConfig::load().unwrap_or_else(|_| AppConfig::default());
    let media_path = do_text_to_speech(&cleaned_text, custom_path, output_format, &config).await?;
    Ok(media_path)
}

// ---------------------------------------------------------------------------
// Re-export from voice module for stt.rs and tts.rs to use
// ---------------------------------------------------------------------------

/// Re-export STT logic for backward compat
pub async fn stt_execute<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    let transcript = execute_stt(call).await?;
    Ok(oben_models::ToolResult {
        call_id: call.call_id.clone(),
        output: transcript,
        error: None,
    })
}

/// Re-export TTS logic for backward compat
pub async fn tts_execute<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    let media_path = execute_tts(call).await?;
    Ok(oben_models::ToolResult {
        call_id: call.call_id.clone(),
        output: media_path,
        error: None,
    })
}
