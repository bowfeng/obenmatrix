use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// Constants
const OPENAI_BASE: &str = "https://api.openai.com/v1";
const RUNWAYML_BASE: &str = "https://api.runwayml.com/v1";
const STABLE_VIDEO_BASE: &str = "https://api.stability.ai/v2beta";
const SYNTHESIA_BASE: &str = "https://api.synthesia.io/v2";
const PIKA_BASE: &str = "https://api.pika.art/v1";

const ENV_OPENAI: &str = "OPENAI_API_KEY";
const ENV_RUNWAYML: &str = "RUNWAYML_API_KEY";
const ENV_STABLE_VIDEO: &str = "STABLE_VIDEO_API_KEY";
const ENV_SYNTHESIA: &str = "SYNTHESIA_API_KEY";
const ENV_PIKA: &str = "PIKA_API_KEY";

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_video_gen_tool() -> ToolMeta {
    ToolMeta {
        name: "video_generation".into(),
        description: "Generate videos from text prompts. Supports OpenAI, Runway ML, Stable Video, Synthesia, and Pika.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("prompt", "Text prompt describing the desired video", "string"),
            ToolParameter::optional("model", "Model to use: video-1, runway-gen3a, stable-video, synthesia, pika", "string"),
            ToolParameter::optional("duration", "Video duration in seconds", "number"),
            ToolParameter::optional("size", "Video resolution", "string"),
        ]),
    }
}

pub struct VideoGenTool;

// OpenAI video provider
async fn generate_openai_video(
    client: &reqwest::Client,
    prompt: &str,
    duration: u32,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_OPENAI)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

    let response = client
        .post(format!("{}/videos/generations", OPENAI_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({
            "model": "video-1",
            "prompt": prompt,
            "duration": duration,
            "size": "1024x1024"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("OpenAI Video API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let video_url = json["data"][0]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No video URL in response"))?;

    Ok(video_url.to_string())
}

// Runway ML provider
async fn generate_runwayml_video(
    client: &reqwest::Client,
    prompt: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_RUNWAYML)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("RUNWAYML_API_KEY not set"))?;

    let response = client
        .post(format!("{}/generations", RUNWAYML_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "model": "gen3a-turbo"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Runway ML API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let video_url = json["video"]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No video URL in response"))?;

    Ok(video_url.to_string())
}

// Stable Video provider (via Stability AI)
async fn generate_stable_video(
    client: &reqwest::Client,
    prompt: &str,
    duration: u32,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_STABLE_VIDEO)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("STABLE_VIDEO_API_KEY not set"))?;

    let response = client
        .post(format!("{}/stable-video/text-to-video", STABLE_VIDEO_BASE.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "duration": duration,
            "fps": 24,
            "seed": null
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Stable Video API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let video_url = json["video"]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No video URL in response"))?;

    Ok(video_url.to_string())
}

// Synthesia provider (via Synthesia API)
async fn generate_synthesia_video(
    client: &reqwest::Client,
    prompt: &str,
    duration: u32,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_SYNTHESIA)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("SYNTHESIA_API_KEY not set"))?;

    let response = client
        .post(format!("{}/videos", SYNTHESIA_BASE.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "title": prompt,
            "script": prompt,
            "duration": duration,
            "aspect_ratio": "16:9"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Synthesia API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let video_url = json["videoUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No video URL in response"))?;

    Ok(video_url.to_string())
}

// Pika provider (via Pika API)
async fn generate_pika_video(
    client: &reqwest::Client,
    prompt: &str,
    duration: u32,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_PIKA)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("PIKA_API_KEY not set"))?;

    let response = client
        .post(format!("{}/generate", PIKA_BASE.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "duration": duration,
            "aspect_ratio": "16:9",
            "frames_per_second": 24
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Pika API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let video_url = json["videoUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No video URL in response"))?;

    Ok(video_url.to_string())
}

// ---------------------------------------------------------------------------
// Tool execution
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Tool for VideoGenTool {
    fn name(&self) -> &str {
        "video_generation"
    }
    fn description(&self) -> &str {
        "Generate videos from text prompts. Supports OpenAI, Runway ML, Stable Video, Synthesia, and Pika."
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let prompt = call.required_str("prompt").unwrap_or_default();
        let model = call.required_str("model").unwrap_or("video-1");
        let duration = call
            .required_str("duration")
            .unwrap_or("5")
            .parse::<u32>()
            .unwrap_or(5);

        let client = reqwest::Client::new();

        let result = match model {
            "video-1" | "openai" => {
                generate_openai_video(&client, &prompt, duration).await
            }
            "runway-gen3a" | "runway" => {
                generate_runwayml_video(&client, &prompt).await
            }
            "stable-video" => {
                generate_stable_video(&client, &prompt, duration).await
            }
            "synthesia" => {
                generate_synthesia_video(&client, &prompt, duration).await
            }
            "pika" => {
                generate_pika_video(&client, &prompt, duration).await
            }
            _ => Err(anyhow::anyhow!("Unknown video generation model: {}", model)),
        };

        match result {
            Ok(video_url) => ToolResult {
                call_id: call.call_id.clone(),
                output: format!("Generated video: {}", video_url),
                error: None,
            },
            Err(e) => ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(VideoGenTool);
    registry.register_with_def(tool, make_video_gen_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_gen_tool_name() {
        let tool = VideoGenTool;
        assert_eq!(tool.name(), "video_generation");
    }

    #[test]
    fn test_video_gen_tool_description() {
        let tool = VideoGenTool;
        assert!(tool.description().contains("Generate videos"));
    }

    #[test]
    fn test_video_gen_tool_clone() {
        let tool = VideoGenTool;
        let cloned = tool.clone_tool();
        assert_eq!(cloned.name(), "video_generation");
    }

    #[test]
    fn test_video_gen_tool_stable_video_model() {
        let tool = VideoGenTool;
        assert!(tool.description().contains("Stable Video"));
    }

    #[test]
    fn test_video_gen_tool_synthesia_model() {
        let tool = VideoGenTool;
        assert!(tool.description().contains("Synthesia"));
    }

    #[test]
    fn test_video_gen_tool_pika_model() {
        let tool = VideoGenTool;
        assert!(tool.description().contains("Pika"));
    }
}
