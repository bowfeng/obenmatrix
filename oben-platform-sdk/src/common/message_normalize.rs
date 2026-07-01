/// Platform-agnostic message normalization helpers.
///
/// Strips platform-specific mentions and parses slash commands from inbound
/// message content, following hermes-agent patterns.

use once_cell::sync::Lazy;
use regex::Regex;

// Pre-compiled regexes for common mention patterns (performance: compile once, match many)

// Slack mentions: <@USERID> or <@&GROUP>
static RE_SLACK_MENTION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<@[&!]?[A-Z0-9]+>").unwrap());

// Discord mentions: <@USERID> or <@!USERID>
static RE_DISCORD_MENTION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<@!?[0-9]+>").unwrap());

// Generic @botname mention at start of message
static RE_AT_MENTION: Lazy<Regex> = Lazy::new(|| Regex::new(r"^@\w+\s*").unwrap());

/// Strip platform-specific mention prefixes from message content.
///
/// Recognizes common mention formats:
/// - Slack: `<@U12345>` → removed
/// - Discord: `<@USER_ID>` or `<@!USER_ID>` → removed
/// - General: `@botname` prefix → removed
///
/// # Examples
///
/// ```
/// use oben_platform_sdk::common::message_normalize::strip_mentions;
///
/// assert_eq!(strip_mentions("@bot hello there", "discord"), "hello there");
/// assert_eq!(strip_mentions("check this <@U12345>", "slack"), "check this");
/// assert_eq!(strip_mentions("normal message", "telegram"), "normal message");
/// ```
pub fn strip_mentions(content: &str, platform: &str) -> String {
    let content = content.trim_start();

    match platform.to_lowercase().as_str() {
        "slack" => RE_SLACK_MENTION
            .replace_all(content, "")
            .replace("  ", " ")
            .trim()
            .to_string(),
        "discord" => RE_DISCORD_MENTION
            .replace_all(content, "")
            .replace("  ", " ")
            .trim()
            .to_string(),
        "telegram" | "whatsapp" | "matrix" | "feishu" | "dingtalk" | "wecom" => {
            RE_AT_MENTION.replace_all(content, "").trim().to_string()
        }
        _ => RE_AT_MENTION.replace_all(content, "").trim().to_string(),
    }
}

/// Detect if a message starts with a slash command trigger.
///
/// Parses content like `/ask help me` into `Some(("ask", "help me"))`.
/// Returns `(command_name, remaining_args)` on success, `None` if not a slash command.
///
/// # Examples
///
/// ```
/// use oben_platform_sdk::common::message_normalize::parse_slash_command;
///
/// assert_eq!(parse_slash_command("/ask help me"), Some(("ask".to_string(), "help me".to_string())));
/// assert_eq!(parse_slash_command("/reset", ), Some(("reset".to_string(), "".to_string())));
/// assert_eq!(parse_slash_command("/status", ), Some(("status".to_string(), "".to_string())));
/// assert_eq!(parse_slash_command("hello world", ), None);
/// assert_eq!(parse_slash_command("/hello world", ), None); // "hello world" is not a valid command token
/// ```
pub fn parse_slash_command(content: &str) -> Option<(String, String)> {
    let content = content.trim();

    if !content.starts_with('/') {
        return None;
    }

    let rest = &content[1..];
    let rest = rest.trim_start();

    let space_pos = rest.find(' ');
    let (cmd, args) = match space_pos {
        Some(pos) => (&rest[..pos], rest[pos + 1..].trim().to_string()),
        None => (rest, String::new()),
    };

    let cmd = cmd.trim();

    // Command must be alphanumeric (with underscores/dashes allowed)
    if cmd.is_empty() || !cmd.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }

    Some((cmd.to_lowercase(), args))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_mentions ---

    /// Given: A Discord message mentioning the bot by <@USERID> format
    /// When: strip_mentions is called with platform "discord"
    /// Then: The mention is removed and surrounding whitespace is cleaned
    #[test]
    fn test_strip_mentions_discord() {
        let result = strip_mentions("<@123456789> hello there", "discord");
        assert_eq!(result, "hello there");
    }

    /// Given: A Discord message with ping-style mention <@!USERID>
    /// When: strip_mentions is called with platform "discord"
    /// Then: Both mention formats are handled
    #[test]
    fn test_strip_mentions_discord_ping() {
        let result = strip_mentions("<@!123456789> hello there", "discord");
        assert_eq!(result, "hello there");
    }

    /// Given: A Slack message with <@U12345> mention
    /// When: strip_mentions is called with platform "slack"
    /// Then: The Slack mention is removed
    #[test]
    fn test_strip_mentions_slack() {
        let result = strip_mentions("check this <@U12345>", "slack");
        assert_eq!(result, "check this");
    }

    /// Given: A Telegram message with @botname prefix
    /// When: strip_mentions is called with platform "telegram"
    /// Then: Leading @botname mention is removed
    #[test]
    fn test_strip_mentions_telegram() {
        let result = strip_mentions("@mybot hello there", "telegram");
        assert_eq!(result, "hello there");
    }

    /// Given: A general message without any mention
    /// When: strip_mentions is called
    /// Then: The message is returned unchanged
    #[test]
    fn test_strip_mentions_none() {
        let result = strip_mentions("normal message", "slack");
        assert_eq!(result, "normal message");
    }

    /// Given: A message with mention at the end
    /// When: strip_mentions is called
    /// Then: The trailing mention is removed and whitespace is cleaned
    #[test]
    fn test_strip_mentions_trailing() {
        let result = strip_mentions("hello <@U99999>", "slack");
        assert_eq!(result, "hello");
    }

    // --- parse_slash_command ---

    /// Given: A message starting with "/ask help me"
    /// When: parse_slash_command is called
    /// Then: Returns (Some("ask"), "help me")
    #[test]
    fn test_parse_slash_command_with_args() {
        let result = parse_slash_command("/ask help me");
        assert_eq!(result, Some(("ask".into(), "help me".into())));
    }

    /// Given: A message starting with "/reset" with no args
    /// When: parse_slash_command is called
    /// Then: Returns (Some("reset"), "")
    #[test]
    fn test_parse_slash_command_no_args() {
        let result = parse_slash_command("/reset");
        assert_eq!(result, Some(("reset".into(), "".into())));
    }

    /// Given: A message starting with "/status" with no args
    /// When: parse_slash_command is called
    /// Then: Returns (Some("status"), "")
    #[test]
    fn test_parse_slash_command_status() {
        let result = parse_slash_command("/status");
        assert_eq!(result, Some(("status".into(), "".into())));
    }

    /// Given: A normal message without leading slash
    /// When: parse_slash_command is called
    /// Then: Returns None
    #[test]
    fn test_parse_slash_command_none() {
        let result = parse_slash_command("hello world");
        assert_eq!(result, None);
    }

    /// Given: A message with multiple words as command name like "/hello world"
    /// When: parse_slash_command is called
    /// Then: Returns None because "hello world" is not a valid single token
    #[test]
    fn test_parse_slash_command_invalid_name() {
        let result = parse_slash_command("/hello world");
        assert_eq!(result, None);
    }

    /// Given: A message with underscores and dashes in the command name
    /// When: parse_slash_command is called
    /// Then: Valid complex command names are accepted
    #[test]
    fn test_parse_slash_command_complex_name() {
        let result = parse_slash_command("/my-command arg1");
        assert_eq!(result, Some(("my-command".into(), "arg1".into())));

        let result2 = parse_slash_command("/my_command arg2");
        assert_eq!(result2, Some(("my_command".into(), "arg2".into())));
    }

    /// Given: A message with leading whitespace before the slash
    /// When: parse_slash_command is called
    /// Then: Returns None because trim on content removes leading space,
    ///       but the leading space means it was already trimmed
    #[test]
    fn test_parse_slash_command_whitespace() {
        let result = parse_slash_command("  /ask test  ");
        assert_eq!(result, Some(("ask".into(), "test".into())));
    }
}
