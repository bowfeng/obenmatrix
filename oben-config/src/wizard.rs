//! Setup wizard — interactive config wizard (maps to `hermes setup`).

use anyhow::Result;
use dialoguer::{Input, Select};
use tracing::info;

use super::config::AppConfig;

pub fn run_setup(config: &mut AppConfig) -> Result<()> {
    println!("\n🦀 ObenAgent Setup Wizard\n");
    println!("Note: use --profile <name> to configure a profile-specific config.\n");

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

    let (selected_provider, base_url) = match selected {
        0 => ("openrouter".to_string(), None),
        1 => ("openai".to_string(), None),
        2 => ("anthropic".to_string(), None),
        3 => ("bedrock".to_string(), None),
        4 => ("gemini".to_string(), None),
        5 => {
            config.model.base_url = Some("http://localhost:1234/v1".to_string());
            (
                "custom".to_string(),
                Some("http://localhost:1234/v1".to_string()),
            )
        }
        6 => {
            let url: String = Input::new()
                .with_prompt("Custom API base URL")
                .default("http://localhost:1234/v1".to_string())
                .interact()?;
            config.model.base_url = Some(url.clone());
            ("custom".to_string(), Some(url))
        }
        _ => unreachable!(),
    };

    // Resolve through the provider registry so aliases map to canonical kind + transport
    let provider_info =
        oben_models::provider_registry::resolve_provider_info(&selected_provider)
            .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", selected_provider))?;

    if base_url.is_none() && !provider_info.base_url.is_empty() {
        config.model.base_url = Some(provider_info.base_url.to_string());
    }

    // Map canonical name back to ProviderKind enum
    config.model.kind = match provider_info.canonical {
        "openai" => oben_models::ProviderKind::OpenAI,
        "anthropic" => oben_models::ProviderKind::Anthropic,
        "openrouter" => oben_models::ProviderKind::OpenRouter,
        "bedrock" => oben_models::ProviderKind::Bedrock,
        "gemini" => oben_models::ProviderKind::Gemini,
        _ => oben_models::ProviderKind::Custom,
    };

    // --- API key FIRST (moved up so we can use it for listing) ---
    let api_key: String = Input::new()
        .with_prompt("API key (leave blank to skip / set later)")
        .default(String::new())
        .interact()?;
    if !api_key.trim().is_empty() {
        config.model.api_key = Some(api_key);
    }

    // --- Model listing + model name selection ---
    let model: String = {
        let model_prompt = "Model name (e.g. qwen/qwen3-235b:free, gpt-4o)";
        let default_model = "qwen/qwen3-235b:free";

        if !config.model.api_key.as_ref().map_or(true, |k| k.trim().is_empty()) {
            // API key available — attempt to list models
            println!("\n🔍 Fetching available models from provider...");
            match try_list_models(&config.model) {
                Some(display_options) if !display_options.is_empty() => {
                    // Got model list, let user pick
                    let mut select_options = display_options.clone();
                    select_options.insert(
                        select_options.len(),
                        "⚙️  Skip listing - enter model manually".to_string(),
                    );
                    let idx = Select::new()
                        .with_prompt("Select a model")
                        .items(&select_options)
                        .default(0)
                        .interact()?;
                    if idx < (select_options.len() - 1) {
                        let id = display_options[idx].split_whitespace().next().unwrap().to_string();
                        println!("✅ Selected: {}", id);
                        id
                    } else {
                        let manual: String = Input::new()
                            .with_prompt(model_prompt)
                            .default(default_model.to_string())
                            .interact()?;
                        manual
                    }
                }
                Some(_) => {
                    println!("⚠️  No models with max_tokens returned. Proceeding with manual input.");
                    let manual: String = Input::new()
                        .with_prompt(model_prompt)
                        .default(default_model.to_string())
                        .interact()?;
                    manual
                }
                None => {
                    println!("⚠️  Could not fetch model list. Proceeding with manual input.");
                    let manual: String = Input::new()
                        .with_prompt(model_prompt)
                        .default(default_model.to_string())
                        .interact()?;
                    manual
                }
            }
        } else {
            // No API key — skip listing entirely, go straight to manual input
            println!("ℹ️  No API key set, skipping model listing.");
            let manual: String = Input::new()
                .with_prompt(model_prompt)
                .default(default_model.to_string())
                .interact()?;
            manual
        }
    };
    config.model.model = model;

    // Step 3.5: Auto-detect max_tokens from provider
    println!("\n🔍 Discovering model capabilities...");
    if let Some(max_tokens) = detect_max_tokens(&config.model) {
        config.model.max_tokens = Some(max_tokens);
        info!("Auto-detected max_tokens: {} from provider", max_tokens);
        println!("✅ Found model (max tokens: {})", max_tokens);
    } else {
        println!("⚠️  Could not reach provider to auto-detect max_tokens.");
        println!("   max_tokens will use default (8192). You can configure it manually later.");
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
    config.save_with_profile(None)?;

    println!("\n✅ Configuration saved successfully.\n");
    println!("You can re-run this wizard anytime with: `oben setup`");
    println!("Use --profile <name> to manage profile-specific configurations.\n");

    Ok(())
}

/// Attempt to fetch available models from the provider.
///
/// Runs in a separate thread with its own tokio runtime to avoid
/// "Cannot start a runtime from within a runtime" panic when called
/// from inside the CLI's #[tokio::main] context.
///
/// Returns display strings on success, None on error.
fn try_list_models(config: &oben_models::ProviderConfig) -> Option<Vec<String>> {
    let config_clone = config.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().ok()?;
        let transport = oben_transport::Transport::from_config(&config_clone, "");
        if let Ok(resp) = rt.block_on(async { transport.list_models().await }) {
            let mut models: Vec<_> = resp
                .data
                .into_iter()
                .filter(|m| m.max_model_len.unwrap_or(0) > 0)
                .collect();
            models.sort_by(|a, b| b.max_model_len.cmp(&a.max_model_len));
            let display: Vec<String> = models.iter().take(100).map(|m| {
                let max_t = m.max_model_len.unwrap_or(0);
                format!("{} ({}, max_tokens: {})", m.id, m.owned_by, max_t)
            })
            .collect();
            Some(display)
        } else {
            None
        }
    })
    .join()
    .ok()
    .flatten()
}

/// Detect max_tokens from the LLM provider and return it if found.
///
/// Runs in a separate thread with its own tokio runtime to avoid
/// "Cannot start a runtime from within a runtime" panic when called
/// from inside the CLI's #[tokio::main] context.
fn detect_max_tokens(config: &oben_models::ProviderConfig) -> Option<usize> {
    let config_clone = config.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().ok()?;
        let transport =
            oben_transport::Transport::from_config(&config_clone, "");
        let result = rt.block_on(async {
            transport.find_model(&config_clone.model).await
        });
        match result {
            Ok(Some(model_info)) => model_info.max_model_len,
            Ok(None) => None,
            Err(_) => None,
        }
    })
    .join()
    .ok()
    .flatten()
}
