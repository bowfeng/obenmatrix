use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_config::config::AppConfig;
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Image analysis — downloads images and calls vision APIs
// ---------------------------------------------------------------------------

/// SSRF guard — block private/internal URLs.
fn is_safe_url(url: &str) -> bool {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return false;
    }
    let host = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .and_then(|u| u.split('/').next())
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    if host == "localhost"
        || host == "127.0.0.1"
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".corp")
        || host.ends_with(".home")
    {
        return false;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 {
        let first = parts[0].parse::<u8>().unwrap_or(0);
        let second = parts[1].parse::<u8>().unwrap_or(0);
        if first == 10 {
            return false;
        }
        if first == 172 && second >= 16 && second <= 31 {
            return false;
        }
        if first == 192 && second == 168 {
            return false;
        }
        if first == 127 {
            return false;
        }
        if first == 169 && second == 254 {
            return false;
        }
    }
    true
}

/// Check if a string is a data: URL.
fn is_data_url(url: &str) -> bool {
    url.trim().starts_with("data:image/")
}

/// Parse a data: URL into (mime_type, base64_data).
fn parse_data_url(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if !url.starts_with("data:image/") {
        return None;
    }
    let comma = url.find(',')?;
    let metadata = &url[..comma];
    let data = &url[comma + 1..];
    // metadata = "data:image/png" or "data:image/png;base64"
    let extension = metadata
        .strip_prefix("data:image/")
        .unwrap_or(metadata)
        .split(';')
        .next()
        .unwrap_or("");
    let mime = format!("image/{}", extension.trim());
    Some((mime, data.to_string()))
}

/// Detect MIME type from image bytes (magic number detection).
fn detect_mime(image_data: &[u8]) -> &str {
    if image_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if image_data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if image_data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        "image/gif"
    } else if image_data.starts_with(b"RIFF") && &image_data[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

/// Download image from URL to bytes.
async fn download_image(url: &str) -> Result<(Vec<u8>, String), String> {
    let client = Client::new();
    let response = match client
        .get(url)
        .header("User-Agent", "ObenAgent/1.0 (vision tool)")
        .header("Accept", "image/*,*/*;q=0.8")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return Err(format!("Failed to download image: {}", e)),
    };
    if !response.status().is_success() {
        return Err(format!("HTTP {} downloading image", response.status()));
    }
    let bytes = match response.bytes().await {
        Ok(d) => d,
        Err(e) => return Err(format!("Failed to read image data: {}", e)),
    };
    let mime = detect_mime(&bytes);
    Ok((bytes.to_vec(), mime.to_string()))
}

/// OpenAI API response for chat completions.
#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    content: String,
}

/// Call OpenAI-compatible vision API.
async fn analyze_with_openai(
    image_data: &[u8],
    mime: &str,
    prompt: &str,
    base_url: Option<&str>,
    api_key: &str,
    model: &str,
    max_tokens: usize,
) -> Result<String, String> {
    let base_url = base_url.unwrap_or("https://api.openai.com/v1");
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let base64_data = base64::engine::general_purpose::STANDARD.encode(image_data);
    let data_url = format!("data:{};base64,{}", mime, base64_data);

    let client = Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": prompt},
                        {"type": "image_url", "image_url": {"url": data_url, "detail": "high"}}
                    ]
                }
            ],
            "max_tokens": max_tokens,
            "temperature": 0.1
        }))
        .send()
        .await
        .map_err(|e| format!("API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(200).collect();
        return Err(format!("API error {} ({}): {}", status, model, preview));
    }

    let parsed: OpenAIResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(parsed
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_else(|| "No analysis returned".to_string()))
}

/// Anthropic API response for messages.
#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    text: Option<String>,
}

/// Call Anthropic vision API.
async fn analyze_with_anthropic(
    image_data: &[u8],
    mime: &str,
    prompt: &str,
    api_key: &str,
    model: &str,
    max_tokens: usize,
) -> Result<String, String> {
    let url = "https://api.anthropic.com/v1/messages";
    let base64_data = base64::engine::general_purpose::STANDARD.encode(image_data);

    let client = Client::new();
    let resp = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": prompt},
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mime,
                                "data": base64_data
                            }
                        }
                    ]
                }
            ]
        }))
        .send()
        .await
        .map_err(|e| format!("API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(200).collect();
        return Err(format!("API error {} ({}): {}", status, model, preview));
    }

    let parsed: AnthropicResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(parsed
        .content
        .iter()
        .filter_map(|b| b.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}

/// Analyze image from data URL (base64 payload).
async fn analyze_from_data_url(data_url: &str, prompt: &str) -> Result<String, String> {
    let (mime, b64_data) =
        parse_data_url(data_url).ok_or_else(|| "Invalid data URL format".to_string())?;

    let image_data = base64::engine::general_purpose::STANDARD
        .decode(&b64_data)
        .map_err(|e| format!("Failed to decode base64 data: {}", e))?;

    if image_data.is_empty() {
        return Err("Image data is empty".to_string());
    }

    // Detect MIME type from magic bytes if not specified in data URL
    let mime = if mime.is_empty() {
        detect_mime(&image_data).to_string()
    } else {
        mime
    };

    let config = AppConfig::load(None).unwrap_or_default();
    let vision = config.vision;
    let model = vision.model.as_deref().unwrap_or("gpt-4o");
    let max_tokens = vision.max_tokens.unwrap_or(1024);

    let api_key = match (vision.api_key.as_deref(), std::env::var("VISION_API_KEY").ok()) {
        (Some(k), _) if !k.is_empty() => k.to_string(),
        (_, Some(k)) => k,
        _ => return Err("Vision API key not configured. Set vision.api_key in config.yaml or VISION_API_KEY env var.".to_string()),
    };

    match vision.provider.trim() {
        "anthropic" => {
            let anth_model = if model.contains("claude") {
                model.to_string()
            } else {
                "claude-sonnet-4-20250514".to_string()
            };
            analyze_with_anthropic(
                &image_data,
                &mime,
                prompt,
                &api_key,
                &anth_model,
                max_tokens,
            )
            .await
        }
        _ => {
            analyze_with_openai(
                &image_data,
                &mime,
                prompt,
                vision.base_url.as_deref(),
                &api_key,
                model,
                max_tokens,
            )
            .await
        }
    }
}

/// Analyze image using configured vision API.
async fn analyze_image(image_url: &str, prompt: &str) -> Result<String, String> {
    // Handle data URLs directly (base64 encoded images)
    if is_data_url(image_url) {
        return analyze_from_data_url(image_url, prompt).await;
    }

    let config = AppConfig::load(None).unwrap_or_default();
    let vision = config.vision;

    let provider = vision.provider.trim();
    let model = vision.model.as_deref().unwrap_or("gpt-4o");
    let max_tokens = vision.max_tokens.unwrap_or(1024);

    // Get API key: config > environment
    let api_key = match (vision.api_key.as_deref(), std::env::var("VISION_API_KEY").ok()) {
        (Some(k), _) if !k.is_empty() => k.to_string(),
        (_, Some(k)) => k,
        _ => return Err("Vision API key not configured. Set vision.api_key in config.yaml or VISION_API_KEY env var.".to_string()),
    };

    // Download image
    let (image_data, mime) = download_image(image_url).await?;
    if image_data.is_empty() {
        return Err("Downloaded image is empty".to_string());
    }

    // Call the appropriate API
    match provider {
        "anthropic" => {
            let anth_model = if model.contains("claude") {
                model.to_string()
            } else {
                "claude-sonnet-4-20250514".to_string()
            };
            analyze_with_anthropic(
                &image_data,
                &mime,
                prompt,
                &api_key,
                &anth_model,
                max_tokens,
            )
            .await
        }
        _ => {
            // Default: OpenAI-compatible API
            analyze_with_openai(
                &image_data,
                &mime,
                prompt,
                vision.base_url.as_deref(),
                &api_key,
                model,
                max_tokens,
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_vision_analyze_tool_def() -> ToolMeta {
    ToolMeta {
        name: "vision_analyze".into(),
        description: "Analyze image or diagram files (PNG, JPG, SVG, JSON diff, etc.) to extract text, structure, charts, code blocks, and diagrams".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("image_url", "The URL to an image file to analyze (HTTP URL or base64 data URL)", "string"),
            ToolParameter::optional("prompt", "A specific question to answer about the content of the image. If not provided, defaults to an all-encompassing analysis.", "string"),
        ]),
    }
}// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct VisionAnalyzeTool;

/// Analyze an image using configured vision API.
async fn execute_vision<'a>(call: &ToolCall<'a>) -> anyhow::Result<ToolResult> {
    let prompt = call.optional_str("prompt").unwrap_or("Describe this image in detail.");
    let image_url = call.required_str("image_url")?;

    // Handle data URLs directly (base64 encoded images)
    if is_data_url(image_url) {
        match analyze_from_data_url(image_url, prompt).await {
            Ok(analysis) => Ok(ToolResult {
                call_id: call.call_id.clone(),
                output: analysis,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some(format!("Analysis failed: {}", e)),
            }),
        }
    }
    // SSRF protection (only for http/https URLs)
    else if !is_safe_url(image_url) {
        Ok(ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some("Blocked: URL targets a private or internal network address".to_string()),
        })
    }
    // Download and analyze
    else {
        match analyze_image(image_url, prompt).await {
            Ok(analysis) => Ok(ToolResult {
                call_id: call.call_id.clone(),
                output: analysis,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some(format!("Analysis failed: {}", e)),
            }),
        }
    }
}

#[async_trait::async_trait]
impl Tool for VisionAnalyzeTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }
    fn description(&self) -> &str {
        "Analyze images from URLs or base64 data URLs using vision AI"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_vision(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
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
    let tool = Box::new(VisionAnalyzeTool);
    registry.register_with_def(tool, make_vision_analyze_tool_def());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::ToolParameters;
    use serde_json::json;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        register(&mut registry);
        registry
    }

    #[test]
    fn test_safe_urls() {
        assert!(is_safe_url("https://example.com/image.jpg"));
        assert!(is_safe_url("http://example.org/photo.png"));
        assert!(!is_safe_url("http://192.168.1.1/image.jpg"));
        assert!(!is_safe_url("https://10.0.0.1/photo.png"));
        assert!(!is_safe_url("http://127.0.0.1:8080/image.jpg"));
        assert!(!is_safe_url("http://localhost:3000/image.jpg"));
    }

    #[test]
    fn test_detect_mime_png() {
        let data: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_mime(&data), "image/png");
    }

    #[test]
    fn test_detect_mime_jpeg() {
        let data: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_mime(&data), "image/jpeg");
    }

    #[test]
    fn test_detect_mime_gif() {
        let data: Vec<u8> = vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_mime(&data), "image/gif");
    }

    #[test]
    fn test_detect_mime_webp() {
        let data: Vec<u8> = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        assert_eq!(detect_mime(&data), "image/webp");
    }

    #[tokio::test]
    async fn blocks_localhost() {
        let registry = make_registry();
        let result = registry
            .execute(
                "vision_analyze",
                &json!({
                    "image_url": "http://localhost:3000/test.jpg",
                    "prompt": "What is this?",
                    "call_id": "test-1",
                }),
            )
            .await;
        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Blocked"));
    }

    #[tokio::test]
    async fn blocks_private_ip() {
        let registry = make_registry();
        let result = registry
            .execute(
                "vision_analyze",
                &json!({
                    "image_url": "http://192.168.1.1/image.jpg",
                    "call_id": "test-2",
                }),
            )
            .await;
        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Blocked"));
    }

    #[tokio::test]
    async fn handles_missing_image_url_arg() {
        let registry = make_registry();
        let result = registry
            .execute(
                "vision_analyze",
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
            .contains("Missing required argument: 'image_url'"));
    }

    #[tokio::test]
    async fn handles_invalid_url() {
        let registry = make_registry();
        let result = registry
            .execute(
                "vision_analyze",
                &json!({
                    "image_url": "not-a-url",
                    "prompt": "Analyze",
                    "call_id": "test-4",
                }),
            )
            .await;
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn returns_missing_key_error_without_config() {
        // Without VISION_API_KEY set, the tool should return a helpful error
        let registry = make_registry();
        let result = registry
            .execute(
                "vision_analyze",
                &json!({
                    "image_url": "https://example.com/test.jpg",
                    "call_id": "test-5",
                }),
            )
            .await;
        assert!(result.error.is_some());
        let error_msg = result.error.as_ref().unwrap();
        // Error may be about missing key OR download failure (if network blocks example.com)
        assert!(
            error_msg.contains("API key")
                || error_msg.contains("API error")
                || error_msg.contains("download")
                || error_msg.contains("HTTP"),
            "Expected API key error or download error, got: {}",
            error_msg
        );
    }

    #[test]
    fn test_parse_data_url() {
        let (mime, b64) = parse_data_url("data:image/png;base64,iVBORw0KGgo=").unwrap();
        // MIME should be "image/png" (normalized)
        assert_eq!(mime, "image/png");
        assert_eq!(b64, "iVBORw0KGgo=");

        let (mime, b64) = parse_data_url("data:image/jpeg;base64,/9j/4AAQ").unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(b64, "/9j/4AAQ");

        assert!(parse_data_url("https://example.com/test.jpg").is_none());
        assert!(parse_data_url("data:text/plain,hello").is_none());
    }

    #[test]
    fn test_is_data_url() {
        assert!(is_data_url("data:image/png;base64,abc"));
        assert!(is_data_url("data:image/jpeg;base64,xyz"));
        assert!(!is_data_url("https://example.com/test.jpg"));
        assert!(!is_data_url("http://localhost:8080/test.jpg"));
    }

    #[test]
    fn test_tool_definition_has_parameters() {
        let registry = make_registry();
        let tools = registry.list_tools();
        // Find vision_analyze tool
        let vision_tool = tools
            .iter()
            .find(|t| t.name == "vision_analyze")
            .expect("vision_analyze should be in registry");
        // Check parameters are present (not empty)
        match &vision_tool.parameters {
            ToolParameters::Flat(params) => {
                assert!(!params.is_empty(), "vision_analyze should have parameters");
                let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                assert!(
                    param_names.contains(&"image_url"),
                    "Should have image_url parameter"
                );
                assert!(
                    param_names.contains(&"prompt"),
                    "Should have prompt parameter"
                );
                // image_url should be required
                let image_param = params.iter().find(|p| p.name == "image_url").unwrap();
                assert!(image_param.required, "image_url should be required");
            }
            _ => panic!("Expected Flat parameters"),
        }
    }
}
