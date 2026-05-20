//! All CLI command implementations.
//!
//! Domain crates provide types and business logic only; this crate handles
//! CLI parsing, wiring crates together, and user-facing output.

use anyhow::Result;
use std::io::Write;
use tracing::info;

use clap::Parser;
use crate::cli::{Cli, Commands, ConfigCommand, ModelsCommand, SessionsCommand};
use oben_models::MessageRole;

/// Entry point: parse CLI args and dispatch to the appropriate handler.
pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    // --verbose sets RUST_LOG only if not already configured, so explicit
    // env vars take precedence for fine-grained filtering.
    if cli.verbose && std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "oben=debug");
    }
    oben_utils::logging::init(tracing::Level::INFO);

    match cli.command {
        Commands::Chat { no_stream, continue_session } => run_chat(!no_stream, continue_session.as_deref()).await,
        Commands::Run { prompt, stream } => run_one_shot(&prompt, stream).await,
        Commands::Setup => run_setup(),
        Commands::Config { action } => run_config(action).await,
        Commands::Tools => list_tools(),
        Commands::Skills => list_skills(),
        Commands::Sessions { action } => {
            match action {
                Some(SessionsCommand::List) => list_sessions(),
                Some(SessionsCommand::Compact { session, focus }) => run_compact_session(session.as_deref(), focus.as_deref()).await,
                Some(SessionsCommand::Delete { session }) => run_delete_session(&session),
                Some(SessionsCommand::Dump { session }) => dump_session(session.as_deref()),
                None => list_sessions(),
            }
        }
        Commands::Models { action } => run_models(action).await,
        Commands::Tui => oben_tui::run_tui().await,
    }
}

// ── Chat / Run ──────────────────────────────────────────────────────────

/// Display messages from a session in a compact format.
fn print_session_messages(messages: &[oben_models::Message], max_show: usize) {
    if messages.is_empty() {
        println!("(no messages)");
        return;
    }

    let show_count = messages.len().min(max_show);
    let show = &messages[..show_count];
    let overflow = messages.len().saturating_sub(max_show);

    for msg in show {
        let role = match msg.role {
            MessageRole::User => "📝 你",
            MessageRole::Assistant => "🤖 agent",
            MessageRole::System => "📋 system",
            MessageRole::Tool => "⚙️ tool",
        };
        let text = msg.content.to_text_ref().unwrap_or("<non-text>");
        // Truncate long messages
        let display = if text.len() > 120 {
            format!("{}...", &text[..117])
        } else {
            text.to_string()
        };
        println!("  {} {}", role, display);
    }

    if overflow > 0 {
        println!("  ... {} more messages", overflow);
    }
}

async fn run_chat(stream: bool, continue_with: Option<&str>) -> Result<()> {
    info!("Starting interactive chat...");

    let config = oben_config::AppConfig::load()?;
    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);

    let tool_names: Vec<String> = tools.list_tools().iter()
        .map(|t| t.name.clone()).collect();

    let identity = oben_config::defaults::default_system_prompt();
    let skills_dirs = vec![std::path::PathBuf::from("skills")];
    let context_cwd = std::env::current_dir().ok();

    let volatile = oben_conversation::system_prompt::build_volatile_block(
        None, None, Some(&config.model.model),
    );
    let assembled = oben_conversation::system_prompt::build_system_prompt(
        &identity, &tool_names, &skills_dirs, context_cwd.as_deref(),
        None, Some(&volatile),
    );

    let chat = oben_conversation::ChatSession::new(oben_conversation::ChatSessionConfig {
        system_prompt_text: assembled.prompt.clone(),
        transport: create_transport(&config, &assembled.prompt, tool_names.clone()),
        tools: std::sync::Arc::new(tools),
        max_iterations: config.max_iterations.unwrap_or(50),
        max_messages: config.context.max_messages.unwrap_or(100),
    })?;

    let mut chat = chat;

    // Continue an existing session if requested
    if let Some(key) = continue_with {
        // "latest" means use the most recent session
        let resolved_key = if key == "latest" {
            chat.session_manager().active_session().map(|s| s.name.clone()).unwrap_or_else(|| key.to_string())
        } else {
            key.to_string()
        };
        let name = chat.continue_session(&resolved_key)?;
        if let Some(s) = chat.session_manager().active_session() {
            let msg_count = s.messages.len();
            println!("Continuing session: {} ({} messages)\n", name, msg_count);
            print_session_messages(&s.messages, 10);
            println!();
        }
    } else {
        // 如果有已存在的 active session，显示出来
        if let Some(name) = chat.loaded_session_name() {
            if let Some(s) = chat.session_manager().active_session() {
                println!("Session: {} ({} messages)\n", name, s.messages.len());
            }
        }
    }
    println!("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");

    loop {
        print!("> ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input == "quit" || input == "exit" { break; }
        if input.is_empty() { continue; }

        let response = chat.turn(input, stream, stream.then(|| {
            Box::new(|text: &str| {
                print!("{}", text);
                std::io::stdout().flush().ok();
            }) as oben_models::StreamDeltaCallback
        })).await?;
        if stream {
            println!();
        } else {
            println!("\n{}", response);
        }
    }

    Ok(())
}

async fn run_one_shot(prompt: &str, stream: bool) -> Result<()> {
    let config = oben_config::AppConfig::load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);

    let system_prompt = oben_config::defaults::default_system_prompt();
    let mut conversation = oben_conversation::ConversationLoop::new(
        create_transport(&config, &system_prompt, vec![]),
        std::sync::Arc::new(tools),
        config.max_iterations.unwrap_or(50),
        config.context.max_messages.unwrap_or(100),
    );

    let mut messages = Vec::new();
    let call_mode = oben_models::CallMode::Fresh("cli-session".to_string());

    // Preflight check: compress if session already over threshold
    let passes = conversation.preflight_check(&mut messages).await;
    if let Ok(n) = passes {
        if n > 0 {
            eprintln!("Preflight: {} compression pass(es) before turn", n);
        }
    }

    let response = if stream {
        conversation.run_turn_with_streaming(
            &mut messages,
            oben_models::Message::user(prompt),
            &call_mode,
            Some(Box::new(|text: &str| {
                print!("{}", text);
                std::io::stdout().flush().ok();
            })),
        ).await?
    } else {
        conversation.run_turn(&mut messages, oben_models::Message::user(prompt), &call_mode).await?
    };
    if !stream { println!("\n{}", response); } else { println!(); }

    Ok(())
}

// ── Setup & Config ──────────────────────────────────────────────────────

fn run_setup() -> Result<()> {
    let mut config = oben_config::AppConfig::load()?;
    oben_config::wizard::run_setup(&mut config)?;
    Ok(())
}

async fn run_config(action: ConfigCommand) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    match action {
        ConfigCommand::Show => {
            println!("{}", serde_yaml::to_string(&config)?);
        }
        ConfigCommand::Edit => {
            let path = oben_config::AppConfig::config_path();
            println!("Config file: {}", path.display());
            println!("Edit it manually, or run `oben setup` for the wizard.");
        }
    }
    Ok(())
}

// ── Tools & Skills ──────────────────────────────────────────────────────

fn list_tools() -> Result<()> {
    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);
    let tool_list = tools.list_tools();
    if tool_list.is_empty() {
        println!("No tools registered.");
    } else {
        println!("Registered tools ({}):", tool_list.len());
        for tool in tool_list {
            println!("  📦 {} — {}", tool.name, tool.description);
        }
    }
    Ok(())
}

fn list_skills() -> Result<()> {
    let skills = oben_skills::builtin_skills();
    println!("Built-in skills ({}):", skills.len());
    for skill in skills {
        println!("  📖 {} ({}) — {}", skill.name, skill.category, skill.description);
    }
    Ok(())
}

// ── Sessions ────────────────────────────────────────────────────────────

fn list_sessions() -> Result<()> {
    let mut memory = oben_sessions::SessionManager::new()?;
    memory.init()?;
    let sessions = memory.list_sessions();
    if sessions.is_empty() {
        println!("No sessions found.");
    } else {
        println!("Sessions ({}):", sessions.len());
        for s in sessions {
            let marker = memory.active_session().and_then(|a|
                if a.id == s.id { Some(" ← active") } else { None }
            ).unwrap_or("");
            println!("  📄 {} — {} messages{}", s.name, s.message_count(), marker);
        }
    }
    Ok(())
}

async fn run_compact_session(session_key: Option<&str>, focus_topic: Option<&str>) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let mut sm = oben_sessions::SessionManager::new()?;

    let active_id = sm.active().map(|s| s.id.clone());
    let target: String = match session_key {
        Some(key) => key.to_string(),
        None => active_id.unwrap_or_else(|| "active".to_string()),
    };
    let target_ref = target.as_str();

    let session = sm.clone_session(target_ref).ok_or_else(|| {
        anyhow::anyhow!("Session not found: {} (run `oben sessions list` to see available sessions)", target)
    })?;

    if session.message_count() < 8 {
        println!("Session has only {} message(s). Minimum 8 required for compaction.", session.message_count());
        return Ok(());
    }

    println!("Compacting session '{}' ({} messages)...", session.name, session.message_count());

    let transport = create_transport(&config, "", Vec::new());
    let comp_config = oben_conversation::compression::CompressionConfig::default();

    let result = oben_conversation::compact_session_messages(
        &transport,
        &session.messages,
        &comp_config,
        session.memory_context.as_deref(),
        focus_topic,
        1,
    ).await?;

    if let Some(s) = sm.session_mut(&session.id) {
        s.messages = result.messages;
        s.updated_at = chrono::Utc::now();
        if let Some(summary) = result.summary {
            s.memory_context = Some(summary.clone());
            let old_msg_count = session.messages.len();
            s.summary_chunks.push(oben_models::SummaryChunk {
                from: 1, to: old_msg_count, summary,
            });
        }
    }
    sm.save_session(&session.id)?;

    println!("✓ Compaction complete:");
    println!("  Before: {} messages, ~{} tokens", result.stats.original_count, result.stats.original_tokens);
    println!("  After:  {} messages, ~{} tokens", result.stats.compressed_count, result.stats.compressed_tokens);
    println!("  Saved:  {:.0}% tokens ({} tool results pruned)",
        result.stats.savings_pct, result.stats.pruned_tool_results);
    if result.stats.summary_generated {
        println!("  Summary: LLM-generated (iterative)");
    } else {
        println!("  Summary: LLM call skipped/fallback");
    }
    if focus_topic.is_some() {
        println!("  Focus: {:?}", focus_topic);
    }

    Ok(())
}

fn run_delete_session(session_key: &str) -> Result<()> {
    let mut sm = oben_sessions::SessionManager::new()?;
    sm.init()?;
    sm.delete(session_key)?;
    println!("Deleted session '{}'", session_key);
    Ok(())
}

fn dump_session(session_key: Option<&str>) -> Result<()> {
    let mut sm = oben_sessions::SessionManager::new()?;
    sm.load(None)?;

    let active_id = sm.active().map(|s| s.id.clone());
    let target: String = match session_key {
        Some(key) => key.to_string(),
        None => active_id.clone().unwrap_or_else(|| "active".to_string()),
    };

    let session_id = sm.find_key(&target)
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}. Run `oben sessions list` to see available sessions", target))?;

    let sessions: Vec<oben_models::Session> = sm.list_sessions().into_iter().map(|s| s.clone()).collect();
    let session = sessions.iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?
        .clone();

    let session_name = session.metadata.title.as_deref()
        .unwrap_or(&session.id)
        .replace(" ", "-");
    let filename = format!("{}/dump-{}-{}.json",
        std::env::current_dir().unwrap().display(),
        session_name,
        chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    let dump: serde_json::Value = serde_json::json!({
        "id": session.id,
        "name": session.name,
        "title": session.metadata.title,
        "message_count": session.messages.len(),
        "messages": session.messages,
    });

    let json = serde_json::to_string_pretty(&dump)?;
    std::fs::write(&filename, &json)?;

    println!("Dumped {} messages from '{}' to {}",
        session.messages.len(),
        session.metadata.title.as_deref().unwrap_or(&session.name),
        filename);
    Ok(())
}

// ── Models ──────────────────────────────────────────────────────────────

async fn run_models(action: ModelsCommand) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let transport = create_transport(&config, "", Vec::new());

    match action {
        ModelsCommand::List => {
            println!("Fetching models from provider...\n");
            let models = transport.list_models().await?;
            println!("Found {} model(s):\n", models.data.len());

            let headers = &["ID", "Max Tokens", "Owned By"];
            let rows: Vec<Vec<String>> = models.data.iter().map(|m| vec![
                m.id.clone(),
                m.max_model_len.map(|t| t.to_string()).unwrap_or_else(|| "N/A".to_string()),
                m.owned_by.clone(),
            ]).collect();
            oben_utils::terminal::print_table_stderr(headers, rows);
        }
        ModelsCommand::Info { model } => {
            println!("Looking up model: {}\n", model);
            match transport.find_model(&model).await? {
                Some(m) => {
                    let headers = &["Field", "Value"];
                    let rows = vec![
                        vec!["ID".to_string(), m.id],
                        vec!["Object".to_string(), m.object],
                        vec!["Created".to_string(), chrono::DateTime::from_timestamp(m.created as i64, 0).map(|d| d.to_string()).unwrap_or("unknown".to_string())],
                        vec!["Owned By".to_string(), m.owned_by],
                        vec!["Max Model Length".to_string(), m.max_model_len.map(|t| t.to_string()).unwrap_or("N/A".to_string())],
                        vec!["Root".to_string(), m.root.unwrap_or("N/A".to_string())],
                        vec!["Parent".to_string(), m.parent.unwrap_or("N/A".to_string())],
                    ];
                    oben_utils::terminal::print_table_stderr(headers, rows);
                }
                None => {
                    println!("Model '{}' not found.", model);
                    println!("Run 'oben models list' to see available models.");
                }
            }
        }
    }
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn collect_tool_defs(registry: &oben_tools::ToolRegistry) -> Vec<oben_models::Tool> {
    registry.list_tools().into_iter().map(|t| (*t).clone()).collect()
}

fn create_transport(
    config: &oben_config::AppConfig,
    system_prompt: &str,
    _tool_names: Vec<String>,
) -> oben_transport::ChatCompletionsTransport {
    oben_transport::ChatCompletionsTransport::from_config_with_tools(
        &config.model,
        system_prompt,
        collect_tool_defs(&oben_tools::ToolRegistry::new()),
    )
}
