/// Rich callback system for platform integration.
///
/// Mirrors Hermes' 11+ callback parameters consolidated into a single struct.
/// All callbacks are `Option<Box<dyn Fn(...) + Send + Sync>>`.
///
/// Note: `AgentCallbacks` is NOT `Clone` because `Box<dyn Fn>` doesn't impl `Clone`.
/// This is by design - callers create the callbacks once and pass by reference.

/// Agent callbacks — rich set for platform integration.
#[derive(Default)]
pub struct AgentCallbacks {
    /// Tool progress: (tool_name, args_preview)
    pub tool_progress: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool started: (tool_name, args_json)
    pub tool_start: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool completed: (tool_name, args_json, result)
    pub tool_complete: Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>,
    /// Thinking/thought stream delta
    pub thinking: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Reasoning stream delta
    pub reasoning: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Clarification request: (question, choices) -> user answer
    pub clarify: Option<Box<dyn Fn(&str, &[String]) -> String + Send + Sync>>,
    /// Step-by-step status message
    pub step: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Token stream delta (for TTS etc.)
    pub stream_delta: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Interim assistant message (non-streaming, full text)
    pub interim_assistant: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Tool generation event: (tool_name, call_id)
    pub tool_gen: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Lifecycle status: (level, message)
    pub status: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Verbose print — always visible even during streaming
    pub vprint: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl AgentCallbacks {
    /// Call tool progress callback.
    pub fn call_tool_progress(&self, tool_name: &str, args_preview: &str) {
        if let Some(cb) = &self.tool_progress {
            cb(tool_name, args_preview);
        }
    }

    /// Call tool start callback.
    pub fn call_tool_start(&self, tool_name: &str, args_json: &str) {
        if let Some(cb) = &self.tool_start {
            cb(tool_name, args_json);
        }
    }

    /// Call tool complete callback.
    pub fn call_tool_complete(&self, tool_name: &str, args_json: &str, result: &str) {
        if let Some(cb) = &self.tool_complete {
            cb(tool_name, args_json, result);
        }
    }

    /// Call thinking callback.
    pub fn call_thinking(&self, text: &str) {
        if let Some(cb) = &self.thinking {
            cb(text);
        }
    }

    /// Call reasoning callback.
    pub fn call_reasoning(&self, text: &str) {
        if let Some(cb) = &self.reasoning {
            cb(text);
        }
    }

    /// Call clarify callback — returns user answer or empty string.
    pub fn call_clarify(&self, question: &str, choices: &[String]) -> String {
        if let Some(cb) = &self.clarify {
            cb(question, choices)
        } else {
            String::new()
        }
    }

    /// Call step callback.
    pub fn call_step(&self, message: &str) {
        if let Some(cb) = &self.step {
            cb(message);
        }
    }

    /// Call stream delta callback.
    pub fn call_stream_delta(&self, text: &str) {
        if let Some(cb) = &self.stream_delta {
            cb(text);
        }
    }

    /// Call interim assistant callback.
    pub fn call_interim_assistant(&self, text: &str) {
        if let Some(cb) = &self.interim_assistant {
            cb(text);
        }
    }

    /// Call tool gen callback.
    pub fn call_tool_gen(&self, tool_name: &str, call_id: &str) {
        if let Some(cb) = &self.tool_gen {
            cb(tool_name, call_id);
        }
    }

    /// Call status callback.
    pub fn call_status(&self, level: &str, message: &str) {
        if let Some(cb) = &self.status {
            cb(level, message);
        }
    }

    /// Call vprint callback.
    pub fn call_vprint(&self, message: &str) {
        if let Some(cb) = &self.vprint {
            cb(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_callbacks_noop() {
        let cb = AgentCallbacks::default();
        cb.call_tool_progress("shell", "ls");
        cb.call_tool_start("shell", "{}");
        cb.call_tool_complete("shell", "{}", "done");
        cb.call_thinking("thinking...");
        cb.call_reasoning("reasoning...");
        cb.call_step("step 1");
        cb.call_stream_delta("hello");
        cb.call_interim_assistant("hi");
        cb.call_tool_gen("shell", "call-1");
        cb.call_status("lifecycle", "started");
        cb.call_vprint("verbose");
        let answer = cb.call_clarify("which one?", &["a".to_string(), "b".to_string()]);
        assert_eq!(answer, "");
    }

    #[test]
    fn test_tool_progress_callback() {
        let invoked = std::sync::Arc::new(std::sync::Mutex::new(false));
        let invoked_clone = invoked.clone();
        let cb = AgentCallbacks {
            tool_progress: Some(Box::new(move |name: &str, preview: &str| {
                assert_eq!(name, "shell");
                assert_eq!(preview, "ls");
                *invoked_clone.lock().unwrap() = true;
            })),
            ..Default::default()
        };
        cb.call_tool_progress("shell", "ls");
        assert!(*invoked.lock().unwrap());
    }

    #[test]
    fn test_clarify_callback_returns_answer() {
        let cb = AgentCallbacks {
            clarify: Some(Box::new(move |_q: &str, _c: &[String]| "A".to_string())),
            ..Default::default()
        };
        assert_eq!(cb.call_clarify("pick one?", &["A".into(), "B".into()]), "A");
    }
}
