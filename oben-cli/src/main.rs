//! ObenAgent CLI — the main entry point.
//!
//! Maps to `hermes_cli/main.py` + `cli.py`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

#[derive(Parser)]
#[command(name = "oben", version, about = "The self-improving AI agent — Rust port of Hermes Agent")]
struct Cli {
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
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show current config
    Show,
    /// Edit config file
    Edit,
}

#[tokio::main]
async fn main() -> Result<()> {
    oben_utils::logging::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Chat => run_chat().await,
        Commands::Run { prompt } => run_one_shot(&prompt).await,
        Commands::Setup => run_setup(),
        Commands::Config { action } => run_config(action).await,
        Commands::Tools => list_tools(),
        Commands::Skills => list_skills(),
        Commands::Sessions => list_sessions(),
        Commands::Info => show_info(),
    }
}

async fn run_chat() -> Result<()> {
    info!("Starting interactive chat...");

    let config = oben_config::AppConfig::load()?;
    let mut memory = oben_memory::MemoryManager::new();
    memory.load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    register_builtin_tools(&mut tools);

    let mut skills = oben_skills::SkillLoader::new();
    skills.add_dir("./skills");
    let loaded_skills = skills.load_all()?;
    let mut skill_manager = oben_skills::SkillManager::new();
    skill_manager.load_skills(loaded_skills);
    // Also add builtin skills
    for skill in oben_skills::builtin_skills() {
        skill_manager.add_skill(skill);
    }

    // Build system prompt with skill instructions
    let mut system_prompt = oben_config::defaults::default_system_prompt();
    system_prompt.push_str(&skill_manager.build_skill_instructions());

    // Create transport
    let transport = oben_transport::chat_completions::ChatCompletionsTransport::from_config(&config.model);

    // Start a new session
    let session_name = if let Some(s) = memory.active_session() {
        s.name.clone()
    } else {
        "default".to_string()
    };
    let session = memory.new_session(&format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")));

    println!("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");

    let mut conversation = oben_core::ConversationLoop::new(
        transport,
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

async fn run_one_shot(prompt: &str) -> Result<()> {
    let config = oben_config::AppConfig::load()?;
    let mut memory = oben_memory::MemoryManager::new();
    memory.load()?;

    let mut tools = oben_tools::ToolRegistry::new();
    register_builtin_tools(&mut tools);

    let transport = oben_transport::chat_completions::ChatCompletionsTransport::from_config(&config.model);

    let mut conversation = oben_core::ConversationLoop::new(
        transport,
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
            println!("{}", serde_yaml::to_string_pretty(&config)?);
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
            let marker = if s.id == memory.active_session().map(|a| &a.id).unwrap_or("") {
                " ← active"
            } else {
                ""
            };
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
    println!("  info    — Show agent info");
}

/// Register all built-in tools into the registry.
fn register_builtin_tools(tools: &mut oben_tools::ToolRegistry) {
    use oben_models::Tool;

    // Shell tool
    let shell_tool = Tool::builder("shell")
        .description("Execute shell commands")
        .param("command", "Shell command to execute", "string", true)
        .param("cwd", "Working directory", "string", false)
        .param("timeout", "Timeout in seconds", "number", false)
        .build();
    tools.register(shell_tool, std::sync::Arc::new(oben_tools::shell::execute_shell));

    // Read file tool
    let read_tool = Tool::builder("read_file")
        .description("Read the contents of a file")
        .param("path", "Path to the file", "string", true)
        .build();
    tools.register(read_tool, std::sync::Arc::new(oben_tools::read_write::read_file));

    // Write file tool
    let write_tool = Tool::builder("write_file")
        .description("Write content to a file")
        .param("path", "Path to write to", "string", true)
        .param("content", "Content to write", "string", true)
        .build();
    tools.register(write_tool, std::sync::Arc::new(oben_tools::read_write::write_file));

    // HTTP GET tool
    let http_tool = Tool::builder("http_get")
        .description("Make an HTTP GET request")
        .param("url", "URL to fetch", "string", true)
        .build();
    tools.register(http_tool, std::sync::Arc::new(oben_tools::web::http_get));
}
