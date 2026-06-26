use std::io::Write;

/// The communication protocol between agent engine and the external interface.
///
/// NOT a hook — this is a bidirectional interface. The engine calls methods on this trait
/// to communicate with the interface layer (CLI, TUI, Gateway).
///
/// Output methods (`print_info`, `print_newline`, `write_raw`) let each implementation
/// decide how to render: CLI writes to stdout, TUI buffers into screen panels,
/// Gateway sends platform-specific messages. Under the hood, all go through
/// `write_raw` for maximum flexibility.
///
/// `InteractionProvider` has no knowledge of streaming semantics. Streaming is handled
/// separately through the HookEngine's streaming delta hooks, which fire for each token.
/// The provider itself is used for structured I/O: read input, write prompts/info, check exit.
#[async_trait::async_trait]
pub trait InteractionProvider: Send + Sync {
    /// Read user input. Returns None when disconnected or no more input.
    async fn read_input(&self) -> Option<String>;

    /// Check if the conversation should exit.
    fn should_exit(&self, input: &str) -> bool;

    /// Write raw bytes to the output channel.
    /// Each implementation decides how to render: CLI writes to stdout,
    /// TUI buffers into message panels, Gateway sends platform messages.
    fn write_raw(&self, bytes: &[u8]);

    /// Print a prompt marker (e.g., "> ").
    fn print_prompt(&self) {
        self.write_raw(b"> ");
    }

    /// Print informational text with a trailing newline.
    fn print_info(&self, message: &str) {
        self.write_raw(message.as_bytes());
        self.print_newline();
    }

    /// Print a blank newline.
    fn print_newline(&self) {
        self.write_raw(b"\n");
    }

    /// Flush the output buffer.
    fn flush(&self);
}

/// Implementation for CLI (blocking stdin/stdout).
///
/// Uses `std::io` directly for maximum compatibility with the blocking
/// `stdin`/`stdout` available in standard CLI contexts.
pub struct StdioProvider;

impl StdioProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdioProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl InteractionProvider for StdioProvider {
    async fn read_input(&self) -> Option<String> {
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            Some(input.trim().to_string())
        } else {
            Some(String::new())
        }
    }

    fn should_exit(&self, input: &str) -> bool {
        input == "quit" || input == "exit"
    }

    fn write_raw(&self, bytes: &[u8]) {
        let _ = std::io::stdout().write_all(bytes);
        let _ = std::io::stdout().flush();
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
    }
}
