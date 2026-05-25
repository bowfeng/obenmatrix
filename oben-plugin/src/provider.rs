//! Provider traits — pluggable backend abstraction.
//!
//! Maps to Hermes' provider system where plugins register alternative
//! backends for image_gen, video_gen, web_search, browser, memory,
//! and context_engine.
//!
//! Each provider type has its own trait and registry. A provider is
//! a plugin that implements a specific capability, and can be selected
//! via config (e.g., `image_gen.provider = "openai"`).

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// ProviderProfile — capability metadata for a provider
// ---------------------------------------------------------------------------

/// Metadata about a provider's capabilities (models, endpoints, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    /// Unique provider name (e.g., "openai", "dall-e-3", "tavily").
    pub name: String,

    /// Human-readable display name.
    pub display_name: String,

    /// Provider description.
    pub description: String,

    /// Available models (empty means "use the provider default").
    pub models: Vec<String>,

    /// Default model when none specified.
    pub default_model: Option<String>,

    /// Environment variables this provider requires.
    pub requires_env: Vec<String>,

    /// Whether this provider is currently configured and ready.
    pub is_available: bool,

    /// Additional configuration options.
    pub config: Option<serde_json::Value>,
}

impl ProviderProfile {
    pub fn new(
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        models: Vec<String>,
        default_model: Option<String>,
        requires_env: Vec<String>,
        is_available: bool,
        config: Option<serde_json::Value>,
    ) -> Self {
        Self {
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            models,
            default_model,
            requires_env,
            is_available,
            config,
        }
    }

    /// Check if required environment variables are available.
    pub fn has_required_env(&self) -> bool {
        self.requires_env.iter().all(|env| std::env::var(env).is_ok())
    }

    /// List all models this provider supports, including default.
    pub fn all_models(&self) -> Vec<&str> {
        let mut models: Vec<&str> = self.models.iter().map(|s| s.as_str()).collect();
        if let Some(ref default) = self.default_model {
            if !models.contains(&default.as_str()) {
                models.push(default);
            }
        }
        models
    }

    /// Get the effective model to use.
    pub fn effective_model(&self, requested: Option<&str>) -> Option<String> {
        let model = requested
            .map(String::from)
            .or(self.default_model.clone());
        model.filter(|m| self.all_models().iter().any(|s| *s == m.as_str()))
    }
}

// ---------------------------------------------------------------------------
// ImageGenProvider — image generation backend trait
// ---------------------------------------------------------------------------

/// Output from an image generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenOutput {
    /// URL to the generated image.
    pub url: Option<String>,

    /// Base64-encoded image data (if URL not available).
    pub data: Option<String>,

    /// MIME type of the image (e.g., "image/png").
    pub mime_type: String,

    /// Width in pixels.
    pub width: i32,

    /// Height in pixels.
    pub height: i32,

    /// Error message if generation failed.
    pub error: Option<String>,
}

/// Trait for image generation providers.
#[async_trait]
pub trait ImageGenProvider: Send + Sync {
    /// Provider identifier (e.g., "openai", "stability").
    fn name(&self) -> &str;

    /// Whether this provider is configured and has valid credentials.
    fn is_available(&self) -> bool;

    /// List available models/backends this provider supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Generate an image from a text prompt.
    async fn generate(
        &self,
        prompt: &str,
        model: Option<&str>,
        width: Option<i32>,
        height: Option<i32>,
        n: Option<u8>,
    ) -> Result<ImageGenOutput>;

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// WebSearchProvider — web search/extract backend trait
// ---------------------------------------------------------------------------

/// A search result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Title of the result.
    pub title: String,

    /// URL of the result.
    pub url: String,

    /// Snippet/summary of the result.
    pub snippet: String,

    /// Score/relevance (0.0 to 1.0).
    pub score: f32,
}

/// Output from a web search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchOutput {
    /// Search results.
    pub results: Vec<SearchResult>,

    /// Number of results returned.
    pub num_results: usize,

    /// Search query that was executed.
    pub query: String,

    /// Error message if search failed.
    pub error: Option<String>,
}

/// Trait for web search providers.
#[async_trait]
pub trait WebSearchProvider: Send + Sync {
    /// Provider identifier (e.g., "tavily", "serpapi").
    fn name(&self) -> &str;

    /// Whether this provider is configured and has valid credentials.
    fn is_available(&self) -> bool;

    /// List available models/backends this provider supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Search the web for a query.
    async fn search(
        &self,
        query: &str,
        max_results: Option<u8>,
        depth: Option<&str>,
    ) -> Result<WebSearchOutput>;

    /// Fetch and extract content from a URL.
    async fn extract(&self, url: &str) -> Result<String> {
        let _ = url;
        Err(anyhow::anyhow!("extract not implemented"))
    }

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// BrowserProvider — cloud browser backend trait
// ---------------------------------------------------------------------------

/// Output from a browser visit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserOutput {
    /// Page title.
    pub title: String,

    /// Page content (HTML or extracted text).
    pub content: String,

    /// Final URL after redirects.
    pub final_url: String,

    /// HTTP status code.
    pub status_code: u16,

    /// Error message if browsing failed.
    pub error: Option<String>,
}

/// Trait for cloud browser providers.
#[async_trait]
pub trait BrowserProvider: Send + Sync {
    /// Provider identifier (e.g., "puppeteer", "playwright").
    fn name(&self) -> &str;

    /// Whether this provider is configured and has valid credentials.
    fn is_available(&self) -> bool;

    /// List available models/backends this provider supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Visit a URL and return page content.
    async fn browse(&self, url: &str, wait_for_selector: Option<&str>) -> Result<BrowserOutput>;

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// VideoGenProvider — video generation backend trait
// ---------------------------------------------------------------------------

/// Output from a video generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenOutput {
    /// URL to the generated video.
    pub url: Option<String>,

    /// Base64-encoded video data (if URL not available).
    pub data: Option<String>,

    /// MIME type of the video (e.g., "video/mp4").
    pub mime_type: String,

    /// Duration in seconds.
    pub duration: i32,

    /// Video format (e.g., "mp4", "webm").
    pub format: String,

    /// Error message if generation failed.
    pub error: Option<String>,
}

/// Trait for video generation providers.
#[async_trait]
pub trait VideoGenProvider: Send + Sync {
    /// Provider identifier (e.g., "sora", "runway").
    fn name(&self) -> &str;

    /// Whether this provider is configured and has valid credentials.
    fn is_available(&self) -> bool;

    /// List available models/backends this provider supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Generate a video from a text prompt.
    async fn generate_video(
        &self,
        prompt: &str,
        model: Option<&str>,
        duration: Option<i32>,
        format: Option<&str>,
    ) -> Result<VideoGenOutput>;

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// ContextEngine — context compression/summarization trait
// ---------------------------------------------------------------------------

/// Output from a context compression request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEngineOutput {
    /// Compressed messages.
    pub messages: Vec<serde_json::Value>,

    /// Summary of compressed context.
    pub summary: String,

    /// Number of messages before compression.
    pub input_count: usize,

    /// Number of messages after compression.
    pub output_count: usize,

    /// Estimated tokens saved.
    pub tokens_saved: usize,
}

/// Trait for context compression engines.
#[async_trait]
pub trait ContextEngine: Send + Sync {
    /// Engine identifier (e.g., "llm-summarize", "keyword-extract").
    fn name(&self) -> &str;

    /// Whether this engine is configured and ready.
    fn is_available(&self) -> bool;

    /// List available models/backends this engine supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Compress messages to fit within token limits.
    fn compress(
        &self,
        messages: &[serde_json::Value],
        max_tokens: Option<usize>,
        model: Option<&str>,
    ) -> Result<ContextEngineOutput>;

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// ModelProvider — custom model provider trait
// ---------------------------------------------------------------------------

/// Output from a model chat completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionOutput {
    /// The assistant's response content.
    pub content: Option<String>,
    /// Tool calls made by the model.
    pub tool_calls: Vec<ChatToolCall>,
    /// Token usage information.
    pub usage: Option<CompletionUsage>,
}

/// A tool call from a model completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    pub function: ToolCallFunction,
}

/// The function details of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Token usage from a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

/// Model provider — custom model provider with endpoint and auth.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Provider identifier (e.g., "local", "enterprise", "openrouter").
    fn name(&self) -> &str;

    /// Whether this provider is configured and has valid credentials.
    fn is_available(&self) -> bool;

    /// List available models/backends this provider supports.
    fn list_models(&self) -> Vec<ProviderProfile>;

    /// Get the default model name.
    fn default_model(&self) -> Option<String> {
        self.list_models().first().and_then(|m| m.default_model.clone())
    }

    /// Send a chat completion request.
    async fn chat_completion(
        &self,
        messages: &[serde_json::Value],
        model: Option<&str>,
        temperature: Option<f64>,
        max_tokens: Option<usize>,
    ) -> Result<ChatCompletionOutput>;

    /// Optional: get setup schema for UI configuration.
    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// ProviderKind — categorizes provider types
// ---------------------------------------------------------------------------

/// Categories of provider plugins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    /// Image generation (DALL-E, Stable Diffusion, etc.).
    #[serde(rename = "image_gen")]
    ImageGen,

    /// Video generation (Sora, Runway, etc.).
    #[serde(rename = "video_gen")]
    VideoGen,

    /// Web search (Tavily, SerpAPI, etc.).
    #[serde(rename = "web_search")]
    WebSearch,

    /// Browser automation (Puppeteer, Playwright, etc.).
    #[serde(rename = "browser")]
    Browser,

    /// Context compression engine (LLM summarizer, keyword extractor).
    #[serde(rename = "context_engine")]
    ContextEngine,

    /// Custom model provider (self-hosted, enterprise, etc.).
    #[serde(rename = "model_provider")]
    ModelProvider,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImageGen => write!(f, "image_gen"),
            Self::VideoGen => write!(f, "video_gen"),
            Self::WebSearch => write!(f, "web_search"),
            Self::Browser => write!(f, "browser"),
            Self::ContextEngine => write!(f, "context_engine"),
            Self::ModelProvider => write!(f, "model_provider"),
        }
    }
}

impl FromStr for ProviderKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "image_gen" => Ok(Self::ImageGen),
            "video_gen" => Ok(Self::VideoGen),
            "web_search" => Ok(Self::WebSearch),
            "browser" => Ok(Self::Browser),
            "context_engine" => Ok(Self::ContextEngine),
            "model_provider" => Ok(Self::ModelProvider),
            _ => Err(anyhow::anyhow!("Unknown provider kind: '{}'", s)),
        }
    }
}

/// Check if a provider kind string is valid.
pub fn is_valid_provider_kind(s: &str) -> bool {
    matches!(s, "image_gen" | "video_gen" | "web_search" | "browser" | "context_engine" | "model_provider")
}

// ---------------------------------------------------------------------------
// Provider Registries — typed wrappers around Vec<Box<dyn Trait>>
// ---------------------------------------------------------------------------

/// Registry for image generation providers (non-exclusive).
pub struct ImageGenRegistry {
    providers: Vec<Box<dyn ImageGenProvider + Send + Sync>>,
}

impl ImageGenRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a provider.
    pub fn register(&mut self, provider: Box<dyn ImageGenProvider + Send + Sync>) {
        self.providers.push(provider);
    }

    /// Get the first provider (default).
    pub fn get_default(&self) -> Option<&(dyn ImageGenProvider + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    /// Get a specific provider by name.
    pub fn get_by_name(&self, name: &str) -> Option<&(dyn ImageGenProvider + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    /// List all registered providers.
    pub fn list(&self) -> Vec<&(dyn ImageGenProvider + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    /// Provider count.
    pub fn len(&self) -> usize { self.providers.len() }
    /// Check if empty.
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    /// Clear all providers.
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for ImageGenRegistry {
    fn default() -> Self { Self::new() }
}

/// Registry for video generation providers (non-exclusive).
pub struct VideoGenRegistry {
    providers: Vec<Box<dyn VideoGenProvider + Send + Sync>>,
}

impl VideoGenRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn VideoGenProvider + Send + Sync>) {
        self.providers.push(provider);
    }

    pub fn get_default(&self) -> Option<&(dyn VideoGenProvider + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    pub fn get_by_name(&self, name: &str) -> Option<&(dyn VideoGenProvider + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<&(dyn VideoGenProvider + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    pub fn len(&self) -> usize { self.providers.len() }
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for VideoGenRegistry {
    fn default() -> Self { Self::new() }
}

/// Registry for web search providers (non-exclusive).
pub struct WebSearchRegistry {
    providers: Vec<Box<dyn WebSearchProvider + Send + Sync>>,
}

impl WebSearchRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn WebSearchProvider + Send + Sync>) {
        self.providers.push(provider);
    }

    pub fn get_default(&self) -> Option<&(dyn WebSearchProvider + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    pub fn get_by_name(&self, name: &str) -> Option<&(dyn WebSearchProvider + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<&(dyn WebSearchProvider + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    pub fn len(&self) -> usize { self.providers.len() }
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for WebSearchRegistry {
    fn default() -> Self { Self::new() }
}

/// Registry for browser providers (non-exclusive).
pub struct BrowserRegistry {
    providers: Vec<Box<dyn BrowserProvider + Send + Sync>>,
}

impl BrowserRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn BrowserProvider + Send + Sync>) {
        self.providers.push(provider);
    }

    pub fn get_default(&self) -> Option<&(dyn BrowserProvider + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    pub fn get_by_name(&self, name: &str) -> Option<&(dyn BrowserProvider + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<&(dyn BrowserProvider + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    pub fn len(&self) -> usize { self.providers.len() }
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for BrowserRegistry {
    fn default() -> Self { Self::new() }
}

/// Registry for context compression engines (exclusive — one at a time).
pub struct ContextEngineRegistry {
    providers: Vec<Box<dyn ContextEngine + Send + Sync>>,
}

impl ContextEngineRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a provider. In exclusive mode, replaces previous.
    pub fn register(&mut self, provider: Box<dyn ContextEngine + Send + Sync>) {
        self.providers.clear();
        self.providers.push(provider);
    }

    pub fn get_default(&self) -> Option<&(dyn ContextEngine + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    pub fn get_by_name(&self, name: &str) -> Option<&(dyn ContextEngine + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<&(dyn ContextEngine + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    pub fn len(&self) -> usize { self.providers.len() }
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for ContextEngineRegistry {
    fn default() -> Self { Self::new() }
}

/// Registry for model providers (non-exclusive).
pub struct ModelProviderRegistry {
    providers: Vec<Box<dyn ModelProvider + Send + Sync>>,
}

impl ModelProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn ModelProvider + Send + Sync>) {
        self.providers.push(provider);
    }

    pub fn get_default(&self) -> Option<&(dyn ModelProvider + Send + Sync)> {
        self.providers.first().map(|p| p.as_ref())
    }

    pub fn get_by_name(&self, name: &str) -> Option<&(dyn ModelProvider + Send + Sync)> {
        self.providers.iter().find(|p| p.name() == name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<&(dyn ModelProvider + Send + Sync)> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    pub fn len(&self) -> usize { self.providers.len() }
    pub fn is_empty(&self) -> bool { self.providers.is_empty() }
    pub fn clear(&mut self) { self.providers.clear(); }
}

impl Default for ModelProviderRegistry {
    fn default() -> Self { Self::new() }
}

/// Marker types for typed provider registries.
pub struct ImageGenMarker;
pub struct VideoGenMarker;
pub struct WebSearchMarker;
pub struct BrowserMarker;
pub struct ContextEngineMarker;
pub struct ModelProviderMarker;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_kind_from_str() {
        /// given: provider kind strings
        /// when: ProviderKind::from_str() is called
        /// then: returns correct ProviderKind variants
        assert_eq!(ProviderKind::from_str("image_gen").unwrap(), ProviderKind::ImageGen);
        assert_eq!(ProviderKind::from_str("web_search").unwrap(), ProviderKind::WebSearch);
        assert_eq!(ProviderKind::from_str("browser").unwrap(), ProviderKind::Browser);
        assert_eq!(ProviderKind::from_str("context_engine").unwrap(), ProviderKind::ContextEngine);
        assert_eq!(ProviderKind::from_str("model_provider").unwrap(), ProviderKind::ModelProvider);
        assert!(ProviderKind::from_str("invalid").is_err());
    }

    #[test]
    fn test_provider_kind_display() {
        /// given: ProviderKind variants
        /// when: Display::fmt is called
        /// then: returns snake_case string
        assert_eq!(ProviderKind::ImageGen.to_string(), "image_gen");
        assert_eq!(ProviderKind::WebSearch.to_string(), "web_search");
        assert_eq!(ProviderKind::ModelProvider.to_string(), "model_provider");
    }

    #[test]
    fn test_is_valid_provider_kind() {
        /// given: various provider kind strings
        /// when: is_valid_provider_kind() is called
        /// then: returns true only for valid kinds
        assert!(is_valid_provider_kind("image_gen"));
        assert!(is_valid_provider_kind("web_search"));
        assert!(is_valid_provider_kind("model_provider"));
        assert!(!is_valid_provider_kind("invalid"));
    }

    #[test]
    fn test_provider_profile_new() {
        /// given: provider profile fields
        /// when: ProviderProfile::new() is called
        /// then: returns fully configured profile
        let profile = ProviderProfile::new(
            "test-provider",
            "Test Provider",
            "A test provider",
            vec!["v1".into(), "v2".into()],
            Some("v1".into()),
            vec!["API_KEY".into()],
            true,
            None,
        );

        assert_eq!(profile.name, "test-provider");
        assert_eq!(profile.display_name, "Test Provider");
        assert_eq!(profile.models, vec!["v1", "v2"]);
        assert_eq!(profile.default_model, Some("v1".into()));
        assert!(profile.is_available);
    }

    #[test]
    fn test_provider_profile_effective_model() {
        /// given: a provider profile with default model
        /// when: effective_model() is called
        /// then: returns requested model if valid, otherwise default
        let profile = ProviderProfile::new(
            "test",
            "Test",
            "Test",
            vec!["v1".into(), "v2".into()],
            Some("v1".into()),
            vec![],
            true,
            None,
        );

        // Requested model is valid
        assert_eq!(profile.effective_model(Some("v2")), Some("v2".to_string()));

        // Requested model is invalid
        assert_eq!(profile.effective_model(Some("invalid")), None);

        // No requested model
        assert_eq!(profile.effective_model(None), Some("v1".to_string()));
    }

    #[test]
    fn test_provider_profile_all_models() {
        /// given: a provider profile with models and default
        /// when: all_models() is called
        /// then: returns all models including default
        let profile = ProviderProfile::new(
            "test",
            "Test",
            "Test",
            vec!["v1".into()],
            Some("v1".into()),
            vec![],
            true,
            None,
        );

        let models = profile.all_models();
        assert_eq!(models, vec!["v1"]);

        // Default not in list
        let profile2 = ProviderProfile::new(
            "test2",
            "Test2",
            "Test2",
            vec!["v1".into()],
            Some("default".into()),
            vec![],
            true,
            None,
        );
        let models2 = profile2.all_models();
        assert_eq!(models2.len(), 2);
        assert!(models2.contains(&"v1"));
        assert!(models2.contains(&"default"));
    }

    // ── Registry Tests ─────────────────────────────────────────────

    #[test]
    fn test_image_gen_registry_register_get() {
        /// given: a new image gen registry
        /// when: register() + get_default() are called
        /// then: returns the registered provider
        struct MockImageGen;
        #[async_trait::async_trait]
        impl ImageGenProvider for MockImageGen {
            fn name(&self) -> &str { "mock" }
            fn is_available(&self) -> bool { true }
            fn list_models(&self) -> Vec<ProviderProfile> { vec![] }
            async fn generate(
                &self,
                _prompt: &str, _model: Option<&str>,
                _width: Option<i32>, _height: Option<i32>, _n: Option<u8>,
            ) -> Result<ImageGenOutput> {
                Ok(ImageGenOutput { url: None, data: None, mime_type: "image/png".into(), width: 512, height: 512, error: None })
            }
        }

        let mut reg = ImageGenRegistry::new();
        assert!(reg.is_empty());

        reg.register(Box::new(MockImageGen));
        assert_eq!(reg.len(), 1);

        let provider = reg.get_default().unwrap();
        assert_eq!(provider.name(), "mock");
        assert!(provider.is_available());
    }

    #[test]
    fn test_context_engine_registry_exclusive() {
        /// given: a context engine registry (exclusive)
        /// when: two providers are registered
        /// then: only the last one remains
        struct MockEngine {
            name_val: String,
        }
        impl MockEngine {
            fn new(name: &str) -> Self { Self { name_val: name.to_string() } }
        }

        #[async_trait::async_trait]
        impl ContextEngine for MockEngine {
            fn name(&self) -> &str { &self.name_val }
            fn is_available(&self) -> bool { true }
            fn list_models(&self) -> Vec<ProviderProfile> { vec![] }
            fn compress(
                &self,
                _messages: &[serde_json::Value], _max_tokens: Option<usize>, _model: Option<&str>,
            ) -> Result<ContextEngineOutput> {
                Ok(ContextEngineOutput {
                    messages: vec![], summary: "test".into(),
                    input_count: 0, output_count: 0, tokens_saved: 0,
                })
            }
        }

        let mut reg = ContextEngineRegistry::new();
        reg.register(Box::new(MockEngine::new("engine-a")));
        reg.register(Box::new(MockEngine::new("engine-b")));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get_default().unwrap().name(), "engine-b");
    }

    #[test]
    fn test_get_by_name() {
        /// given: a registry with multiple providers
        /// when: get_by_name() is called with each name
        /// then: returns the matching provider
        struct MockImageGen { name_val: String }
        #[async_trait::async_trait]
        impl ImageGenProvider for MockImageGen {
            fn name(&self) -> &str { &self.name_val }
            fn is_available(&self) -> bool { true }
            fn list_models(&self) -> Vec<ProviderProfile> { vec![] }
            async fn generate(&self, _prompt: &str, _model: Option<&str>, _width: Option<i32>, _height: Option<i32>, _n: Option<u8>) -> Result<ImageGenOutput> {
                Ok(ImageGenOutput { url: None, data: None, mime_type: "image/png".into(), width: 512, height: 512, error: None })
            }
        }

        let mut reg = ImageGenRegistry::new();
        reg.register(Box::new(MockImageGen { name_val: "a".into() }));
        reg.register(Box::new(MockImageGen { name_val: "b".into() }));

        assert!(reg.get_by_name("a").is_some());
        assert!(reg.get_by_name("b").is_some());
        assert!(reg.get_by_name("c").is_none());
    }

    // ── VideoGenProvider Tests ───────────────────────────────────────

    #[test]
    fn test_provider_kind_video_gen() {
        /// given: "video_gen" string
        /// when: ProviderKind::from_str() is called
        /// then: returns ProviderKind::VideoGen
        assert_eq!(ProviderKind::from_str("video_gen").unwrap(), ProviderKind::VideoGen);
        assert_eq!(ProviderKind::VideoGen.to_string(), "video_gen");
        assert!(is_valid_provider_kind("video_gen"));
    }

    #[test]
    fn test_video_gen_output_fields() {
        /// given: video generation output fields
        /// when: VideoGenOutput is constructed
        /// then: all fields are accessible
        let output = VideoGenOutput {
            url: Some("https://example.com/vid.mp4".into()),
            data: None,
            mime_type: "video/mp4".into(),
            duration: 30,
            format: "mp4".into(),
            error: None,
        };
        assert_eq!(output.url.unwrap(), "https://example.com/vid.mp4");
        assert_eq!(output.duration, 30);
        assert_eq!(output.format, "mp4");
    }

    #[test]
    fn test_video_gen_registry_uses_correct_trait() {
        /// given: a VideoGenProvider mock
        /// when: registered via VideoGenRegistry
        /// then: VideoGenRegistry wraps Box<dyn VideoGenProvider>, not ImageGenProvider
        struct MockVideoGen;
        #[async_trait::async_trait]
        impl VideoGenProvider for MockVideoGen {
            fn name(&self) -> &str { "mock-video" }
            fn is_available(&self) -> bool { true }
            fn list_models(&self) -> Vec<ProviderProfile> { vec![] }
            async fn generate_video(
                &self, _prompt: &str, _model: Option<&str>,
                _duration: Option<i32>, _format: Option<&str>,
            ) -> Result<VideoGenOutput> {
                Ok(VideoGenOutput {
                    url: Some("https://example.com/test.mp4".into()),
                    data: None, mime_type: "video/mp4".into(),
                    duration: 15, format: "mp4".into(), error: None,
                })
            }
        }

        let mut reg = VideoGenRegistry::new();
        assert!(reg.is_empty());

        reg.register(Box::new(MockVideoGen));
        assert_eq!(reg.len(), 1);

        let provider = reg.get_default().unwrap();
        assert_eq!(provider.name(), "mock-video");
        assert!(provider.is_available());
    }

    #[test]
    fn test_video_gen_registry_get_by_name() {
        /// given: a registry with two VideoGenProviders
        /// when: get_by_name() is called
        /// then: returns the matching provider
        struct MockVideoGen { name_val: String }
        #[async_trait::async_trait]
        impl VideoGenProvider for MockVideoGen {
            fn name(&self) -> &str { &self.name_val }
            fn is_available(&self) -> bool { true }
            fn list_models(&self) -> Vec<ProviderProfile> { vec![] }
            async fn generate_video(
                &self, _prompt: &str, _model: Option<&str>,
                _duration: Option<i32>, _format: Option<&str>,
            ) -> Result<VideoGenOutput> {
                Ok(VideoGenOutput {
                    url: Some("https://example.com/".into()),
                    data: None, mime_type: "video/mp4".into(),
                    duration: 0, format: "mp4".into(), error: None,
                })
            }
        }

        let mut reg = VideoGenRegistry::new();
        reg.register(Box::new(MockVideoGen { name_val: "runway".into() }));
        reg.register(Box::new(MockVideoGen { name_val: "sora".into() }));

        assert!(reg.get_by_name("runway").is_some());
        assert!(reg.get_by_name("sora").is_some());
        assert!(reg.get_by_name("pika").is_none());
    }
}
