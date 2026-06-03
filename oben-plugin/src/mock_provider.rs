//! Mock provider implementations for testing provider integration.
//!
//! These demonstrate the full trait impl wiring for provider registration,
//! retrieval, and config-driven selection.

use anyhow::Result;
use async_trait::async_trait;

use crate::provider::{
    BrowserOutput, BrowserProvider, ContextEngine, ContextEngineOutput, ImageGenOutput,
    ImageGenProvider, ModelProvider, ProviderProfile, SearchResult, VideoGenOutput,
    VideoGenProvider, WebSearchOutput, WebSearchProvider,
};

// ────────────────────────────────────────────────────────────────────────
// MockImageGenProvider — Demonstrates ImageGenProvider trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock image generation provider that returns deterministic results.
/// Useful for testing provider registration and config-driven selection.
pub struct MockImageGenProvider {
    name_val: String,
    available: bool,
}

impl MockImageGenProvider {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait]
impl ImageGenProvider for MockImageGenProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock image generation provider for testing".to_string(),
            models: vec!["mock-v1".into(), "mock-v2".into()],
            default_model: Some("mock-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    async fn generate(
        &self,
        prompt: &str,
        _model: Option<&str>,
        _width: Option<i32>,
        _height: Option<i32>,
        _n: Option<u8>,
    ) -> Result<ImageGenOutput> {
        Ok(ImageGenOutput {
            url: Some(format!(
                "https://mock.provider/{}.png",
                prompt.chars().take(8).collect::<String>()
            )),
            data: None,
            mime_type: "image/png".to_string(),
            width: 512,
            height: 512,
            error: None,
        })
    }

    fn get_setup_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "api_key": {
                    "type": "string",
                    "description": "API key for mock provider",
                    "sensitive": true
                }
            }
        }))
    }
}

// ────────────────────────────────────────────────────────────────────────
// MockWebSearchProvider — Demonstrates WebSearchProvider trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock web search provider that returns deterministic results.
pub struct MockWebSearchProvider {
    name_val: String,
    available: bool,
}

impl MockWebSearchProvider {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait]
impl WebSearchProvider for MockWebSearchProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock web search provider for testing".to_string(),
            models: vec!["mock-search-v1".into()],
            default_model: Some("mock-search-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    async fn search(
        &self,
        query: &str,
        max_results: Option<u8>,
        _depth: Option<&str>,
    ) -> Result<WebSearchOutput> {
        let n = max_results.unwrap_or(3) as usize;
        let results = (0..n)
            .map(|i| SearchResult {
                title: format!("Result {} for {}", i + 1, query),
                url: format!("https://mock.search/result/{}", i),
                snippet: format!("Mock snippet for query: {}", query),
                score: 0.9 - (i as f32 * 0.1),
            })
            .collect();

        Ok(WebSearchOutput {
            results,
            num_results: n,
            query: query.to_string(),
            error: None,
        })
    }
}

// ────────────────────────────────────────────────────────────────────────
// MockBrowserProvider — Demonstrates BrowserProvider trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock browser provider that returns deterministic results.
pub struct MockBrowserProvider {
    name_val: String,
    available: bool,
}

impl MockBrowserProvider {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait]
impl BrowserProvider for MockBrowserProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock browser provider for testing".to_string(),
            models: vec!["mock-browser-v1".into()],
            default_model: Some("mock-browser-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    async fn browse(&self, url: &str, _wait_for_selector: Option<&str>) -> Result<BrowserOutput> {
        Ok(BrowserOutput {
            title: format!("Page: {}", url),
            content: format!("<html><body>Mock content for {}", url),
            final_url: url.to_string(),
            status_code: 200,
            error: None,
        })
    }
}

// ────────────────────────────────────────────────────────────────────────
// MockContextEngine — Demonstrates ContextEngine trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock context compression engine.
pub struct MockContextEngine {
    name_val: String,
    available: bool,
}

impl MockContextEngine {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait::async_trait]
impl ContextEngine for MockContextEngine {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock context compression engine for testing".to_string(),
            models: vec!["mock-engine-v1".into()],
            default_model: Some("mock-engine-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    fn compress(
        &self,
        messages: &[serde_json::Value],
        _max_tokens: Option<usize>,
        _model: Option<&str>,
    ) -> Result<ContextEngineOutput> {
        Ok(ContextEngineOutput {
            messages: messages.to_vec(),
            summary: format!("Compressed {} messages", messages.len()),
            input_count: messages.len(),
            output_count: messages.len(),
            tokens_saved: 0,
        })
    }
}

// ────────────────────────────────────────────────────────────────────────
// MockVideoGenProvider — Demonstrates VideoGenProvider trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock video generation provider that returns deterministic results.
pub struct MockVideoGenProvider {
    name_val: String,
    available: bool,
}

impl MockVideoGenProvider {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait]
impl VideoGenProvider for MockVideoGenProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock video generation provider for testing".to_string(),
            models: vec!["mock-video-v1".into()],
            default_model: Some("mock-video-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    async fn generate_video(
        &self,
        prompt: &str,
        _model: Option<&str>,
        _duration: Option<i32>,
        _format: Option<&str>,
    ) -> Result<VideoGenOutput> {
        Ok(VideoGenOutput {
            url: Some(format!(
                "https://mock.provider/{}.mp4",
                prompt.chars().take(8).collect::<String>()
            )),
            data: None,
            mime_type: "video/mp4".to_string(),
            duration: 30,
            format: "mp4".to_string(),
            error: None,
        })
    }
}

// ────────────────────────────────────────────────────────────────────────
// MockModelProvider — Demonstrates ModelProvider trait impl
// ────────────────────────────────────────────────────────────────────────

/// A mock model provider.
pub struct MockModelProvider {
    name_val: String,
    available: bool,
}

impl MockModelProvider {
    pub fn new(name: &str, available: bool) -> Self {
        Self {
            name_val: name.to_string(),
            available,
        }
    }
}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn list_models(&self) -> Vec<ProviderProfile> {
        vec![ProviderProfile {
            name: self.name_val.clone(),
            display_name: format!("Mock {}", self.name_val),
            description: "Mock model provider for testing".to_string(),
            models: vec!["mock-model-v1".into()],
            default_model: Some("mock-model-v1".into()),
            requires_env: vec![],
            is_available: self.available,
            config: None,
        }]
    }

    async fn chat_completion(
        &self,
        _messages: &[serde_json::Value],
        _model: Option<&str>,
        _temperature: Option<f64>,
        _max_tokens: Option<usize>,
    ) -> Result<crate::provider::ChatCompletionOutput> {
        Ok(crate::provider::ChatCompletionOutput {
            content: Some("Mock response".to_string()),
            tool_calls: vec![],
            usage: Some(crate::provider::CompletionUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_image_gen_provider() {
        /// given: a mock image gen provider
        /// when: basic methods are called
        /// then: returns expected values
        let provider = MockImageGenProvider::new("mock-test", true);
        assert_eq!(provider.name(), "mock-test");
        assert!(provider.is_available());

        let models = provider.list_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].models, vec!["mock-v1", "mock-v2"]);
        assert_eq!(provider.default_model(), Some("mock-v1".to_string()));
    }

    #[test]
    fn test_mock_image_gen_generate() {
        /// given: a mock image gen provider
        /// when: generate() is called
        /// then: returns deterministic image output
        let rt = tokio::runtime::Runtime::new().unwrap();
        let provider = MockImageGenProvider::new("mock-gen", true);

        rt.block_on(async {
            let result = provider
                .generate("test prompt", None, None, None, None)
                .await
                .unwrap();
            assert!(result.url.is_some());
            assert_eq!(result.mime_type, "image/png");
            assert_eq!(result.width, 512);
            assert_eq!(result.height, 512);
            assert!(result.error.is_none());
        });
    }

    #[test]
    fn test_mock_web_search_provider() {
        /// given: a mock web search provider
        /// when: search() is called
        /// then: returns expected search results
        let rt = tokio::runtime::Runtime::new().unwrap();
        let provider = MockWebSearchProvider::new("mock-search", true);

        rt.block_on(async {
            let result = provider.search("test query", Some(2), None).await.unwrap();
            assert_eq!(result.num_results, 2);
            assert_eq!(result.results.len(), 2);
            assert_eq!(result.query, "test query");
            assert!(result.error.is_none());
            assert_eq!(result.results[0].title, "Result 1 for test query");
        });
    }

    #[test]
    fn test_mock_browser_provider() {
        /// given: a mock browser provider
        /// when: browse() is called
        /// then: returns expected page output
        let rt = tokio::runtime::Runtime::new().unwrap();
        let provider = MockBrowserProvider::new("mock-browser", true);

        rt.block_on(async {
            let result = provider.browse("https://example.com", None).await.unwrap();
            assert_eq!(result.status_code, 200);
            assert_eq!(result.title, "Page: https://example.com");
            assert!(result.content.contains("Mock content"));
            assert!(result.error.is_none());
        });
    }

    #[test]
    fn test_mock_context_engine() {
        /// given: a mock context engine
        /// when: compress() is called
        /// then: returns compressed output
        let provider = MockContextEngine::new("mock-engine", true);
        let messages = vec![
            serde_json::json!({"role": "user", "content": "Hello"}),
            serde_json::json!({"role": "assistant", "content": "Hi there"}),
        ];

        let result = provider.compress(&messages, None, None).unwrap();
        assert_eq!(result.input_count, 2);
        assert_eq!(result.output_count, 2);
        assert_eq!(result.summary, "Compressed 2 messages");
    }

    #[test]
    fn test_mock_model_provider() {
        /// given: a mock model provider
        /// when: chat_completion() is called
        /// then: returns expected completion output
        let rt = tokio::runtime::Runtime::new().unwrap();
        let provider = MockModelProvider::new("mock-model", true);

        rt.block_on(async {
            let result = provider
                .chat_completion(&[], None, None, None)
                .await
                .unwrap();
            assert_eq!(result.content, Some("Mock response".to_string()));
            assert!(result.usage.is_some());
            assert_eq!(result.usage.as_ref().unwrap().total_tokens, 30);
        });
    }

    #[test]
    fn test_provider_availability() {
        /// given: a provider with available=false
        /// when: is_available() is called
        /// then: returns false
        let provider = MockImageGenProvider::new("unavailable", false);
        assert!(!provider.is_available());
        assert_eq!(provider.list_models()[0].is_available, false);
    }

    #[test]
    fn test_get_setup_schema() {
        /// given: a mock image gen provider
        /// when: get_setup_schema() is called
        /// then: returns a JSON schema
        let provider = MockImageGenProvider::new("mock", true);
        let schema = provider.get_setup_schema();
        assert!(schema.is_some());
        let schema = schema.unwrap();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
    }
}
