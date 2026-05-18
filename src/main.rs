/// ObenAgent CLI — the main entry point.
/// Maps to `hermes_cli/main.py` + `cli.py`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::Write;
use tracing::info;

#[derive(Parser)]
#[command(name = "oben", version, about = "The self-improving AI agent — Rust port of Hermes Agent")]
struct Cli {
    /// Enable verbose/debug output
    #[arg(short, long)]
    verbose: bool,
    /// Stream text output as it arrives
    #[arg(short, long)]
    stream: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive conversation
    Chat,
    /// Run a one-shot prompt
    Run {
        /// The prompt/question
        #[arg(short, long)]
        prompt: String,
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
    /// List sessions
    Sessions,
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = if cli.verbose { tracing::Level::DEBUG } else { tracing::Level::INFO };
    oben_utils::logging::init(level);

    match cli.command {
        Commands::Chat => run_chat(cli.stream).await,
        Commands::Run { prompt } => run_one_shot(&prompt, cli.stream).await,
        Commands::Setup => run_setup(),
        Commands::Config { action } => run_config(action).await,
        Commands::Tools => { list_tools(); Ok(()) }
        Commands::Skills => { list_skills(); Ok(()) }
        Commands::Sessions => { list_sessions(); Ok(()) }
        Commands::Info => { show_info(); Ok(()) },
        Commands::Models { action } => run_models(action).await,
    }
}

async fn run_chat(stream: bool) -> Result<()> {
    info!("Starting interactive chat...");

    let config = oben_config::AppConfig::load()?;
    let mut memory = oben_memory::MemoryManager::new();
    memory.load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    register_builtin_tools(&mut tools);

    // Build system prompt with skill instructions (reserved for future use)
    let _system_prompt = oben_config::defaults::default_system_prompt();

    // Start a new session (reserved for future use)
    let _session = memory.new_session(&format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")));

    println!("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");

    let mut conversation = oben_core::ConversationLoop::new(
        oben_transport::chat_completions::ChatCompletionsTransport::from_config(&config.model),
        std::sync::Arc::new(tools),
        config.max_iterations.unwrap_or(50),
        config.context.max_messages.unwrap_or(100),
    );

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

        let response = conversation.run_turn(oben_models::Message::user(input)).await?;
        println!("\n{}", response);

        // Save after each turn
        memory.save()?;
    }

    Ok(())
}

async fn run_one_shot(prompt: &str, stream: bool) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let mut memory = oben_memory::MemoryManager::new();
    memory.load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    register_builtin_tools(&mut tools);

    let mut conversation = oben_core::ConversationLoop::new(
        oben_transport::chat_completions::ChatCompletionsTransport::from_config(&config.model),
        std::sync::Arc::new(tools),
        config.max_iterations.unwrap_or(50),
        config.context.max_messages.unwrap_or(100),
    );

    let response = conversation.run_turn(oben_models::Message::user(prompt)).await?;
    println!("{}", response);

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
    register_builtin_tools(&mut tools);
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

fn list_sessions() {
    let mut memory = oben_memory::MemoryManager::new();
    memory.load().unwrap_or_default();
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
}

fn show_info() {
    println!("ObenAgent v{}", env!("CARGO_PKG_VERSION"));
    println!("Rust port of Hermes Agent by Nous Research");
    println!("\nUsage: oben <command>");
    println!("\nCommands:");
    println!("  chat    — Start an interactive conversation");
    println!("  run     — Run a one-shot prompt");
    println!("  setup   — Run setup wizard");
    println!("  config  — Show or edit configuration");
    println!("  tools   — List available tools");
    println!("  skills  — List available skills");
    println!("  sessions — List sessions");
    println!("  info     — Show agent info");
    println!("  models   — Discover models from LLM provider");
}

/// Register all built-in tools into the registry.
fn register_builtin_tools(tools: &mut oben_tools::ToolRegistry) {
    use oben_models::Tool;

    // Shell tool
    let shell_tool = Tool::builder("shell", "Execute shell commands")
        .param("command", "Shell command to execute", "string", true)
        .param("cwd", "Working directory", "string", false)
        .param("timeout", "Timeout in seconds", "number", false)
        .build();
    tools.register(shell_tool, std::sync::Arc::new(|args: serde_json::Value| {
        Box::pin(oben_tools::shell::execute_shell(args))
    }));

    // Read file tool
    let read_tool = Tool::builder("read_file", "Read the contents of a file")
        .param("path", "Path to the file", "string", true)
        .build();
    tools.register(read_tool, std::sync::Arc::new(|args: serde_json::Value| {
        Box::pin(oben_tools::read_write::read_file(args))
    }));

    // Write file tool
    let write_tool = Tool::builder("write_file", "Write content to a file")
        .param("path", "Path to write to", "string", true)
        .param("content", "Content to write", "string", true)
        .build();
    tools.register(write_tool, std::sync::Arc::new(|args: serde_json::Value| {
        Box::pin(oben_tools::read_write::write_file(args))
    }));

    // HTTP GET tool
    let http_tool = Tool::builder("http_get", "Make an HTTP GET request")
        .param("url", "URL to fetch", "string", true)
        .build();
    tools.register(http_tool, std::sync::Arc::new(|args: serde_json::Value| {
        Box::pin(oben_tools::web::http_get(args))
    }));
}

async fn run_models(action: ModelsCommand) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let transport = oben_transport::ChatCompletionsTransport::from_config(&config.model);

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
