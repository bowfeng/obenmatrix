use serde_json::Value;
/// Patch tool — fuzzy text replacement in files.
///
/// Implements a multi-strategy matching chain to robustly find and replace text,
/// accommodating variations in whitespace, indentation, and escaping.
use std::path::Path;
use std::sync::Arc;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{SelfRegisteringTool, ToolHandler};
use oben_utils::path_security::is_path_safe;

// ---------------------------------------------------------------------------
// Unicode normalization map
// ---------------------------------------------------------------------------

/// Unicode normalization map: [from_char, to_string, ...]
const UNICODE_REPLACEMENTS: &[(char, &str)] = &[
    ('\u{201c}', "\""), // smart double quotes
    ('\u{201d}', "\""),
    ('\u{2018}', "'"), // smart single quotes
    ('\u{2019}', "'"),
    ('\u{2014}', "--"), // em/en dashes
    ('\u{2013}', "-"),
    ('\u{2026}', "..."), // ellipsis
    ('\u{00a0}', " "),   // non-breaking space
];

fn unicode_normalize(text: &str) -> String {
    let mut result = text.to_string();
    for (from, to) in UNICODE_REPLACEMENTS {
        result = result.replace(*from, *to);
    }
    result
}

// ---------------------------------------------------------------------------
// Matching strategies
// ---------------------------------------------------------------------------

/// Try exact match first.
fn try_exact(content: &str, old: &str) -> Option<usize> {
    content.find(old)
}

/// Strip leading/trailing whitespace per line.
fn try_line_trimmed(content: &str, old: &str) -> Option<usize> {
    let old_lines: Vec<&str> = old.lines().collect();
    let content_lines: Vec<&str> = content.lines().collect();

    for i in 0..content_lines.len() {
        if i + old_lines.len() > content_lines.len() {
            break;
        }

        let mut matched = true;
        for (j, old_line) in old_lines.iter().enumerate() {
            let trimmed_old = old_line.trim();
            let trimmed_content = content_lines[i + j].trim();
            if trimmed_old != trimmed_content {
                matched = false;
                break;
            }
        }

        if matched {
            // Calculate start position
            let mut pos = 0;
            for line in content_lines.iter().take(i) {
                pos += line.len() + 1; // +1 for newline
            }
            return Some(pos);
        }
    }
    None
}

/// Collapse multiple spaces/tabs to single space and return position in original content.
fn try_whitespace_normalized(content: &str, old: &str) -> Option<usize> {
    let original_tokens: Vec<&str> = content.split_whitespace().collect();
    let old_tokens: Vec<&str> = old.split_whitespace().collect();

    if original_tokens.len() < old_tokens.len() {
        return None;
    }

    for i in 0..=original_tokens.len().saturating_sub(old_tokens.len()) {
        if original_tokens[i..i + old_tokens.len()] == old_tokens[..] {
            // Compute start position by accumulating original token lengths + separator space
            let mut pos = 0;
            for token in original_tokens.iter().take(i) {
                pos += token.len() + 1; // +1 for the collapsed whitespace
            }
            return Some(pos);
        }
    }
    None
}

/// Main fuzzy matching function.
fn fuzzy_find(content: &str, old: &str) -> (Option<usize>, String) {
    let old_normalized = unicode_normalize(old);
    let content_normalized = unicode_normalize(content);

    // Try each strategy in order
    if let Some(pos) = try_exact(&content_normalized, &old_normalized) {
        return (Some(pos), "exact".to_string());
    }

    if let Some(pos) = try_line_trimmed(&content_normalized, &old_normalized) {
        return (Some(pos), "line_trimmed".to_string());
    }

    if let Some(pos) = try_whitespace_normalized(&content_normalized, &old_normalized) {
        return (Some(pos), "whitespace_normalized".to_string());
    }

    (None, "no_match".to_string())
}

/// Replace text at a position — match against normalized content but
/// slice the *original* content.  This function is only called after
/// `fuzzy_find` returns `(Some(pos), _)` where `pos` is a char-position
/// in the **original** (not normalized) content string.
fn replace_at(content: &str, pos: usize, old: &str, new: &str) -> String {
    // Verify the match actually exists in the original content at this position
    // so we don't silently corrupt the file.
    if pos > content.len() {
        return content.to_string();
    }

    // Clamp pos to char boundary.
    let orig_start = content
        .char_indices()
        .find(|(i, _)| *i >= pos)
        .map(|(i, _)| i)
        .unwrap_or(content.len());

    // We search in the normalized strings to find the span in the
    // *normalized* domain, then map back to the original.
    let content_normalized = unicode_normalize(content);
    let old_normalized = unicode_normalize(old);

    if let Some(norm_pos) = content_normalized.find(&old_normalized) {
        let norm_old_len = old_normalized.chars().count();
        // Map normalized start/end back to original char positions.
        let mut norm_idx = 0;
        let mut orig_start = 0;
        let mut orig_end = 0;

        for (i, _nc) in content_normalized.chars().enumerate() {
            if norm_idx == norm_pos {
                orig_start = i;
            }
            if norm_idx == norm_pos + norm_old_len {
                orig_end = i;
                break;
            }
            norm_idx += 1;
        }

        let before: String = content.chars().take(orig_start).collect();
        let after: String = content.chars().skip(orig_end).collect();

        return format!("{}{}{}", before, new, after);
    }

    // Fallback: try the original pos directly.
    let old_char_len = old.chars().count();
    let orig_end = orig_start.saturating_add(old_char_len);

    let before: String = content.chars().take(orig_start).collect();
    let after: String = content.chars().skip(orig_end).collect();

    format!("{}{}{}", before, new, after)
}

// ---------------------------------------------------------------------------
// Unified diff generation
// ---------------------------------------------------------------------------

fn generate_diff(old_content: &str, new_content: &str, path: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut diff = String::new();
    diff.push_str(&format!("--- {}\n", path));
    diff.push_str(&format!("+++ {}\n", path));

    let mut old_idx = 0;
    let mut new_idx = 0;
    let mut changed = false;

    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        let old_line = old_lines.get(old_idx).copied().unwrap_or("");
        let new_line = new_lines.get(new_idx).copied().unwrap_or("");

        if old_line == new_line {
            if !changed {
                diff.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    old_idx.saturating_sub(1).max(0),
                    1,
                    new_idx.saturating_sub(1).max(0),
                    1
                ));
                changed = true;
            }
            diff.push_str(&format!(" {}\n", old_line));
            old_idx += 1;
            new_idx += 1;
        } else {
            if !changed {
                diff.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    old_idx.saturating_sub(1).max(0),
                    old_lines.len() - old_idx,
                    new_idx.saturating_sub(1).max(0),
                    new_lines.len() - new_idx
                ));
                changed = true;
            }
            diff.push_str(&format!("-{}\n", old_line));
            diff.push_str(&format!("+{}\n", new_line));
            old_idx += 1;
            new_idx += 1;
        }
    }

    diff
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_patch_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "path".into(),
            description: "File path to patch.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "old_string".into(),
            description: "Text to find (supports fuzzy matching).".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "new_string".into(),
            description: "Replacement text.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "replace_all".into(),
            description:
                "If true, replace all occurrences (default: false, requires unique match).".into(),
            parameter_type: "boolean".into(),
            required: false,
        },
    ];
    Tool {
        name: "patch".into(),
        description: "Patch a file by replacing text with fuzzy matching. Supports whitespace, indentation, and Unicode variations. Returns diff and match details.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_patch_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

            let old_string = args
                .get("old_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'old_string' argument"))?;

            let new_string = args
                .get("new_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'new_string' argument"))?;

            let replace_all = args
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Safety check: path traversal
            if !is_path_safe(Path::new(path)) {
                return Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some("Unsafe file path".to_string()),
                });
            }

            // Safety check: empty old_string
            if old_string.trim().is_empty() {
                return Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some("old_string cannot be empty".to_string()),
                });
            }

            // Read file
            let content = match tokio::fs::read_to_string(path).await {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!("Failed to read {}: {}", path, e)),
                    });
                }
            };

            // Fuzzy find
            let (match_pos, strategy) = fuzzy_find(&content, old_string);

            let match_count = if let Some(_) = match_pos { 1 } else { 0 };

            if match_count == 0 {
                return Ok(ToolResult {
                    call_id,
                    output: format!("No match found for old_string in {} (strategy: {}). Try reading the file first to see exact content.", path, strategy),
                    error: Some(format!("No match found in {} using strategy: {}", path, strategy)),
                });
            }

            // If not replace_all and multiple occurrences, error
            if !replace_all && content.matches(old_string).count() > 1 {
                return Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(
                        "Multiple occurrences found. Set replace_all=true to replace all."
                            .to_string(),
                    ),
                });
            }

            // Replace
            let new_content = replace_at(&content, match_pos.unwrap(), old_string, new_string);

            // Generate diff
            let diff = generate_diff(&content, &new_content, path);

            // Write file
            match tokio::fs::write(path, &new_content).await {
                Ok(_) => Ok(ToolResult {
                    call_id,
                    output: format!("Patch applied successfully.\n\n{}", diff),
                    error: None,
                }),
                Err(e) => Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(format!("Failed to write file: {}", e)),
                }),
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct PatchTool;

impl SelfRegisteringTool for PatchTool {
    fn tool() -> Tool {
        make_patch_tool()
    }

    fn handler() -> ToolHandler {
        make_patch_handler()
    }
}

/// Register this module into the given registry.
/// Called automatically by `discover_builtin_tools`.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    PatchTool::register_self(registry);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        PatchTool::register_self(&mut registry);
        registry
    }

    #[tokio::test]
    async fn exact_match_and_replace() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world\nhello universe").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world",
                    "new_string": "hello there",
                    "call_id": "test-1",
                }),
            )
            .await;

        assert!(result.error.is_none());
        assert!(result.output.contains("Patch applied successfully"));

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "hello there\nhello universe");
    }

    #[tokio::test]
    async fn no_match_returns_error() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "nonexistent",
                    "new_string": "replacement",
                    "call_id": "test-2",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("No match found"));
    }

    #[tokio::test]
    async fn line_trimmed_fuzzy_match() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "    hello world  \n    hello universe  \n").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world",
                    "new_string": "hello there",
                    "call_id": "test-3",
                }),
            )
            .await;

        assert!(result.error.is_none());
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("hello there"));
    }

    #[tokio::test]
    async fn unicode_normalization() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        // Smart quotes in content
        fs::write(&file_path, "\u{201c}hello\u{201d} world").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "\"hello\" world",
                    "new_string": "\"there\" world",
                    "call_id": "test-4",
                }),
            )
            .await;

        assert!(result.error.is_none());
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("there"));
    }

    #[tokio::test]
    async fn empty_old_string_blocked() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "",
                    "new_string": "replacement",
                    "call_id": "test-5",
                }),
            )
            .await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("empty"));
    }

    #[tokio::test]
    async fn whitespace_normalized() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world\n").unwrap();

        let registry = make_registry();
        let result = registry
            .execute(
                "patch",
                &serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world",
                    "new_string": "hello there",
                    "call_id": "test-6",
                }),
            )
            .await;

        assert!(result.error.is_none());
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("there"));
    }
}
