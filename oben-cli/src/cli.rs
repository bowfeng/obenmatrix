//! Clap CLI definitions.
//!
//! All argument structs and subcommand enums are defined here.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oben", version, about = "The self-improving AI agent")]
pub struct Cli {
    /// Enable verbose/debug output
    #[arg(short, long)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start an interactive conversation
    Chat {
        /// Disable streaming
        #[arg(long)]
        no_stream: bool,
        /// Continue an existing session by ID or name.
        /// Without a value, continues the most recent session.
        #[arg(short, long = "continue", num_args=0..=1, default_missing_value="latest")]
        continue_session: Option<String>,
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
    /// Discover models via LLM provider
    Models {
        #[command(subcommand)]
        action: ModelsCommand,
    },
    /// Start the terminal UI
    Tui,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show current config
    Show,
    /// Edit config file
    Edit,
}

#[derive(Subcommand)]
pub enum ModelsCommand {
    /// List available models from the LLM provider
    List,
    /// Show details for a specific model
    Info {
        /// Model ID to look up
        model: String,
    },
}

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// List all sessions
    List,
    /// Compact (compress) a session using LLM summarization
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
    /// Dump session messages to a JSON file
    Dump {
        /// Session ID or name (optional, defaults to active session)
        #[arg(short, long)]
        session: Option<String>,
    },
}
