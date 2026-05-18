//! Setup wizard — interactive config wizard (maps to `hermes setup`).

use anyhow::Result;
use dialoguer::{Input, Select};
use tracing::info;

use super::config::AppConfig;

pub fn run_setup(config: &mut AppConfig) -> Result<()> {
    println!("\n🦀 ObenAgent Setup Wizard\n");

    // Step 1: Model provider
    let providers = vec![
        "OpenRouter",
        "OpenAI",
        "Anthropic",
        "Bedrock",
        "Gemini",
        "LMStudio (local)",
        "Custom endpoint",
    ];
    let selected = Select::new()
        .with_prompt("Select LLM provider")
        .items(&providers)
        .default(0)
        .interact()?;

    let kind = match selected {
        0 => oben_models::ProviderKind::OpenRouter,
        1 => oben_models::ProviderKind::OpenAI,
        2 => oben_models::ProviderKind::Anthropic,
        3 => oben_models::ProviderKind::Bedrock,
        4 => oben_models::ProviderKind::Gemini,
        5 => {
            config.model.base_url = Some("http://localhost:1234/v1".to_string());
            oben_models::ProviderKind::Custom
        }
        6 => {
            let url: String = Input::new()
                .with_prompt("Custom API base URL")
                .default("http://localhost:1234/v1".to_string())
                .interact()?;
            config.model.base_url = Some(url);
            oben_models::ProviderKind::Custom
        }
        _ => unreachable!(),
    };
    config.model.kind = kind;

    // Step 2: Model name
    let model: String = Input::new()
        .with_prompt("Model name (e.g. qwen/qwen3-235b:free, gpt-4o)")
        .default("qwen/qwen3-235b:free".to_string())
        .interact()?;
    config.model.model = model;

    // Step 3: API key
    let api_key: String = Input::new()
        .with_prompt("API key (leave blank to skip / set later)")
        .default(String::new())
        .interact()?;
    if !api_key.trim().is_empty() {
        config.model.api_key = Some(api_key);
    }

    // Step 4: Max iterations
    let max_iter: usize = Input::new()
        .with_prompt("Max iterations per turn")
        .default(50)
        .interact()?;
    config.max_iterations = Some(max_iter);

    // Step 5: Context compression
    let compression_methods = vec!["summary", "token_count", "none"];
    let compress_selected = Select::new()
        .with_prompt("Context compression method")
        .items(&compression_methods)
        .default(0)
        .interact()?;
    config.context.compression = compression_methods[compress_selected].to_string();

    // Save
    config.save()?;

    println!("\n✅ Configuration saved to ~/.oben/config.yaml\n");
    println!("You can re-run this wizard anytime with: `oben setup`\n");

    Ok(())
}
