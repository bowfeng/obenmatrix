//! All CLI command implementations.
//!
//! Domain crates provide types and business logic only; this crate handles
//! CLI parsing, wiring crates together, and user-facing output.

use anyhow::Result;
use std::io::Write;
use tracing::info;
use uuid::Uuid;

use crate::cli::{
    Cli, Commands, ConfigCommand, CronCommand, GoalCommand, ModelsCommand, SessionsCommand,
};
use clap::Parser;
use oben_cron::{CronJob, CronStore};
use oben_goals::{GoalStore, JsonGoalStore};
use oben_models::TransportProvider;
use oben_sessions::DBSessionManager;

/// Entry point: parse CLI args and dispatch to the appropriate handler.
pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    // --verbose sets RUST_LOG only if not already configured, so explicit
    // env vars take precedence for fine-grained filtering.
    if cli.verbose && std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "oben=debug");
    }
    let _log_path = oben_utils::logging::init(tracing::Level::INFO);
    // Install panic hook so `panic!` goes into the log file instead of the TUI screen.
    oben_utils::logging::init_panic_hook();

    match cli.command {
        Commands::Chat {
            no_stream,
            continue_session,
        } => run_chat(!no_stream, continue_session.as_deref()).await,
        Commands::Run { prompt, stream } => run_one_shot(&prompt, stream).await,
        Commands::Setup => run_setup(),
        Commands::Config { action } => run_config(action).await,
        Commands::Tools => list_tools(),
        Commands::Skills => list_skills(),
        Commands::Sessions { action } => match action {
            Some(SessionsCommand::List) => list_sessions(),
            Some(SessionsCommand::Compact { session, focus }) => {
                run_compact_session(session.as_deref(), focus.as_deref()).await
            }
            Some(SessionsCommand::Delete { session }) => run_delete_session(&session),
            Some(SessionsCommand::Dump { session }) => dump_session(session.as_deref()),
            None => list_sessions(),
        },
        Commands::Models { action } => run_models(action).await,
        Commands::Tui { session } => oben_tui::run_tui(session.as_deref()).await,
        Commands::Cron { action } => match action {
            None => cron_list(false),
            Some(CronCommand::List { all }) => cron_list(all),
            Some(CronCommand::Create {
                schedule,
                prompt,
                name,
                repeat,
            }) => cron_create(&schedule, prompt.as_deref(), name.as_deref(), repeat),
            Some(CronCommand::Pause { id }) => cron_pause(&id),
            Some(CronCommand::Resume { id }) => cron_resume(&id),
            Some(CronCommand::Remove { id }) => cron_remove(&id),
            Some(CronCommand::Tick) => cron_tick(),
            Some(CronCommand::Start) => cron_start(),
            Some(CronCommand::Info) => cron_info(),
        },
        Commands::Goals { action } => match action {
            None => goal_list(None),
            Some(GoalCommand::Start { goal, max_turns }) => goal_start(&goal, max_turns).await,
            Some(GoalCommand::List { status }) => goal_list(status.as_deref()),
            Some(GoalCommand::Status { goal_id }) => goal_status(&goal_id),
            Some(GoalCommand::Pause { id }) => goal_pause(&id).await,
            Some(GoalCommand::Resume { id, reset }) => goal_resume(&id, reset).await,
            Some(GoalCommand::Clear { id }) => goal_clear(&id).await,
        },
    }
}

// ── Chat / Run ──────────────────────────────────────────────────────────

async fn run_chat(stream: bool, continue_with: Option<&str>) -> Result<()> {
    info!("Starting interactive chat...");

    let config = oben_config::AppConfig::load()?;
    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);

    let tool_names: Vec<String> = tools.list_tools().iter().map(|t| t.name.clone()).collect();

    let identity = oben_config::defaults::default_system_prompt();
    let skills_dirs: Vec<std::path::PathBuf> = config.skills.dirs.iter()
        .map(|d| std::path::PathBuf::from(d))
        .collect();
    let context_cwd = std::env::current_dir().ok();

    let volatile =
        oben_agent::system_prompt::build_volatile_block(None, None, Some(&config.model.model));
    let assembled = oben_agent::system_prompt::build_system_prompt(
        &identity,
        &tool_names,
        &skills_dirs,
        context_cwd.as_deref(),
        None,
        Some(&volatile),
    );

    let max_iterations = config.max_iterations.unwrap_or(50);
    let max_messages = config.context.max_messages.unwrap_or(100);

    let transport = create_transport(&config, &assembled.prompt, &tools);
    let tools = std::sync::Arc::new(tools);

    let mut chat = oben_agent::Agent::new(oben_agent::AgentConfig::from_app_config(
        &config,
        assembled.prompt.clone(),
        max_iterations,
        max_messages,
        skills_dirs,
        transport,
        tools,
        oben_agent::AgentCallbacks::default(),
    )    ).await?;

    let callbacks = create_cli_callbacks();
    chat.interactive_chat(stream, continue_with, &callbacks)
        .await
}

fn create_cli_callbacks() -> oben_agent::AgentCallbacks {
    use std::io::{self, Write};
    oben_agent::AgentCallbacks {
        print_prompt: Some(Box::new(|| print!("> "))),
        print_flush: Some(Box::new(|| { let _ = std::io::stdout().flush(); })),
        print_info: Some(Box::new(|msg: &str| print!("{}
", msg))),
        print_newline: Some(Box::new(|| println!())),
        read_input: Some(Box::new(|| {
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_ok() {
                Some(input.trim().to_string())
            } else {
                Some(String::new())
            }
        })),
        should_exit: Some(Box::new(|input: &str| input == "quit" || input == "exit")),
        ..Default::default()
    }
}

async fn run_one_shot(prompt: &str, stream: bool) -> Result<()> {
    let config = oben_config::AppConfig::load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);

    let system_prompt = oben_config::defaults::default_system_prompt();
    let transport = create_transport(&config, &system_prompt, &tools);
    let tools = std::sync::Arc::new(tools);

    let max_iterations = config.max_iterations.unwrap_or(50);
    let max_messages = config.context.max_messages.unwrap_or(100);

    let skills_dirs: Vec<std::path::PathBuf> = config.skills.dirs.iter()
        .map(|d| std::path::PathBuf::from(d))
        .collect();

    let mut agent = oben_agent::Agent::new(oben_agent::AgentConfig::from_app_config(
        &config,
        system_prompt,
        max_iterations,
        max_messages,
        skills_dirs,
        transport,
        tools,
        oben_agent::AgentCallbacks::default(),
    )).await?;

    let response = agent
        .turn(
            prompt,
            stream,
            stream.then(|| {
                Box::new(|text: &str| {
                    print!("{}", text);
                    std::io::stdout().flush().ok();
                }) as oben_models::StreamDeltaCallback
            }),
            None, // no interrupt from CLI
        )
        .await?;

    if !stream {
        println!("\n{}", response);
    } else {
        println!();
    }

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
            println!("  📦 {} - {}", tool.name, tool.description);
        }
    }
    Ok(())
}

fn list_skills() -> Result<()> {
    let skills = oben_skills::builtin_skills();
    println!("Built-in skills ({}):", skills.len());
    for skill in skills {
        println!(
            "  📖 {} ({}) - {}",
            skill.name, skill.category, skill.description
        );
    }
    Ok(())
}

// ── Sessions ────────────────────────────────────────────────────────────

fn list_sessions() -> Result<()> {
    let mut session_manager = oben_sessions::DBSessionManager::new()?;
    session_manager.init()?;
    let sessions = session_manager.list_sessions(None);
    if sessions.is_empty() {
        println!("No sessions found.");
    } else {
        println!("Sessions ({}):", sessions.len());
        for s in sessions {
            let marker = session_manager
                .active_session()
                .and_then(|a| {
                    if a.id == s.id {
                        Some(" ← active")
                    } else {
                        None
                    }
                })
                .unwrap_or("");
            println!("  📄 {} — {} messages{}", s.name, s.message_count, marker);
        }
    }
    Ok(())
}

async fn run_compact_session(session_key: Option<&str>, focus_topic: Option<&str>) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let mut sm = oben_sessions::DBSessionManager::new()?;

    let active_id = sm.active().map(|s| s.id.clone());
    let target: String = match session_key {
        Some(key) => key.to_string(),
        None => active_id.unwrap_or_else(|| "active".to_string()),
    };
    let target_ref = target.as_str();

    let session = sm.clone_session(target_ref).ok_or_else(|| {
        anyhow::anyhow!(
            "Session not found: {} (run `oben sessions list` to see available sessions)",
            target
        )
    })?;

    if session.message_count() < 8 {
        println!(
            "Session has only {} message(s). Minimum 8 required for compaction.",
            session.message_count()
        );
        return Ok(());
    }

    println!(
        "Compacting session '{}' ({} messages)...",
        session.name,
        session.message_count()
    );

    let transport = create_transport(&config, "", &oben_tools::ToolRegistry::new());
    let comp_config = oben_agent::compact::CompactCofig {
        context_length: config.context.context_length,
        threshold_percent: config.context.threshold_percent,
        ..oben_agent::compact::CompactCofig::default()
    };

    let result = oben_agent::compact_session_messages(
        &transport,
        &session.messages,
        &comp_config,
        session.memory_context.as_deref(),
        focus_topic,
        1,
    )
    .await?;

    if let Some(s) = sm.session_mut(&session.id) {
        s.messages = result.messages;
        s.updated_at = chrono::Utc::now();
        if let Some(summary) = result.summary {
            s.memory_context = Some(summary.clone());
            let old_msg_count = session.messages.len();
            s.summary_chunks.push(oben_models::SummaryChunk {
                from: 1,
                to: old_msg_count as i64,
                summary,
            });
        }
    }
    sm.save_session(Some(&session.id))?;

    println!("✓ Compaction complete:");
    println!(
        "  Before: {} messages, ~{} tokens",
        result.stats.original_count, result.stats.original_tokens
    );
    println!(
        "  After:  {} messages, ~{} tokens",
        result.stats.compacted_count, result.stats.compacted_tokens
    );
    println!(
        "  Saved:  {:.0}% tokens ({} tool results pruned)",
        result.stats.savings_pct, result.stats.pruned_tool_results
    );
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
    let mut sm = oben_sessions::DBSessionManager::new()?;
    sm.init()?;
    sm.delete(session_key)?;
    println!("Deleted session '{}'", session_key);
    Ok(())
}

fn dump_session(session_key: Option<&str>) -> Result<()> {
    let mut sm = oben_sessions::DBSessionManager::new()?;
    sm.load(None)?;

    let active_id = sm.active().map(|s| s.id.clone());
    let target: String = match session_key {
        Some(key) => key.to_string(),
        None => active_id.clone().unwrap_or_else(|| "active".to_string()),
    };

    let session_id = sm.find_key(&target).ok_or_else(|| {
        anyhow::anyhow!(
            "Session not found: {}. Run `oben sessions list` to see available sessions",
            target
        )
    })?;

    let sessions: Vec<oben_models::Session> = sm.list_sessions_full();
    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?
        .clone();

    let session_name = session
        .metadata
        .title
        .as_deref()
        .unwrap_or(&session.id)
        .replace(" ", "-");
    let filename = format!(
        "{}/dump-{}-{}.json",
        std::env::current_dir().unwrap().display(),
        session_name,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );

    let dump: serde_json::Value = serde_json::json!({
        "id": session.id,
        "name": session.name,
        "title": session.metadata.title,
        "message_count": session.messages.len(),
        "messages": session.messages,
    });

    let json = serde_json::to_string_pretty(&dump)?;
    std::fs::write(&filename, &json)?;

    println!(
        "Dumped {} messages from '{}' to {}",
        session.messages.len(),
        session.metadata.title.as_deref().unwrap_or(&session.name),
        filename
    );
    Ok(())
}

// ── Models ──────────────────────────────────────────────────────────────

async fn run_models(action: ModelsCommand) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let transport = create_transport(&config, "", &oben_tools::ToolRegistry::new());

    match action {
        ModelsCommand::List => {
            println!("Fetching models from provider...\n");
            let models = transport.list_models().await?;
            println!("Found {} model(s):\n", models.data.len());

            let headers = &["ID", "Max Tokens", "Owned By"];
            let rows: Vec<Vec<String>> = models
                .data
                .iter()
                .map(|m| {
                    vec![
                        m.id.clone(),
                        m.max_model_len
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "N/A".to_string()),
                        m.owned_by.clone(),
                    ]
                })
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
                        vec![
                            "Created".to_string(),
                            chrono::DateTime::from_timestamp(m.created as i64, 0)
                                .map(|d| d.to_string())
                                .unwrap_or("unknown".to_string()),
                        ],
                        vec!["Owned By".to_string(), m.owned_by],
                        vec![
                            "Max Model Length".to_string(),
                            m.max_model_len
                                .map(|t| t.to_string())
                                .unwrap_or("N/A".to_string()),
                        ],
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

fn collect_tool_defs(registry: &oben_tools::ToolRegistry) -> Vec<oben_models::ToolMeta> {
    registry.list_tools()
}

fn create_transport(
    config: &oben_config::AppConfig,
    system_prompt: &str,
    tools: &oben_tools::ToolRegistry,
) -> std::sync::Arc<dyn oben_models::TransportProvider + Send + Sync> {
    let tool_defs = collect_tool_defs(tools);
    oben_transport::Transport::from_config_with_tools_via_registry(
        &config.model,
        system_prompt,
        &tool_defs,
    )
}

// ── Goals ──────────────────────────────────────────────────────────────────

/// Get a default JsonGoalStore instance.
fn goal_store() -> Result<oben_goals::JsonGoalStore> {
    oben_goals::JsonGoalStore::default_store()
}

async fn goal_start(goal: &str, max_turns: Option<usize>) -> Result<()> {
    let goal_id = format!(
        "goal-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        Uuid::new_v4().as_simple()
    );
    let store = goal_store()?;

    // Create goal record
    store.create_goal(&goal_id, goal, max_turns.unwrap_or(20), None)?;

    // Decomposer: call LLM to break goal into plan nodes.
    // This mirrors `create_plan_from_goal()` but executed synchronously so the CLI
    // blocks until the plan is created and printed before the background loop starts.
    let config = oben_config::AppConfig::load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);
    let tool_defs = collect_tool_defs(&tools);

    let system_prompt = oben_config::defaults::default_system_prompt();
    // Decomposer doesn't need tools — just LLM JSON generation
    let decomposer_transport = std::sync::Arc::new(oben_transport::Transport::from_config(
        &config.model,
        system_prompt.clone(),
    ));
    // transport is used later in the closure when creating goal_agent
    let _transport = oben_transport::Transport::from_config_with_tools_via_registry(
        &config.model,
        &system_prompt,
        &tool_defs,
    );

    let plan_prompt = format!(
        "You are a planner. Break the following goal into a step-by-step plan.\n\n\
         Return ONLY a JSON array. No prose, no explanation, no markdown.\n\
         Each item must have:\n\
         {{\"title\": \"step name\", \"description\": \"what to do\", \"sub_tasks\": []}}\n\n\
         Rules:\n\
         - Each top-level item is a self-contained task\n\
         - sub_tasks is an array of nested task strings (may be empty)\n\
         - Keep tasks specific and actionable\n\n\
         Goal: {}",
        goal
    );

    let plan_messages = vec![oben_models::Message::system(&system_prompt), oben_models::Message::user(&plan_prompt)];

    let plan_llm_result = decomposer_transport
        .chat(&plan_messages, &oben_models::CallMode::Fresh(goal_id.clone()))
        .await?;

    let json_text = plan_llm_result.text.trim();
    // Strip markdown code fences if present
    let json_text = json_text.strip_prefix("```json").unwrap_or(json_text);
    let json_text = json_text.strip_prefix("```").unwrap_or(json_text);
    let json_text = json_text.strip_suffix("```").unwrap_or(json_text).trim();

    if json_text.is_empty() {
        anyhow::bail!(
            "LLM returned empty response. Try again. Full response len: {}",
            plan_llm_result.text.len()
        );
    }

    tracing::info!("Plan LLM response (len={}):\n{}", json_text.len(), json_text);

    let parsed_nodes: Vec<oben_goals::PlanNode> =
        match serde_json::from_str::<Vec<serde_json::Value>>(json_text) {
            Ok(items) => {
                let mut nodes = Vec::new();
                for item in items {
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled")
                        .to_string();
                    let mut node = oben_goals::PlanNode::new(&title);
                    if let Some(sub_tasks) = item.get("sub_tasks") {
                        if let Some(arr) = sub_tasks.as_array() {
                            for sub in arr {
                                let sub_title = if sub.is_string() {
                                    sub.as_str().unwrap_or("").to_string()
                                } else if sub.is_object() {
                                    sub.get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Untitled")
                                        .to_string()
                                } else {
                                    continue;
                                };
                                node.push_sub_node(oben_goals::PlanNode::new(&sub_title));
                            }
                        }
                    }
                    nodes.push(node);
                }
                println!("  Plan created: {} nodes", nodes.len());
                nodes
            }
            Err(e) => anyhow::bail!("Failed to parse plan from LLM: {}", e),
        };

    // Build plan and save via store
    let mut plan = oben_goals::PlanState::new(goal);
    for node in parsed_nodes {
        plan.add_node(node);
    }
    store.save_plan(&goal_id, &plan)?;

    // Save initial goal state to disk
    use oben_goals::GoalState;
    let initial_state = GoalState {
        goal: goal.to_string(),
        status: oben_goals::GoalStatus::Active,
        turns_used: 0,
        max_turns: max_turns.unwrap_or(20),
        created_at: chrono::Utc::now(),
        last_turn_at: None,
        last_verdict: None,
        last_reason: None,
        paused_reason: None,
        consecutive_parse_failures: 0,
    };
    store.save_goal_state(&goal_id, &initial_state)?;

    println!("Goal created: {} (id: {})", goal, goal_id);
    println!("  Status: Active");
    println!("  Max turns: {}", max_turns.unwrap_or(20));
    println!("");

    let mut tools = oben_tools::ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut tools);
    let tools = std::sync::Arc::new(tools);
    let sp = oben_config::defaults::default_system_prompt();
    let transport =
          oben_transport::Transport::from_config_with_tools_via_registry(&config.model, &sp, &tool_defs);

    let max_iterations = config.max_iterations.unwrap_or(50);
    let max_messages = config.context.max_messages.unwrap_or(100);

    let loop_config =   oben_goals::GoalLoopConfig {
        max_turns: max_turns.unwrap_or(20),
        system_prompt: Some(sp.clone()),
        auto_save: true,
    };

    let result =   oben_goals::goal_loop::run_goal_loop(
        goal,
        &goal_id,
        &loop_config,
        &store,
        move |prompt| {
            let transport = transport.clone();
            let tools = tools.clone();
            let sp = sp.clone();
            let max_iterations = max_iterations;
            let max_messages = max_messages;
            let prompt_owned = prompt.to_string();
            let config = config.clone();
            async move {
                let skills_dirs: Vec<std::path::PathBuf> = config.skills.dirs.iter()
                    .map(|d| std::path::PathBuf::from(d))
                    .collect();
                let mut goal_agent =   oben_agent::Agent::new(oben_agent::AgentConfig::from_app_config(
                    &config,
                    sp,
                    max_iterations,
                    max_messages,
                    skills_dirs,
                    transport,
                    tools,
                    oben_agent::AgentCallbacks::default(),
                ))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

                goal_agent
                    .turn_with_message(
                        oben_models::Message::user(&prompt_owned),
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
        },
    )
    .await;

    match result {
        Ok((_plan, gs)) => {
            println!("");
            let status_str = match gs.status {
                oben_goals::GoalStatus::Active => "active",
                oben_goals::GoalStatus::Done => "done",
                oben_goals::GoalStatus::Paused => "paused",
                oben_goals::GoalStatus::Cleared => "cleared",
            };
            println!("Goal '{}' completed. Status: {}", goal, status_str);
            println!("  Turns: {}/{}", gs.turns_used, gs.max_turns);
            if let Some(ref reason) = gs.paused_reason {
                println!("  Paused: {}", reason);
            }
        }
        Err(e) => eprintln!("Goal '{}' failed: {}", goal, e),
    }

    Ok(())
}

fn goal_list(status: Option<&str>) -> Result<()> {
    let store = goal_store()?;
    let goals = store.list_goals(status)?;

    if goals.is_empty() {
        println!("No goals found.");
    } else {
        println!("Goals ({}):\n", goals.len());
        for g in goals {
            let owner_str = g.owner.as_deref().map(|s| &s[..8]).unwrap_or("-");
            println!("  ID: {} | Owner: {} | Goal: {}", g.id, owner_str, g.text);
        }
    }
    Ok(())
}

fn goal_status(goal_id: &str) -> Result<()> {
    let store = goal_store()?;

    match store.get_goal(goal_id)? {
        Some((text, status, max_turns, turns_used, paused_reason)) => {
            println!("Goal: {}", text);
            println!("  ID: {}", goal_id);
            println!("  Status: {}", status);
            println!("  Turns: {}/{}", turns_used, max_turns);
            if let Some(ref reason) = paused_reason {
                println!("  Paused: {}", reason);
            }
        }
        None => {
            println!("Goal '{}' not found.", goal_id);
            println!("Use `oben goals list` to see available goals.");
        }
    }
    Ok(())
}

async fn goal_pause(id: &str) -> Result<()> {
    let store = goal_store()?;
    let goal_id = resolve_if_active(&store, id)?;
    if store.get_goal(&goal_id)?.is_some() {
        store.pause_goal(&goal_id, "user-paused")?;
        println!("Goal paused: {}", goal_id);
    } else {
        anyhow::bail!(
            "Goal not found: {}. Run `oben goals start` to create one.",
            goal_id
        );
    }
    Ok(())
}

async fn goal_resume(id: &str, reset: bool) -> Result<()> {
    let store = goal_store()?;
    let goal_id = resolve_if_active(&store, id)?;
    if store.get_goal(&goal_id)?.is_some() {
        store.resume_goal(&goal_id, reset)?;
        println!("Goal resumed: {} (budget reset: {})", goal_id, reset);
    } else {
        anyhow::bail!("Paused goal not found: {}.", goal_id);
    }
    Ok(())
}

async fn goal_clear(id: &str) -> Result<()> {
    let store = goal_store()?;
    let goal_id = resolve_if_active(&store, id)?;
    if store.get_goal(&goal_id)?.is_some() {
        store.delete_goal(&goal_id)?;
        println!("Goal cleared: {}", goal_id);
    } else {
        anyhow::bail!("Goal not found: {}.", goal_id);
    }
    Ok(())
}

/// Resolve "active" to the most recent active goal from store, otherwise use the id as-is.
fn resolve_if_active(store: &oben_goals::JsonGoalStore, id: &str) -> Result<String> {
    if id == "active" {
        let goal_id = store
            .list_goals(Some("active"))?
            .into_iter()
            .next()
            .map(|g| g.id)
            .ok_or_else(|| {
                anyhow::anyhow!("No active goal found. Run `oben goals start` first.")
            })?;
        Ok(goal_id)
    } else {
        Ok(id.to_string())
    }
}
// ── Cron ───────────────────────────────────────────────────────────────────

fn cron_store() -> std::sync::Arc<CronStore> {
    let dir = CronStore::default_path();
    std::sync::Arc::new(CronStore::new(dir).unwrap_or_else(|e| {
        eprintln!("Error initializing cron store: {}", e);
        std::process::exit(1)
    }))
}

fn cron_list(all: bool) -> Result<()> {
    let store = cron_store();
    let jobs = store.list_jobs(all);
    if jobs.is_empty() {
        println!("No cron jobs.");
    } else {
        println!("Cron jobs ({}):\n", jobs.len());
        for job in &jobs {
            let status = match (&job.state, job.enabled) {
                (oben_cron::JobState::Completed, _) => "✅ completed",
                (oben_cron::JobState::Error, _) => "❌ error",
                (oben_cron::JobState::Paused, true) => "⏸️  paused",
                (oben_cron::JobState::Scheduled, true) => "▶️  active",
                (oben_cron::JobState::Scheduled, false) => "⏸️  paused (disabled)",
                _ => "❓ unknown",
            };
            let next_run = job
                .next_run_at
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "N/A".to_string());
            let last_run = job
                .last_run_at
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".to_string());
            let error_str = if let Some(ref err) = job.last_error {
                format!("\n    Error: {}", err)
            } else {
                String::new()
            };
            println!(
                "  {} {} — {}\n    Schedule: {}\n    Created: {}\n    Next: {}\n    Last run: {}{}",
                job.id,
                job.name,
                status,
                job.schedule,
                job.created_at.format("%Y-%m-%d %H:%M"),
                next_run,
                last_run,
                error_str,
            );
        }
    }
    Ok(())
}

fn cron_create(
    schedule: &str,
    prompt: Option<&str>,
    name: Option<&str>,
    repeat: Option<u32>,
) -> Result<()> {
    let store = cron_store();
    let prompt_text = prompt.unwrap_or("Check for updates and summarize anything new.");
    let job_name = name.unwrap_or("untitled").to_string();
    let job = CronJob::new(job_name, prompt_text.to_string(), schedule, repeat)?;
    store.create(job.clone())?;
    println!("Created cron job '{}':", job.id);
    println!("  Name: {}", job.name);
    println!("  Schedule: {}", job.schedule);
    if let Some(next) = job.next_run_at {
        println!("  Next run: {}", next.format("%Y-%m-%d %H:%M"));
    }
    Ok(())
}

fn cron_pause(id: &str) -> Result<()> {
    let store = cron_store();
    store.pause(id)?;
    println!("Paused job '{}'.", id);
    Ok(())
}

fn cron_resume(id: &str) -> Result<()> {
    let store = cron_store();
    store.resume(id)?;
    println!("Resumed job '{}'.", id);
    Ok(())
}

fn cron_remove(id: &str) -> Result<()> {
    let store = cron_store();
    store.remove(id)?;
    println!("Removed job '{}'.", id);
    Ok(())
}

/// Run the cron tick manually — process all due jobs.
fn cron_tick() -> Result<()> {
    let store = cron_store();
    let due = store.get_due_jobs();
    if due.is_empty() {
        println!("No jobs due.");
    } else {
        let now = chrono::Utc::now();

        let ober_exec = oben_cron::cron_exec_binary();

        println!(
            "cron tick at {}: running {} due job(s)...",
            now.format("%H:%M:%S"),
            due.len()
        );
        for job in &due {
            let _prompt = job.prompt.clone();
            match store.advance_job(&job.id, &ober_exec) {
                Ok(()) => println!("  Job '{}' advanced to next run", job.id),
                Err(e) => println!("  Job '{}': error: {}", job.id, e),
            }
        }
    }
    Ok(())
}

// ── Cron daemon management ────────────────────────────────────────────

fn cron_pid_path() -> std::path::PathBuf {
    CronStore::default_path().join("cron.pid")
}

/// Check if the cron daemon process is running by reading the PID file.
pub fn is_cron_running() -> Option<u32> {
    let pid_path = cron_pid_path();
    if !pid_path.exists() {
        return None;
    }
    let pid: u32 = std::fs::read_to_string(&pid_path)
        .ok()?
        .trim()
        .parse()
        .ok()?;

    // Check if process exists by sending signal 0
    let res = std::process::Command::new("kill")
        .args(&["-0", &pid.to_string()])
        .output();
    match res {
        Ok(out) if out.status.success() => Some(pid),
        Ok(_) => None,
        Err(_) => None,
    }
}

fn find_cron_binary() -> Option<std::path::PathBuf> {
    let candidates = ["target/debug/oben-cron", "target/release/oben-cron"];
    for c in &candidates {
        let p = std::path::PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    let out = std::process::Command::new("which")
        .args(&["oben-cron"])
        .output()
        .ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        let path = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Start the cron daemon as a background process.
/// The daemon process becomes its own process group (not a child).
pub fn cron_start() -> Result<()> {
    let _pid_path = cron_pid_path();

    // Already running?
    if let Some(_pid) = is_cron_running() {
        println!("Cron daemon is already running (PID {})", _pid);
        return Ok(());
    }

    let binary = match find_cron_binary() {
        Some(b) => b,
        None => {
            println!("oben-cron binary not found; building...");
            let output = std::process::Command::new("cargo")
                .args(&["build", "--package", "oben-cron"])
                .output()
                .map_err(|e| anyhow::anyhow!("Failed to run cargo: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("{}", stderr);
                anyhow::bail!("Build of oben-cron failed");
            }
            std::path::PathBuf::from("target/debug/oben-cron")
        }
    };

    println!("Starting cron daemon...");
    let mut child = std::process::Command::new(&binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null()) // detach from stdin
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start cron daemon: {}", e))?;

    let started_pid = child.id();

    // Wait for PID file to be written
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if is_cron_running().is_some() {
            let _ = child.kill(); // daemon now has its own PID file
            println!("Cron daemon started (PID {}).", started_pid);
            return Ok(());
        }
    }

    let _ = child.kill();
    anyhow::bail!("Cron daemon started but PID file was not written.");
}

fn cron_info() -> Result<()> {
    let pid_path = cron_pid_path();

    if let Some(pid) = is_cron_running() {
        println!("Cron daemon: active (PID {})", pid);
        println!("  PID file: {:?}", pid_path);
        if pid_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&pid_path) {
                println!("  PID file contents: {}", content.trim());
            }
        }
        // Show job count
        let store = CronStore::new(CronStore::default_path()).ok();
        if let Some(s) = store {
            let jobs = s.list_jobs(false);
            println!("  Active jobs: {}", jobs.len());
            let all_jobs = s.list_jobs(true);
            println!("  Total jobs: {}", all_jobs.len());
        }
    } else {
        println!("Cron daemon: inactive (not running)");
        if pid_path.exists() {
            println!("  Stale PID file exists: {:?}", pid_path);
            let _ = std::fs::remove_file(&pid_path);
            println!("  Removed stale PID file.");
        }
    }

    Ok(())
}
