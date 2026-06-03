/// Verdict types for the goal judge.

/// The judge's verdict on whether a goal is satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeVerdict {
    /// The goal is fully satisfied.
    Done(String),
    /// The goal is not yet satisfied; continue working.
    Continue(String),
    /// The judge could not be reached or produced unusable output.
    Skipped(String),
}

/// Parse the judge's raw response into a verdict.
///
/// The judge is expected to return JSON like:
/// `{"done": true, "reason": "The agent successfully created the file."}`
pub fn parse_judge_response(raw: &str) -> JudgeVerdict {
    let raw = raw.trim();
    if raw.is_empty() {
        return JudgeVerdict::Continue("judge returned empty response".to_string());
    }

    // Strip markdown code fences if present
    let text = if raw.starts_with("```") {
        let stripped = raw.trim_start_matches('`');
        let stripped = stripped.trim_start();
        // Strip language tag like "json\n"
        let stripped = stripped.splitn(2, '\n').nth(1).unwrap_or(stripped);
        let end = stripped.find("```");
        match end {
            Some(pos) => &stripped[..pos],
            None => stripped,
        }
    } else {
        raw
    };

    // Try to parse the whole text as JSON
    let data: Result<serde_json::Value, _> = serde_json::from_str(text);
    let data = match data {
        Ok(d) => d,
        Err(_) => {
            // Try to find the first JSON object in the text
            let mut found = false;
            let mut json_str = String::new();
            let mut in_string = false;
            let mut escape_next = false;
            for c in text.chars() {
                if escape_next {
                    json_str.push(c);
                    escape_next = false;
                    continue;
                }
                if c == '\\' && in_string {
                    json_str.push(c);
                    escape_next = true;
                    continue;
                }
                if c == '"' {
                    in_string = !in_string;
                }
                if c == '{' && !in_string {
                    json_str.push(c);
                    found = true;
                    continue;
                }
                if found {
                    json_str.push(c);
                    if c == '}' && !in_string {
                        break;
                    }
                }
            }
            if found {
                if let Ok(v) = serde_json::from_str(&json_str) {
                    v
                } else {
                    let preview = &text[..text.len().min(200)];
                    return JudgeVerdict::Continue(format!(
                        "judge reply was not JSON: {:?}",
                        preview
                    ));
                }
            } else {
                let preview = &text[..text.len().min(200)];
                return JudgeVerdict::Continue(format!("judge reply was not JSON: {:?}", preview));
            }
        }
    };

    if !data.is_object() {
        return JudgeVerdict::Continue("judge reply was not a JSON object".to_string());
    }

    let done_val = data.get("done");
    let done = match done_val {
        Some(v) if v.is_boolean() => v.as_bool().unwrap_or(false),
        Some(v) if v.is_string() => {
            let s = v.as_str().unwrap_or("").to_lowercase();
            s == "true" || s == "yes" || s == "1" || s == "done"
        }
        _ => false,
    };

    let reason = data
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("no reason provided")
        .to_string();

    if done {
        JudgeVerdict::Done(reason)
    } else {
        JudgeVerdict::Continue(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_judge_done() {
        let response = r#"{"done": true, "reason": "All files created."}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(ref r) if r == "All files created."));
    }

    #[test]
    fn test_parse_judge_continue() {
        let response = r#"{"done": false, "reason": "Still need to write tests."}"#;
        let verdict = parse_judge_response(response);
        assert!(
            matches!(verdict, JudgeVerdict::Continue(ref r) if r == "Still need to write tests.")
        );
    }

    #[test]
    fn test_parse_judge_done_string_true() {
        let response = r#"{"done": "true", "reason": "It works."}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(_)));
    }

    #[test]
    fn test_parse_judge_done_string_yes() {
        let response = r#"{"done": "yes", "reason": "Done."}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(_)));
    }

    #[test]
    fn test_parse_judge_empty() {
        let response = "";
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Continue(ref r) if r.contains("empty")));
    }

    #[test]
    fn test_parse_judge_non_json() {
        let response = "I think the agent is still working on it.";
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Continue(_)));
    }

    #[test]
    fn test_parse_judge_with_code_fences() {
        let response = "```json\n{\"done\": true, \"reason\": \"File created.\"}\n```";
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(ref r) if r == "File created."));
    }

    #[test]
    fn test_parse_judge_no_reason() {
        let response = r#"{"done": true}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(ref r) if r == "no reason provided"));
    }

    #[test]
    fn test_parse_judge_null_reason() {
        let response = r#"{"done": true, "reason": null}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Done(ref r) if r == "no reason provided"));
    }

    #[test]
    fn test_parse_judge_done_1() {
        let response = r#"{"done": 1, "reason": "done."}"#;
        let verdict = parse_judge_response(response);
        assert!(matches!(verdict, JudgeVerdict::Continue(_)));
    }
}
