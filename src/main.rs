/// ObenAgent CLI — the main entry point.
/// Maps to `hermes_cli/main.py` + `cli.py`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{debug, info};

#[derive(Parser)]
#[command(name = "oben", version, about = "The self-improving AI agent")]
struct Cli {
    /// Enable verbose/debug output
    #[arg(short, long)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive conversation
    Chat {
        /// Disable streaming
        #[arg(long)]
        no_stream: bool,
    },
    /// Run a one-shot prompt
    Run {
        /// The prompt/question
        #[arg(short, long)]
        prompt: String,
        /// Stream text output as it arrives
        #[arg(long)]
        stream: bool,
    },
    /// Setup/configure the agent
    Setup,
    /// Show current configuration
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
    /// List available tools
    Tools,
    /// List and manage skills
    Skills,
    /// List and manage sessions
    Sessions {
        #[command(subcommand)]
        action: Option<SessionsCommand>,
    },
    /// Show agent info
    Info,
    /// Discover models via LLM provider
    Models {
        #[command(subcommand)]
        action: ModelsCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show current config
    Show,
    /// Edit config file
    Edit,
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// List available models from the LLM provider
    List,
    /// Show details for a specific model
    Info {
        /// Model ID to look up
        model: String,
    },
}

/// Session management commands
#[derive(Subcommand)]
enum SessionsCommand {
    /// List all sessions
    List,
    /// Compact (compress) a session using LLM summarization
    ///
    /// Performs full session compaction: prunes tool results, protects
    /// head/tail messages, summarizes middle turns, and iteratively
    /// updates previous summaries.
    Compact {
        /// Session ID or name to compact
        #[arg(short, long)]
        session: Option<String>,
        /// Focus topic — prioritise preserving info related to this topic
        #[arg(short, long)]
        focus: Option<String>,
    },
    /// Delete a session
    Delete {
        /// Session ID or name to delete
        #[arg(short, long)]
        session: String,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // --verbose sets RUST_LOG only if not already configured, so explicit
    // env vars take precedence for fine-grained filtering.
    if cli.verbose && std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "oben=debug");
    }
    oben_utils::logging::init(tracing::Level::INFO);

    match cli.command {
        Commands::Chat { no_stream } => run_chat(!no_stream).await,
        Commands::Run { prompt, stream } => run_one_shot(&prompt, stream).await,
        Commands::Setup => run_setup(),
        Commands::Config { action } => run_config(action).await,
        Commands::Tools => { list_tools(); Ok(()) }
        Commands::Skills => { list_skills(); Ok(()) }
        Commands::Sessions { action } => {
            match action {
                Some(SessionsCommand::List) => list_sessions(),
                Some(SessionsCommand::Compact { session, focus }) => run_compact_session(session.as_deref(), focus.as_deref()).await,
                Some(SessionsCommand::Delete { session }) => run_delete_session(&session),
                None => list_sessions(),
            }
        }
        Commands::Info => { show_info(); Ok(()) },
        Commands::Models { action } => run_models(action).await,
    }
}

async fn run_chat(stream: bool) -> Result<()> {
    info!("Starting interactive chat...");

    let config = oben_config::AppConfig::load()?;
    let mut memory = oben_sessions::SessionManager::new()?;

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);

    // Collect available tool names for conditional guidance.
    let tool_names: Vec<String> = tools.list_tools().iter()
        .map(|t| t.name.clone())
        .collect();

    // Build the 3-tier system prompt:
    //   Stable: identity (from config or default) + tool guidance + skills
    //   Context: cwd-dependent files (.oben.md, AGENTS.md, etc.)
    //   Volatile: memory context, timestamp (built per-turn)
    let identity = oben_config::defaults::default_system_prompt();
    let skills_dirs = vec![
        PathBuf::from("skills"),
    ];
    let context_cwd = std::env::current_dir().ok();

    let volatile = oben_conversation::system_prompt::build_volatile_block(
        None,
        None,
        Some(&config.model.model),
    );
    let assembled = oben_conversation::system_prompt::build_system_prompt(
        &identity,
        &tool_names,
        &skills_dirs,
        context_cwd.as_deref(),
        None,
        Some(&volatile),
    );
    debug!("System prompt ({} chars): {}...", assembled.prompt.len(), &assembled.prompt.chars().take(100).collect::<String>());

    // Create conversation loop with 3-tier system prompt.
    let system_prompt = assembled.prompt.clone();
    let mut conversation = oben_conversation::ConversationLoop::new(
        create_transport(&config, &system_prompt, collect_tool_defs(&tools)),
        std::sync::Arc::new(tools),
        config.max_iterations.unwrap_or(50),
        config.context.max_messages.unwrap_or(100),
    );

    // Start a new session (or use existing active session)
    let session_id = if let Some(sid) = memory.active_session().map(|s| s.id.clone()) {
        memory.switch_session(&sid)?.id.clone()
    } else {
        memory.new_session(&format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))).id.clone()
    };

    memory.load(Some(session_id.as_str()))?;

    let mut call_mode = oben_models::CallMode::Fresh(session_id.clone());

    println!("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");

    // Read user input
    loop {
        print!("> ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input == "quit" || input == "exit" {
            break;
        }

        if input.is_empty() {
            continue;
        }

        // Run turn — messages borrowed from session, no sync needed
        let response = {
            let session = memory.active_session_mut().unwrap();
            let turn_start = Instant::now();
            let result = if stream {
                conversation.run_turn_with_streaming(
                    &mut session.messages,
                    oben_models::Message::user(input),
                    &call_mode,
                    Some(Box::new(|text: &str| {
                        print!("{}", text);
                        std::io::stdout().flush().ok();
                    })),
                )
                .await?
            } else {
                conversation.run_turn(&mut session.messages, oben_models::Message::user(input), &call_mode).await?
            };
            let turn_dur = turn_start.elapsed();
            debug!("Raw response: {:?}", result);
            info!("Turn completed in {:.2}s", turn_dur.as_secs_f64());
            result
        };
        if !stream {
            println!("\n{}", response);
        } else {
            println!();
        }

        // Persist (session.messages is already up to date — no sync needed)
        let save_start = Instant::now();
        memory.save(None)?;
        let save_dur = save_start.elapsed();
        info!("Save completed in {:.2}s", save_dur.as_secs_f64());
        call_mode = oben_models::CallMode::Incremental(session_id.clone());
    }

    Ok(())
}

async fn run_one_shot(prompt: &str, stream: bool) -> Result<()> {
    let config = oben_config::AppConfig::load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);


    let system_prompt = oben_config::defaults::default_system_prompt();
    let mut conversation = oben_conversation::ConversationLoop::new(
        create_transport(&config, &system_prompt, collect_tool_defs(&tools)),
        std::sync::Arc::new(tools),
        config.max_iterations.unwrap_or(50),
        config.context.max_messages.unwrap_or(100),
    );

    let mut messages = Vec::new();
    let call_mode = oben_models::CallMode::Fresh("cli-session".to_string());
    let response = if stream {
        conversation.run_turn_with_streaming(
            &mut messages,
            oben_models::Message::user(prompt),
            &call_mode,
            Some(Box::new(|text: &str| {
                print!("{}", text);
                std::io::stdout().flush().ok();
            })),
        )
        .await?
    } else {
        conversation.run_turn(&mut messages, oben_models::Message::user(prompt), &call_mode).await?
    };
    debug!("Raw response: {:?}", response);
    if !stream {
        println!("\n{}", response);
    } else {
        println!();
    }

    Ok(())
}

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

fn list_tools() {
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
}

fn list_skills() {
    let skills = oben_skills::builtin_skills();
    println!("Built-in skills ({}):", skills.len());
    for skill in skills {
        println!("  📖 {} ({}) — {}", skill.name, skill.category, skill.description);
    }
}

fn list_sessions() -> Result<()> {
    let memory = oben_sessions::SessionManager::new()?;
    let sessions = memory.list_sessions();
    if sessions.is_empty() {
        println!("No sessions found.");
    } else {
        println!("Sessions ({}):", sessions.len());
        for s in sessions {
            let marker = memory.active_session().and_then(|a| if a.id == s.id { Some(" ← active") } else { None }).unwrap_or("");
            println!("  📄 {} — {} messages{}", s.name, s.message_count(), marker);
        }
    }
    Ok(())
}

fn show_info() {
    println!("ObenAgent v{}", env!("CARGO_PKG_VERSION"));
    println!("self-improving AI agent");
    println!("\nUsage: oben <command>");
    println!("\nCommands:");
    println!("  chat    — Start an interactive conversation");
    println!("  run     — Run a one-shot prompt");
    println!("  setup   — Run setup wizard");
    println!("  config  — Show or edit configuration");
    println!("  tools   — List available tools");
    println!("  skills  — List available skills");
    println!("  sessions [list|compact|delete] — Manage sessions");
    println!("  info     — Show agent info");
    println!("  models   — Discover models from LLM provider");
}

/// Collect tool definitions from a registry for structured tool calling.
fn collect_tool_defs(registry: &oben_tools::ToolRegistry) -> Vec<oben_models::Tool> {
    registry.list_tools().into_iter().map(|t| (*t).clone()).collect()
}

/// Create a ChatCompletionsTransport with tools for structured tool calling.
fn create_transport(
    config: &oben_config::AppConfig,
    system_prompt: &str,
    tools: Vec<oben_models::Tool>,
) -> oben_transport::ChatCompletionsTransport {
    oben_transport::ChatCompletionsTransport::from_config_with_tools(
        &config.model,
        system_prompt,
        tools,
    )
}

/// Run session compaction using the SessionManager from oben-sessions.
async fn run_compact_session(session_key: Option<&str>, focus_topic: Option<&str>) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let mut sm = oben_sessions::SessionManager::new()?;

    // Find session (default to active)
    let active_id = sm.active().map(|s| s.id.clone());
    let target: String = match session_key {
        Some(key) => key.to_string(),
        None => active_id.unwrap_or_else(|| "active".to_string()),
    };
    let target_ref = target.as_str();

    // Clone session data
    let session = sm.clone_session(target_ref).ok_or_else(|| {
        anyhow::anyhow!("Session not found: {} (run `oben sessions list` to see available sessions)", target)
    })?;

    if session.message_count() < 8 {
        println!("Session has only {} message(s). Minimum 8 required for compaction.", session.message_count());
        return Ok(());
    }

    println!("Compacting session '{}' ({} messages)...", session.name, session.message_count());

    // Build a transport for the summary LLM call (no tools needed for compaction)
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

    // Update session — fresh borrow, no conflicts with previous code
    if let Some(s) = sm.session_mut(&session.id) {
        s.messages = result.messages;
        s.updated_at = chrono::Utc::now();
        if let Some(summary) = result.summary {
            s.memory_context = Some(summary.clone());
            // Add summary chunk for the compressed messages
            let old_msg_count = session.messages.len();
            s.summary_chunks.push(oben_models::SummaryChunk {
                from: 1,
                to: old_msg_count,
                summary,
            });
        }
    }
    sm.save_session(&session.id)?;

    // Report results
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
    sm.delete(session_key)?;
    println!("Deleted session '{}'", session_key);
    Ok(())
}

async fn run_models(action: ModelsCommand) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let transport = create_transport(&config, "", Vec::new());

    match action {
        ModelsCommand::List => {
            println!("Fetching models from provider...\n");
            let models = transport.list_models().await?;
            println!("Found {} model(s):\n", models.data.len());

            let headers = &["ID", "Max Tokens", "Owned By"];
            let rows: Vec<Vec<String>> = models
                .data
                .iter()
                .map(|m| vec![
                    m.id.clone(),
                    m.max_model_len.map(|t| t.to_string()).unwrap_or_else(|| "N/A".to_string()),
                    m.owned_by.clone(),
                ])
                .collect();
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