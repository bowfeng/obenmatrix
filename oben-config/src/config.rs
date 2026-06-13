use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use oben_models::ProviderConfig;

pub use oben_models::SessionStoreKind;

/// All application settings, stored in ~/.config/obenalien/config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub model: ProviderConfig,
    pub temperature: Option<f64>,
    pub max_tokens: Option<usize>,
    pub max_iterations: Option<usize>,
    /// Maximum delegation depth for subagent spawning. When depth >= this value,
    /// children become leaf agents and cannot delegate further.
    pub max_spawn_depth: Option<usize>,
    /// Maximum number of subagent tasks running concurrently in batch mode.
    /// Excess tasks wait in a queue until a slot opens up. Default is 5.
    pub max_concurrent_tasks: Option<usize>,
    pub tools: ToolsConfig,
    pub skills: SkillsConfig,
    pub gateway: Option<GatewayConfig>,
    pub display: DisplayConfig,
    pub context: ContextConfig,
    pub voice: VoiceConfig,
    pub providers: Vec<ProviderConfig>,
    pub custom_providers: Vec<String>,
    pub vision: VisionConfig,
    /// Session storage backend: "database" (default) or "memory".
    #[serde(default)]
    pub session_store: SessionStoreKind,
    /// Retry behavior for API calls.
    #[serde(default)]
    pub retry: RetryConfig,
    /// Concurrency settings for tool dispatch and compaction.
    #[serde(default)]
    pub concurrency: ConcurrencyConfig,
    /// Post-turn hook settings.
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Fallback model chain.
    pub fallback_models: Vec<FallbackConfig>,
    /// Agent personality and behavior.
    #[serde(default)]
    pub agent: AgentConfig,
    /// Event routing configuration for the callback relay.
    #[serde(default)]
    pub events: EventsConfig,
}

/// Configuration for vision/image analysis.
/// When set, vision tools call the specified API with downloaded images.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionConfig {
    /// API provider: "openai" or "anthropic".
    #[serde(default = "default_vision_provider")]
    pub provider: String,
    /// Base URL for the API (use OpenRouter-compatible URL for multi-provider).
    pub base_url: Option<String>,
    /// API key for the vision provider.
    pub api_key: Option<String>,
    /// Model name for vision analysis.
    pub model: Option<String>,
    /// Max tokens for the analysis response.
    pub max_tokens: Option<usize>,
}

fn default_vision_provider() -> String {
    "openai".to_string()
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            provider: default_vision_provider(),
            base_url: None,
            api_key: None,
            model: None,
            max_tokens: Some(1024),
        }
    }
}

/// Configuration for voice (STT + TTS) tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// STT (speech-to-text) configuration.
    pub stt: SttConfig,
    /// TTS (text-to-speech) configuration.
    pub tts: TtsConfig,
}

/// STT provider selection.
/// Returns the default provider name ("whisper-rs").
fn default_stt_provider() -> String {
    String::from("whisper-rs")
}

/// STT configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    /// Provider: "whisper-rs" (local GGML), "openai", "groq", "mistral", "xai", "elevenlabs".
    #[serde(default = "default_stt_provider")]
    pub provider: String,
    /// Model name (provider-specific). Defaults vary by provider.
    pub model: Option<String>,
    /// Language override (ISO 639-1 code, e.g. "en", "zh", "de"). Auto-detect if None.
    pub language: Option<String>,
    // Provider-specific sub-configs
    /// OpenAI-compatible STT config (used as base URL for OpenAI/Groq/Mistral/xAI/ElevenLabs).
    #[serde(flatten)]
    pub openai_like: OpenAiAudioConfig,
    /// Whisper local model path (relative to HOME or absolute).
    /// If None, uses the default "base" model downloaded from official source.
    pub model_path: Option<String>,
}

/// OpenAI-compatible audio API configuration.
/// All OpenAI-compatible providers (OpenAI itself, Groq, Mistral, xAI, ElevenLabs)
/// share the same `/v1/audio/transcriptions` API shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiAudioConfig {
    /// Base URL for the transcription API. Defaults to "https://api.openai.com/v1".
    pub base_url: Option<String>,
}

fn default_openai_audio_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

impl Default for OpenAiAudioConfig {
    fn default() -> Self {
        Self {
            base_url: Some(default_openai_audio_base_url()),
        }
    }
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: default_stt_provider(),
            model: None,
            language: None,
            openai_like: OpenAiAudioConfig::default(),
            model_path: None,
        }
    }
}

/// TTS provider selection.
/// Returns the default provider name ("edge").
fn default_tts_provider() -> String {
    String::from("edge")
}

/// TTS configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    /// Provider: "edge" (default, free), "openai", "elevenlabs", "gemini", "xai", "mistral".
    #[serde(default = "default_tts_provider")]
    pub provider: String,
    /// Voice name/ID for the provider.
    pub voice: Option<String>,
    /// Speed multiplier (e.g. 1.0 = normal, 0.5 = slow, 2.0 = fast).
    pub speed: Option<f64>,
    /// Output format: .mp3 (default), .ogg (Telegram voice bubbles), .wav.
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Base URL for OpenAI-compatible providers (OpenAI, xAI). Not used by Edge/Mistral/ElevenLabs/Gemini.
    pub base_url: Option<String>,
    /// Model name for OpenAI provider.
    pub model: Option<String>,
    /// Custom command TTS configuration for local engines (Piper, KittenTTS, etc.).
    #[serde(default)]
    pub command: Option<CommandTtsConfig>,
}

fn default_output_format() -> String {
    "mp3".to_string()
}

/// Custom command TTS configuration for local engines (Piper, KittenTTS, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandTtsConfig {
    /// Shell command template. Hermes writes text to a temp file and runs:
    ///   {command} with {input_path}, {text_path}, {output_path}, {format} placeholders.
    pub command: String,
    /// Output format: mp3, wav, ogg, flac.
    #[serde(default = "default_cmd_tts_format")]
    pub output_format: String,
    /// If true, output is treated as voice-compatible (for Telegram voice bubbles).
    #[serde(default)]
    pub voice_compatible: bool,
}

fn default_cmd_tts_format() -> String {
    "mp3".to_string()
}

impl Default for CommandTtsConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            output_format: default_cmd_tts_format(),
            voice_compatible: false,
        }
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            provider: default_tts_provider(),
            voice: None,
            speed: None,
            output_format: default_output_format(),
            base_url: None,          // Providers use their own defaults
            model: None,             // Providers use their own defaults
            command: None,
        }
    }
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt: SttConfig::default(),
            tts: TtsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub enabled: Vec<String>,
    /// Auto-enable tools by category.
    #[serde(default)]
    pub auto_detect: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            auto_detect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    pub enabled: Vec<String>,
    pub auto_use: Vec<String>,
    /// Skill directories to scan (default: ["skills"]).
    #[serde(default = "default_skills_dirs")]
    pub dirs: Vec<String>,
}

fn default_skills_dirs() -> Vec<String> {
    vec!["skills".to_string()]
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            auto_use: vec![],
            dirs: default_skills_dirs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub telegram: Option<PlatformConfig>,
    pub discord: Option<PlatformConfig>,
    pub slack: Option<PlatformConfig>,
    pub whatsapp: Option<PlatformConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub enabled: bool,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub spinner_style: String,
    pub theme: String,
    pub code_block_language: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            spinner_style: "dots".to_string(),
            theme: "dark".to_string(),
            code_block_language: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    /// Max messages to keep in context before compression.
    #[serde(default)]
    pub max_messages: Option<usize>,
    /// Compression method: "summary", "token_count", "none".
    #[serde(default = "default_compression")]
    pub compression: String,
    /// Total context window size in tokens (default: 128000).
    #[serde(default = "default_context_length")]
    pub context_length: usize,
    /// Token threshold as percentage of context_length for compaction (default: 0.75 = 75%).
    #[serde(default = "default_threshold_percent")]
    pub threshold_percent: f64,
    /// Project context file names to discover (walk to git root for first entry).
    pub files: Vec<String>,
    /// Max chars to read from context files.
    #[serde(default = "default_context_max_chars")]
    pub max_chars: usize,
}

fn default_context_max_chars() -> usize {
    20_000
}

fn default_compression() -> String {
    "summary".to_string()
}

fn default_context_length() -> usize {
    128_000
}

fn default_threshold_percent() -> f64 {
    0.75
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_messages: Some(100),
            compression: "summary".to_string(),
            context_length: default_context_length(),
            threshold_percent: default_threshold_percent(),
            files: vec![".obenalien.md".to_string(), "OBEN.md".to_string(), "AGENTS.md".to_string(), "CLAUDE.md".to_string(), ".cursorrules".to_string()],
            max_chars: default_context_max_chars(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryConfig {
    #[serde(default = "default_retry_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_retry_base_delay_ms")]
    pub base_delay_ms: u64,
    #[serde(default = "default_retry_max_delay_ms")]
    pub max_delay_ms: u64,
    #[serde(default = "default_retry_jitter_factor")]
    pub jitter_factor: f64,
    pub retryable_codes: Vec<u16>,
}

fn default_retry_max_retries() -> u32 {
    3
}

fn default_retry_base_delay_ms() -> u64 {
    500
}

fn default_retry_max_delay_ms() -> u64 {
    60_000
}

fn default_retry_jitter_factor() -> f64 {
    0.5
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_retry_max_retries(),
            base_delay_ms: default_retry_base_delay_ms(),
            max_delay_ms: default_retry_max_delay_ms(),
            jitter_factor: default_retry_jitter_factor(),
            retryable_codes: vec![429, 500, 502, 503, 504],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConcurrencyConfig {
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default)]
    pub serial_only_tools: Vec<String>,
    pub destructive_tools: Vec<String>,
}

fn default_max_concurrency() -> usize {
    8
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            max_concurrency: default_max_concurrency(),
            serial_only_tools: vec![],
            destructive_tools: vec![
                "write_file".to_string(),
                "patch".to_string(),
                "create_dir".to_string(),
                "delete_file".to_string(),
                "shell".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    /// Hook types enabled for this agent (e.g. ["nudge"]).
    /// Each type must have a corresponding entry in `configs`.
    #[serde(default = "default_hooks_enabled")]
    pub enabled: Vec<String>,
    /// Per-hook-type configuration keyed by hook type name.
    #[serde(default)]
    pub configs: std::collections::BTreeMap<String, serde_yaml::Value>,
}

fn default_hooks_enabled() -> Vec<String> {
    vec!["nudge".to_string()]
}

impl Default for HooksConfig {
    fn default() -> Self {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "nudge".to_string(),
            serde_yaml::Value::Mapping(Default::default()),
        );
        Self {
            enabled: default_hooks_enabled(),
            configs: map,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_discipline: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            identity: None,
            execution_discipline: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EventsConfig {
    #[serde(default)]
    pub tool_events: EventFilters,
    #[serde(default)]
    pub thought_events: EventFilters,
    #[serde(default)]
    pub status_events: EventFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EventFilters {
    #[serde(default = "default_true")]
    pub progress: bool,
    #[serde(default = "default_true")]
    pub start_complete: bool,
    #[serde(default = "default_true")]
    pub thinking: bool,
    #[serde(default = "default_true")]
    pub reasoning: bool,
    #[serde(default = "default_true")]
    pub lifecycle: bool,
    #[serde(default = "default_true")]
    pub fallback: bool,
}

fn default_true() -> bool {
    true
}

impl Default for EventFilters {
    fn default() -> Self {
        Self {
            progress: true,
            start_complete: true,
            thinking: true,
            reasoning: true,
            lifecycle: true,
            fallback: true,
        }
    }
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            tool_events: EventFilters::default(),
            thought_events: EventFilters::default(),
            status_events: EventFilters::default(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model: ProviderConfig::new(
                oben_models::ProviderKind::OpenRouter,
                "qwen/qwen3-235b:free",
            ),
            temperature: Some(0.7),
            max_tokens: Some(8192),
            max_iterations: Some(50),
            max_spawn_depth: Some(3),
            max_concurrent_tasks: Some(5),
            tools: ToolsConfig {
                enabled: vec![],
                auto_detect: true,
            },
            skills: SkillsConfig {
                enabled: vec![],
                auto_use: vec![],
                dirs: default_skills_dirs(),
            },
            gateway: None,
            display: DisplayConfig {
                spinner_style: "dots".to_string(),
                theme: "dark".to_string(),
                code_block_language: true,
            },
            context: ContextConfig {
                max_messages: Some(100),
                compression: "summary".to_string(),
                context_length: 128_000,
                threshold_percent: 0.75,
                files: vec![".obenalien.md".to_string(), "OBEN.md".to_string(), "AGENTS.md".to_string(), "CLAUDE.md".to_string(), ".cursorrules".to_string()],
                max_chars: default_context_max_chars(),
            },
            providers: Vec::new(),
            custom_providers: Vec::new(),
            vision: VisionConfig::default(),
            voice: VoiceConfig::default(),
            session_store: SessionStoreKind::Database,
            retry: RetryConfig::default(),
            concurrency: ConcurrencyConfig::default(),
            hooks: HooksConfig::default(),
            fallback_models: Vec::new(),
            agent: AgentConfig::default(),
            events: EventsConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults;
    use oben_models::ProviderKind;
    use std::fs;

    #[test]
    fn test_default_model_is_openrouter_qwen() {
        let config = AppConfig::default();
        assert_eq!(config.model.kind, ProviderKind::OpenRouter);
        assert_eq!(config.model.model, "qwen/qwen3-235b:free");
        assert_eq!(config.model.default_model, "qwen/qwen3-235b:free");
        assert!(config.model.api_key.is_none());
    }

    #[test]
    fn test_default_settings() {
        let config = AppConfig::default();
        assert_eq!(config.temperature, Some(0.7));
        assert_eq!(config.max_tokens, Some(8192));
        assert_eq!(config.max_iterations, Some(50));
        assert!(config.tools.auto_detect);
        assert_eq!(config.display.theme, "dark");
        assert_eq!(config.context.compression, "summary");
        assert_eq!(config.context.max_messages, Some(100));
    }

    #[test]
    fn test_default_system_prompt_not_empty() {
        let prompt = defaults::default_system_prompt();
        assert!(!prompt.trim().is_empty());
        assert!(prompt.contains("AI agent"));
        assert!(prompt.contains("tools"));
    }

    #[test]
    fn test_config_yaml_roundtrip() {
        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        // Compare core fields
        assert_eq!(restored.model.kind, config.model.kind);
        assert_eq!(restored.model.model, config.model.model);
        assert_eq!(restored.temperature, config.temperature);
        assert_eq!(restored.max_tokens, config.max_tokens);
        assert_eq!(restored.max_iterations, config.max_iterations);
        assert_eq!(restored.tools.auto_detect, config.tools.auto_detect);
        assert_eq!(restored.display.theme, config.display.theme);
        assert_eq!(restored.context.compression, config.context.compression);
    }

    #[test]
    fn test_config_yaml_roundtrip_with_gateway() {
        let mut config = AppConfig::default();
        config.gateway = Some(GatewayConfig {
            telegram: Some(PlatformConfig {
                enabled: true,
                token: Some("tg-secret-token".to_string()),
            }),
            discord: None,
            slack: None,
            whatsapp: None,
        });
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(restored.gateway.is_some());
        let gw = restored.gateway.unwrap();
        let tg = gw.telegram.unwrap();
        assert!(tg.enabled);
        assert_eq!(tg.token, Some("tg-secret-token".to_string()));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");

        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        fs::write(&config_path, &yaml).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&content).unwrap();
        assert_eq!(restored.model.kind, config.model.kind);
        assert_eq!(restored.temperature, config.temperature);
    }

    #[test]
    fn test_minimal_config_deserialize() {
        // User's actual config only has model settings.
        // All other fields should use defaults.
        let yaml = r#"model:
  kind: Custom
  base_url: "http://10.0.0.177:8000/v1"
  model: "qwen35-local"
  default_model: "qwen35-local"
  api_key: "sk-local"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.kind, oben_models::ProviderKind::Custom);
        assert_eq!(config.model.model, "qwen35-local");
        assert_eq!(
            config.model.base_url.as_deref(),
            Some("http://10.0.0.177:8000/v1")
        );
        assert_eq!(config.tools.enabled, Vec::<String>::new());
        assert!(config.tools.auto_detect);
        assert_eq!(config.display.theme, "dark");
        assert_eq!(config.context.compression, "summary");
    }

    #[test]
    fn test_providers_field_serializes_empty() {
        let config = AppConfig::default();
        assert!(config.providers.is_empty());
        assert!(config.custom_providers.is_empty());
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(restored.providers.is_empty());
        assert!(restored.custom_providers.is_empty());
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|d| d.join("oben"))
            .unwrap_or_else(|| PathBuf::from("~/.config/obenalien"))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.yaml")
    }

    /// Read from `~/.obenalien/config.yaml` (legacy/standard path).
    pub fn config_dir_legacy() -> PathBuf {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("~"));
        home.join(".config/obenalien")
    }

    /// Read from `~/.config/obenalien/config.yaml` (legacy/standard path).
    pub fn config_path_legacy() -> PathBuf {
        Self::config_dir_legacy().join("config.yaml")
    }

    /// Load config from `~/.config/obenalien/config.yaml`.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path_legacy();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut config: Self = serde_yaml::from_str(&content)?;

        if let Some(t) = config.temperature {
            config.model.temperature = Some(t);
        }
        if let Some(m) = config.max_tokens {
            config.model.max_tokens = Some(m);
        }

        if config
            .model
            .api_key
            .as_ref()
            .map_or(true, |v| v.trim().is_empty())
        {
            if let Some(key) =
                oben_models::provider_registry::resolve_api_key_from_env(config.model.kind.as_str())
            {
                config.model.api_key = Some(key);
            }
        }

        if config
            .model
            .base_url
            .as_ref()
            .map_or(true, |v| v.trim().is_empty())
        {
            if let Some(env_var) = config.model.kind.base_url_env_var() {
                if let Ok(val) = std::env::var(env_var) {
                    let val = val.trim().to_string();
                    if !val.is_empty() {
                        config.model.base_url = Some(val);
                    }
                }
            }
        }

        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir_legacy();
        std::fs::create_dir_all(&dir)?;
        let content = serde_yaml::to_string(self)?;
        std::fs::write(dir.join("config.yaml"), content)?;
        Ok(())
    }
}
