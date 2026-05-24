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
///
/// Providers implement this to expose image generation capabilities
/// (e.g., DALL-E, Stable Diffusion, Midjourney).
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
///
/// Providers implement this to expose web search capabilities
/// (e.g., Tavily, SerpAPI, Google Custom Search).
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
///
/// Providers implement this to expose headless browser capabilities
/// (e.g., Puppeteer, Playwright, Chrome headless).
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
///
/// Providers implement this to compress long conversation contexts
/// while preserving important information (e.g., LLM-based summarization,
/// keyword extraction, relevance scoring).
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
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImageGen => write!(f, "image_gen"),
            Self::VideoGen => write!(f, "video_gen"),
            Self::WebSearch => write!(f, "web_search"),
            Self::Browser => write!(f, "browser"),
            Self::ContextEngine => write!(f, "context_engine"),
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
            _ => Err(anyhow::anyhow!("Unknown provider kind: '{}'", s)),
        }
    }
}

/// Check if a provider kind string is valid.
pub fn is_valid_provider_kind(s: &str) -> bool {
    matches!(s, "image_gen" | "video_gen" | "web_search" | "browser" | "context_engine")
}

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
        assert!(ProviderKind::from_str("invalid").is_err());
    }

    #[test]
    fn test_provider_kind_display() {
        /// given: ProviderKind variants
        /// when: Display::fmt is called
        /// then: returns snake_case string
        assert_eq!(ProviderKind::ImageGen.to_string(), "image_gen");
        assert_eq!(ProviderKind::WebSearch.to_string(), "web_search");
    }

    #[test]
    fn test_is_valid_provider_kind() {
        /// given: various provider kind strings
        /// when: is_valid_provider_kind() is called
        /// then: returns true only for valid kinds
        assert!(is_valid_provider_kind("image_gen"));
        assert!(is_valid_provider_kind("web_search"));
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
}
