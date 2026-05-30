/// Skill content preprocessing.
/// Maps to `agent/skill_preprocessing.py`.
///
/// Applies two transformations to skill content before it is used as
/// instructions or sent to the LLM:
///
/// 1. **Template variable substitution** — replaces `${HERMES_SKILL_DIR}`
///    and `${HERMES_SESSION_ID}` with concrete values.
/// 2. **Inline shell expansion** — executes `!`command`` snippets and
///    replaces them with their stdout (e.g. `"Today is `date +%Y-%m-%d`").

use regex::Regex;
use std::path::Path;
use std::process::Command;

// ── Constants ───────────────────────────────────────────────────────────────

/// Maximum output size for inline shell (prevents runaway commands from
/// blowing out the context window).
const INLINE_SHELL_MAX_OUTPUT: usize = 4_000;

/// Default timeout for inline shell commands (seconds).
const DEFAULT_SHELL_TIMEOUT: u64 = 10;

// ── Regex patterns ──────────────────────────────────────────────────────────

/// Matches `${HERMES_SKILL_DIR}` / `${HERMES_SESSION_ID}` tokens.
static TEMPLATE_VAR_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"\$\{(HERMES_SKILL_DIR|HERMES_SESSION_ID)\}").unwrap());

/// Matches inline shell snippets like:  !`date +%Y-%m-%d`
/// Non-greedy, single-line only — no newlines inside the backticks.
static INLINE_SHELL_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"!`([^`\n]+)`").unwrap());

// ── Template variables ──────────────────────────────────────────────────────

/// Configuration controlling which preprocessing features are enabled.
///
/// Mirrors the `skills` section of config.yaml:
/// ```yaml
/// skills:
///   template_vars: true
///   inline_shell: false
///   inline_shell_timeout: 10
/// ```
#[derive(Debug, Clone)]
pub struct PreprocessingConfig {
    pub template_vars: bool,
    pub inline_shell: bool,
    pub inline_shell_timeout: u64,
}

impl Default for PreprocessingConfig {
    fn default() -> Self {
        Self {
            template_vars: true,   // enabled by default
            inline_shell: false,   // disabled by default (security)
            inline_shell_timeout: DEFAULT_SHELL_TIMEOUT,
        }
    }
}

/// Substitute `${HERMES_SKILL_DIR}` and `${HERMES_SESSION_ID}` tokens in skill content.
///
/// Only substitutes tokens for which a concrete value is available.
/// Unresolved tokens are left in place so the author can spot them.
pub fn substitute_template_vars(
    content: &str,
    skill_dir: Option<&Path>,
    session_id: Option<&str>,
) -> String {
    if content.is_empty() {
        return content.to_string();
    }

    let skill_dir_str = skill_dir.and_then(|p| p.to_str()).map(|s| s.to_string());

    TEMPLATE_VAR_RE
        .replace_all(content, |caps: &regex::Captures| {
            let token = caps.get(1).unwrap().as_str();
            match token {
                "HERMES_SKILL_DIR" => {
                    skill_dir_str.as_deref().unwrap_or(caps.get(0).unwrap().as_str())
                }
                "HERMES_SESSION_ID" => {
                    session_id.unwrap_or(caps.get(0).unwrap().as_str())
                }
                _ => caps.get(0).unwrap().as_str(),
            }
            .to_string()
        })
        .into_owned()
}

// ── Inline shell ────────────────────────────────────────────────────────────

/// Execute a single inline shell snippet and return its stdout (trimmed).
///
/// Failures return a short `[inline-shell error: ...]` marker instead of
/// raising, so one bad snippet can't wreck the whole skill message.
fn run_inline_shell(command: &str, cwd: Option<&Path>, timeout: u64) -> String {
    let _timeout = std::cmp::max(1, timeout);

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let result = cmd.output();

    let output = match result {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if stdout.is_empty() && !out.stderr.is_empty() {
                String::from_utf8_lossy(&out.stderr).trim().to_string()
            } else {
                stdout
            }
        }
        Err(e) => return format!("[inline-shell error: {}]", e),
    };

    // Cap output to prevent runaway commands from blowing out the context
    if output.len() > INLINE_SHELL_MAX_OUTPUT {
        format!(
            "{}...[truncated]",
            &output[..INLINE_SHELL_MAX_OUTPUT]
        )
    } else {
        output
    }
}

/// Expand inline shell snippets in content.
///
/// Replaces every `!`cmd`` with its stdout.  Runs each snippet with the
/// skill directory as CWD so relative paths in the snippet work as expected.
pub fn expand_inline_shell(
    content: &str,
    skill_dir: Option<&Path>,
    timeout: u64,
) -> String {
    if !content.contains("!`") {
        return content.to_string();
    }

    INLINE_SHELL_RE
        .replace_all(content, |caps: &regex::Captures| {
            let cmd = caps.get(1).unwrap().as_str().trim();
            if cmd.is_empty() {
                return String::new();
            }
            run_inline_shell(cmd, skill_dir, timeout)
        })
        .into_owned()
}

// ── Full preprocessing ──────────────────────────────────────────────────────

/// Apply configured SKILL.md template and inline-shell preprocessing.
///
/// This is the main entry point used when building skill instructions.
pub fn preprocess_skill_content(
    content: &str,
    skill_dir: Option<&Path>,
    session_id: Option<&str>,
    config: &PreprocessingConfig,
) -> String {
    if content.is_empty() {
        return content.to_string();
    }

    let mut result = content.to_string();

    if config.template_vars {
        result = substitute_template_vars(&result, skill_dir, session_id);
    }
    if config.inline_shell {
        result = expand_inline_shell(&result, skill_dir, config.inline_shell_timeout);
    }

    result
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_preprocess_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    // ── Template variable substitution ────────────────────────────────────

    #[test]
    fn test_substitute_skill_dir() {
        let dir = temp_dir("skill_dir");
        let content = "Your skill lives at ${HERMES_SKILL_DIR}";
        let result = substitute_template_vars(content, Some(&dir), None);
        assert!(result.contains(dir.to_string_lossy().as_ref()));
        assert!(!result.contains("${HERMES_SKILL_DIR}"));
    }

    #[test]
    fn test_substitute_session_id() {
        let content = "Session: ${HERMES_SESSION_ID}";
        let result = substitute_template_vars(content, None, Some("sess-123"));
        assert_eq!(result, "Session: sess-123");
        assert!(!result.contains("${HERMES_SESSION_ID}"));
    }

    #[test]
    fn test_substitute_unresolved_token() {
        let content = "Session: ${HERMES_SESSION_ID}";
        let result = substitute_template_vars(content, None, None);
        // Unresolved token should be left as-is
        assert_eq!(result, "Session: ${HERMES_SESSION_ID}");
    }

    #[test]
    fn test_substitute_no_tokens() {
        let content = "Just plain text, no tokens.";
        let result = substitute_template_vars(content, None, None);
        assert_eq!(result, content);
    }

    #[test]
    fn test_substitute_empty_content() {
        let result = substitute_template_vars("", None, None);
        assert_eq!(result, "");
    }

    #[test]
    fn test_substitute_multiple_tokens() {
        let dir = temp_dir("multi");
        let content = "${HERMES_SKILL_DIR} - ${HERMES_SESSION_ID}";
        let result = substitute_template_vars(
            content,
            Some(&dir),
            Some("test-sess"),
        );
        assert!(result.contains(dir.to_string_lossy().as_ref()));
        assert!(result.contains("test-sess"));
    }

    // ── Inline shell expansion ────────────────────────────────────────────

    #[test]
    fn test_expand_inline_shell_basic() {
        let content = "Today is !`date +%Y-%m-%d`";
        let result = expand_inline_shell(content, None, 5);
        assert!(result.starts_with("Today is "));
        assert!(!result.contains("!`"));
    }

    #[test]
    fn test_expand_inline_shell_no_snippets() {
        let content = "No inline shell here.";
        let result = expand_inline_shell(content, None, 5);
        assert_eq!(result, content);
    }

    #[test]
    fn test_expand_inline_shell_empty_command() {
        // Empty command !`` is not matched by regex (requires 1+ chars), so unchanged
        let content = "Before !`` after";
        let result = expand_inline_shell(content, None, 5);
        assert_eq!(result, content);
    }

    #[test]
    fn test_expand_inline_shell_error() {
        // Bash outputs error to stderr, which run_inline_shell captures
        let content = "Run !`nonexistent_command_xyz_12345`";
        let result = expand_inline_shell(content, None, 5);
        assert!(result.contains("command not found"));
    }

    #[test]
    fn test_expand_inline_shell_with_cwd() {
        let dir = temp_dir("shell_cwd");
        // Create a test file
        fs::write(dir.join("test.txt"), "hello from cwd").ok();
        let content = "Content: !`cat test.txt`";
        let result = expand_inline_shell(content, Some(&dir), 5);
        assert!(result.contains("hello from cwd"));
    }

    #[test]
    fn test_expand_inline_shell_truncation() {
        // Generate output longer than INLINE_SHELL_MAX_OUTPUT (4000)
        let content = "Long: !`python3 -c \"print('A' * 5000)\"`";
        let result = expand_inline_shell(content, None, 5);
        assert!(result.len() < 5000);
        assert!(result.contains("[truncated]"));
    }

    // ── Full preprocessing ────────────────────────────────────────────────

    #[test]
    fn test_preprocess_no_config() {
        let content = "Simple content";
        let config = PreprocessingConfig::default();
        let result = preprocess_skill_content(content, None, None, &config);
        assert_eq!(result, content);
    }

    #[test]
    fn test_preprocess_template_vars_only() {
        let dir = temp_dir("preprocess_tv");
        let content = "Skill: ${HERMES_SKILL_DIR}";
        let config = PreprocessingConfig {
            template_vars: true,
            inline_shell: false,
            inline_shell_timeout: 5,
        };
        let result = preprocess_skill_content(&content, Some(&dir), None, &config);
        assert!(result.contains(dir.to_string_lossy().as_ref()));
    }

    #[test]
    fn test_preprocess_template_vars_disabled() {
        let dir = temp_dir("preprocess_tv_off");
        let content = "Skill: ${HERMES_SKILL_DIR}";
        let config = PreprocessingConfig {
            template_vars: false,
            inline_shell: false,
            inline_shell_timeout: 5,
        };
        let result = preprocess_skill_content(&content, Some(&dir), None, &config);
        // Should remain unchanged since template_vars is disabled
        assert!(result.contains("${HERMES_SKILL_DIR}"));
    }

    #[test]
    fn test_preprocess_inline_shell_enabled() {
        let config = PreprocessingConfig {
            template_vars: false,
            inline_shell: true,
            inline_shell_timeout: 5,
        };
        let content = "Today is !`date +%Y-%m-%d`";
        let result = preprocess_skill_content(&content, None, None, &config);
        assert!(!result.contains("!`"));
        assert!(result.starts_with("Today is "));
    }

    #[test]
    fn test_preprocess_inline_shell_disabled() {
        let config = PreprocessingConfig {
            template_vars: false,
            inline_shell: false,
            inline_shell_timeout: 5,
        };
        let content = "Today is !`date +%Y-%m-%d`";
        let result = preprocess_skill_content(&content, None, None, &config);
        assert_eq!(result, content);
    }

    #[test]
    fn test_preprocess_combined() {
        let dir = temp_dir("preprocess_combined");
        fs::write(dir.join("greeting.txt"), "hello").ok();
        let config = PreprocessingConfig {
            template_vars: true,
            inline_shell: true,
            inline_shell_timeout: 5,
        };
        let content = "Skill: ${HERMES_SKILL_DIR}, greeting: !`cat greeting.txt`";
        let result = preprocess_skill_content(&content, Some(&dir), Some("s1"), &config);
        assert!(result.contains(dir.to_string_lossy().as_ref()));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_preprocess_empty_content() {
        let config = PreprocessingConfig::default();
        let result = preprocess_skill_content("", None, None, &config);
        assert_eq!(result, "");
    }

    // ── Regex edge cases ──────────────────────────────────────────────────

    #[test]
    fn test_inline_shell_multiple_snippets() {
        let config = PreprocessingConfig {
            template_vars: false,
            inline_shell: true,
            inline_shell_timeout: 5,
        };
        let content = "A: !`echo one` and B: !`echo two`";
        let result = preprocess_skill_content(&content, None, None, &config);
        assert!(result.contains("one"));
        assert!(result.contains("two"));
        assert!(!result.contains("!`"));
    }

    #[test]
    fn test_inline_shell_multiline_ignored() {
        // Inline shell should NOT span multiple lines
        let content = "First line !`echo first`\nSecond !`echo\nmulti`";
        let result = expand_inline_shell(content, None, 5);
        // The multiline snippet should produce an error or partial match
        assert!(!result.contains("!`echo first`"));
    }

    #[test]
    fn test_template_var_nested_braces() {
        let content = "outer ${HERMES_SKILL_DIR} inner ${HERMES_SESSION_ID}";
        let dir = temp_dir("nested");
        let result = substitute_template_vars(content, Some(&dir), Some("sid"));
        assert!(result.contains(dir.to_string_lossy().as_ref()));
        assert!(result.contains("sid"));
    }
}
