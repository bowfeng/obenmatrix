/// Fallback: parse tool call invocations from text content.
///
/// When the LLM doesn't support OpenAI-structured tool_calls, it may output
/// tool invocations as text like:
///   `"terminal" {"command": "ls"}`
///   or: `terminal({"command": "ls"})`
///   or: `<tool>terminal</tool>{"command":"ls"}`
///
/// This function scans the text for tool invocation patterns and returns
/// a list of `TransportToolCall` when the LLM outputs text-based tool calls
/// but the transport receives no structured `tool_calls`.

use oben_models::TransportToolCall;

/// Extract a complete JSON object from a string that starts with `{`.
/// Handles nested objects and strings with braces inside quoted values.
fn extract_json_object(s: &str) -> Option<&str> {
    let s = s.trim();
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    let mut start = None;
    for (i, ch) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => {
                depth += 1;
                if start.is_none() {
                    start = Some(i);
                }
            }
                '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
        }
    }
    None
}

pub fn parse_tool_calls_from_text(text: &str) -> Vec<TransportToolCall> {
    let mut tool_calls = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pattern 1: Scan entire text for `"tool_name"` followed by a JSON object `{...}`
    // Handles both `"terminal" {"command": "ls"}` and garbled output like
    // `It "terminal"analyze" "arguments"{}"` where we find the tool name in quotes
    // and then look for the nearest JSON object after it.
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        // Look for a quoted word that could be a tool name
        if chars[i] == '"' {
            // Find the end of the quoted string
            let start = i + 1;
            let mut end = start;
            while end < len && chars[end] != '"' {
                end += 1;
            }
            if end < len {
                let word = &text[start..end];
                // Check: is this a valid tool name (no spaces)?
                if !word.is_empty() && !word.contains(|c: char| c.is_whitespace())
                    && word.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                {
                    // Look ahead for a JSON object after this quoted word
                    let after_quote = end + 1;
                    // Skip past trailing quotes or other chars that might appear
                    let mut j = after_quote;
                    while j < len && chars[j] == '"' { j += 1; }
                    if j < len && chars[j] == '{' {
                        if let Some(args_str) = extract_json_object(&text[j..]) {
                            if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                                if seen.insert(word.to_string()) {
                                    tool_calls.push(TransportToolCall {
                                        id: format!("call_{}_{}", tool_calls.len(), word),
                                        tool_name: word.to_string(),
                                        arguments: args,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    // Pattern 2: `tool_name({"args": ...})` — function call style
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(paren_pos) = trimmed.find('(') {
            let before = &trimmed[..paren_pos];
            let tool_name = before.trim();
            if !tool_name.is_empty()
                && tool_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && !tool_name.contains('"')
            {
                if let Some(close_paren) = trimmed.rfind(')') {
                    let inner = &trimmed[paren_pos + 1..close_paren];
                    let inner = inner.trim();
                    if let Some(args_str) = extract_json_object(inner) {
                        if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                            if seen.insert(tool_name.to_string()) {
                                tool_calls.push(TransportToolCall {
                                    id: format!("call_{}_{}", tool_calls.len(), tool_name),
                                    tool_name: tool_name.to_string(),
                                    arguments: args,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Pattern 3: `<tool>name</tool>{"args": ...}` — XML-like tags
    if let Some(tag_start) = text.find("<tool>") {
        if let Some(tag_end_pos) = text[tag_start..].find("</tool>") {
            let name_start = tag_start + "<tool>".len();
            let name_end = tag_start + tag_end_pos;
            let tool_name = &text[name_start..name_end];
            let after_tag = &text[name_end..];
            if let Some(brace_pos) = after_tag.find('{') {
                if let Some(args_str) = extract_json_object(&after_tag[brace_pos..]) {
                    if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                        if seen.insert(tool_name.to_string()) {
                            tool_calls.push(TransportToolCall {
                                id: format!("call_{}_{}", tool_calls.len(), tool_name),
                                tool_name: tool_name.to_string(),
                                arguments: args,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pattern 4: `{"name": "tool_name", "arguments": {...}}` — raw JSON object
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                if let Some(args) = obj.get("arguments").filter(|v| v.is_object()) {
                    if seen.insert(name.to_string()) {
                        tool_calls.push(TransportToolCall {
                            id: format!("call_{}_{}", tool_calls.len(), name),
                            tool_name: name.to_string(),
                            arguments: args.clone(),
                        });
                    }
                }
            }
        }
    }

    tool_calls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quoted_name_with_json_object() {
        let text = r#"  "terminal" {"command": "ls"}"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[0].arguments["command"], "ls");
    }

    #[test]
    fn test_function_call_style() {
        let text = r#"terminal({"command": "pwd"})"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[0].arguments["command"], "pwd");
    }

    #[test]
    fn test_xml_tag_style() {
        let text = r#"<tool>terminal</tool>{"command": "ls -la"}"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn test_raw_json_object() {
        let text = r#"{"name": "terminal", "arguments": {"command": "pwd"}}"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[0].arguments["command"], "pwd");
    }

    #[test]
    fn test_no_tool_calls() {
        let text = "Hello, how can I help you?";
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_multiple_tool_calls() {
        let text = r#""terminal" {"command": "ls"}
"read_file" {"path": "README.md"}"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[1].tool_name, "read_file");
    }

    #[test]
    fn test_empty_text() {
        let calls = parse_tool_calls_from_text("");
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_fallback_id_generation() {
        let text = r#""terminal" {"command": "ls"}"#;
        let calls = parse_tool_calls_from_text(text);
        assert!(calls[0].id.starts_with("call_0_terminal"));
    }

    #[test]
    fn test_pattern2_no_duplicate_pattern1() {
        // When both patterns match, pattern 1 takes precedence
        let text = r#"terminal({"command": "ls"})"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
    }

    #[test]
    fn test_json_with_nested_objects() {
        // Nested objects inside the args should parse correctly
        let text = r#""terminal" {"command": "ls", "options": {"all": true, "human": true}}"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "terminal");
        assert_eq!(calls[0].arguments["command"], "ls");
        assert_eq!(calls[0].arguments["options"]["all"], true);
    }
}
