use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// Constants
const OPENAI_BASE: &str = "https://api.openai.com/v1";
const FLUX_BASE: &str = "https://api.flux.ai/v1";
const MIDJOURNEY_BASE: &str = "https://api.midjourney.com/v1";
const STABLE_DIFFUSION_BASE: &str = "https://api.stability.ai/v1";
const STABLE_VIDEO_BASE: &str = "https://api.stability.ai/v2beta";

const ENV_OPENAI: &str = "OPENAI_API_KEY";
const ENV_FLUX: &str = "FLUX_API_KEY";
const ENV_MIDJOURNEY: &str = "MIDJOURNEY_API_KEY";
const ENV_STABLE_DIFFUSION: &str = "STABLE_DIFFUSION_API_KEY";
const ENV_STABLE_VIDEO: &str = "STABLE_VIDEO_API_KEY";

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_image_gen_tool() -> ToolMeta {
    ToolMeta {
        name: "image_generation".into(),
        description: "Generate images from text prompts. Supports OpenAI DALL-E, FLUX, Midjourney, Stable Diffusion, and Stable Video.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("prompt", "Text prompt describing the desired image", "string"),
            ToolParameter::optional("model", "Model to use: dall-e-3, flux, midjourney, stable-diffusion-xl, stable-video", "string"),
            ToolParameter::optional("size", "Image size: 1024x1024, 1024x1792, 1792x1024", "string"),
            ToolParameter::optional("quality", "Image quality: standard, hd", "string"),
        ]),
    }
}

pub struct ImageGenTool;

// DALL-E provider
async fn generate_dalle_image(
    client: &reqwest::Client,
    prompt: &str,
    model: &str,
    size: &str,
    quality: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_OPENAI)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

    let response = client
        .post(format!("{}/images/generations", OPENAI_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": size,
            "quality": quality,
            "response_format": "b64_json"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("DALL-E API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let b64_json = json["data"][0]["b64_json"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No image data in response"))?;

    Ok(format!("DATA:image/png;base64,{}", b64_json))
}

// FLUX provider
async fn generate_flux_image(
    client: &reqwest::Client,
    prompt: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_FLUX)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("FLUX_API_KEY not set"))?;

    let response = client
        .post(format!("{}/generations", FLUX_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "model": "FLUX.1.1-pro",
            "aspect_ratio": "1:1"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("FLUX API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let image_url = json["images"][0]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No image URL in response"))?;

    Ok(image_url.to_string())
}

// Midjourney provider (via API proxy)
async fn generate_midjourney_image(
    client: &reqwest::Client,
    prompt: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_MIDJOURNEY)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("MIDJOURNEY_API_KEY not set"))?;

    let response = client
        .post(format!("{}/imagine", MIDJOURNEY_BASE))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Midjourney API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let image_url = json["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No image URL in response"))?;

    Ok(image_url.to_string())
}

async fn generate_stable_diffusion_image(
    client: &reqwest::Client,
    prompt: &str,
    model: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_STABLE_DIFFUSION)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("STABLE_DIFFUSION_API_KEY not set"))?;

    let response = client
        .post(format!("{}/stable-diffusion/v1/generate", STABLE_DIFFUSION_BASE.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "model": model,
            "image_height": 1024,
            "image_width": 1024,
            "samples": 1,
            "steps": 30
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Stable Diffusion API error: {} - {}", status, body));
    }

    let json: serde_json::Value = response.json().await?;
    let image_url = json["artifacts"][0]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No image URL in response"))?;

    Ok(image_url.to_string())
}

async fn generate_stable_video_image(
    client: &reqwest::Client,
    prompt: &str,
    model: &str,
) -> anyhow::Result<String> {
    let api_key = std::env::var(ENV_STABLE_VIDEO)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("STABLE_VIDEO_API_KEY not set"))?;

    let response = client
        .post(format!("{}/stable-video/img2vid", STABLE_VIDEO_BASE.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "model": model,
            "frames_per_second": 6,
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

// ---------------------------------------------------------------------------
// Tool execution
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Tool for ImageGenTool {
    fn name(&self) -> &str {
        "image_generation"
    }
    fn description(&self) -> &str {
        "Generate images from text prompts. Supports OpenAI DALL-E, FLUX, Midjourney, Stable Diffusion, and Stable Video."
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let prompt = call.required_str("prompt").unwrap_or_default();
        let model = call.required_str("model").unwrap_or("dall-e-3");
        let size = call.required_str("size").unwrap_or("1024x1024");
        let quality = call.required_str("quality").unwrap_or("standard");

        let client = reqwest::Client::new();
        
        let result = match model {
            "dall-e-3" | "dall-e" => {
                generate_dalle_image(&client, &prompt, &model, &size, &quality).await
            }
            "flux" => {
                generate_flux_image(&client, &prompt).await
            }
            "midjourney" | "mj" => {
                generate_midjourney_image(&client, &prompt).await
            }
            "stable-diffusion" | "stable-diffusion-xl" => {
                generate_stable_diffusion_image(&client, &prompt, model).await
            }
            "stable-video" => {
                generate_stable_video_image(&client, &prompt, model).await
            }
            _ => Err(anyhow::anyhow!("Unknown image generation model: {}", model)),
        };

        match result {
            Ok(image_data) => ToolResult {
                call_id: call.call_id.clone(),
                output: format!("Generated image: {}", image_data),
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
    let tool = Box::new(ImageGenTool);
    registry.register_with_def(tool, make_image_gen_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_gen_tool_name() {
        let tool = ImageGenTool;
        assert_eq!(tool.name(), "image_generation");
    }

    #[test]
    fn test_image_gen_tool_description() {
        let tool = ImageGenTool;
        assert!(tool.description().contains("Generate images"));
    }

    #[test]
    fn test_image_gen_tool_clone() {
        let tool = ImageGenTool;
        let cloned = tool.clone_tool();
        assert_eq!(cloned.name(), "image_generation");
    }

    #[test]
    fn test_image_gen_tool_stable_diffusion_model() {
        let tool = ImageGenTool;
        assert!(tool.description().contains("Stable Diffusion"));
    }

    #[test]
    fn test_image_gen_tool_stable_video_model() {
        let tool = ImageGenTool;
        assert!(tool.description().contains("Stable Video"));
    }
}
