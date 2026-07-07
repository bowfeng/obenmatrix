/// CLI coordinator — I/O only. The agent owns the turn loop.
use std::sync::Arc;

use oben_agent::hooks::HookEngine;
use oben_agent::interaction::{InteractionProvider, StdioProvider};
use oben_agent::{ConversationConfig, ConversationCoordinator, ConversationResult};

/// Configuration for CLI conversation behavior.
pub struct CliConfig {
    /// Shared conversation configuration (retry, fallback, dispatch, etc.).
    pub conversation: ConversationConfig,
    /// Hook engine shared with Agent (Arc to avoid duplicate instantiation).
    pub hooks: Arc<HookEngine>,
    /// Whether to stream output.
    pub stream: bool,
    /// Maximum number of user turns in the conversation (None = unlimited).
    pub max_turns: Option<usize>,
}

impl CliConfig {
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

/// CLI coordinator — the full interactive chat loop for terminal-based interaction.
pub struct CliCoordinator {
    config: CliConfig,
    turn_count: usize,
    /// Interaction provider reused across turns (avoids re-instantiating per-call).
    interaction: StdioProvider,
}

impl CliCoordinator {
    /// Create a new CLI coordinator from conversation config and shared hooks.
    /// Hook registration happens in `on_loop_start` via the trait.
    pub fn from_conversation(
        conversation: ConversationConfig,
        hooks: Arc<HookEngine>,
        stream: bool,
        max_turns: Option<usize>,
    ) -> Self {
        Self {
            config: CliConfig {
                conversation,
                hooks,
                stream,
                max_turns,
            },
            turn_count: 0,
            interaction: StdioProvider::new(),
        }
    }

    /// Create a new CLI coordinator with explicit config.
    pub fn from_config(config: CliConfig) -> Self {
        Self {
            config,
            turn_count: 0,
            interaction: StdioProvider::new(),
        }
    }

    /// Blocking read for stdin — called from the main thread (not from an
    /// async executor). Safe because the CLI is single-threaded tokio.
    fn prompt_and_read_blocking(&self) -> Option<String> {
        self.interaction.write_raw(b"> ");
        self.interaction.flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok().and_then(|n| {
            if n == 0 {
                None // EOF
            } else {
                Some(line)
            }
        })
    }
}

#[async_trait::async_trait]
impl ConversationCoordinator for CliCoordinator {
    fn on_loop_start(&mut self) {
        // Register streaming hook on first loop start.
        if self.config.stream {
            self.config.hooks.register_streaming(Box::new(
                crate::coordinator::cli::CliStreamingHook::new(),
            ));
        }
    }

    async fn next_turn(&mut self) -> Option<String> {
        loop {
            let line = match self.prompt_and_read_blocking() {
                Some(line) => line,
                None => return None, // EOF
            };
            let trimmed = line.trim().to_string();

            // Empty input → exit
            if trimmed.is_empty() {
                return None;
            }

            // Quit commands: /quit, /q, /exit
            if matches!(trimmed.as_str(), "/quit" | "/q" | "/exit") {
                return None;
            }

            return Some(trimmed);
        }
    }

    fn on_turn_complete(
        &mut self,
        response: &str,
        _msg_count: usize,
        _turn_count: u32,
        success: bool,
    ) -> bool {
        let interaction = &self.interaction;
        interaction.print_newline();

        // In non-streaming mode, print the response directly.
        // In streaming mode, the hooks already printed delta-by-delta.
        if !self.config.stream {
            let _ = interaction.write_raw(response.as_bytes());
            interaction.flush();
        }

        if !success {
            // On error, exit the loop.
            return false;
        }

        // Check max turns.
        self.turn_count += 1;
        if let Some(max) = self.config.max_turns {
            if self.turn_count >= max {
                return false; // exit after hitting budget
            }
        }

        true // continue
    }

    fn on_loop_end(&mut self, _outcome: &ConversationResult) {
        // Nothing special needed — CLI just exits naturally.
    }
}

/// Streaming hook that writes each delta directly to stdout.
pub struct CliStreamingHook;

impl CliStreamingHook {
    pub fn new() -> Self { Self }
}

impl oben_agent::hooks::kind::Hook for CliStreamingHook {
    fn id(&self) -> &str { "cli_streaming" }
    fn priority(&self) -> u32 { 1 }
}

impl oben_agent::hooks::kind::StreamingHooks for CliStreamingHook {
    fn on_stream_delta(&self, text: &str) {
        if !text.is_empty() {
            use std::io::Write;
            let _ = std::io::stdout().write_all(text.as_bytes());
            let _ = std::io::stdout().flush();
        }
    }
}
