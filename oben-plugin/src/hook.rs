/// Hook types and hook invocation system.
///
/// Maps to Hermes' `VALID_HOOKS` set and `invoke_hook()` function.
/// 17 hook types that fire at specific lifecycle points.
/// Each callback wrapped in try/except so one misbehaving plugin
/// cannot break the core agent loop.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use tracing::warn;

/// All 17 lifecycle hook types in Hermes.
///
/// Each hook fires at a specific point in the agent lifecycle.
/// Some hooks (transform_* and pre_llm_call) may return values
/// that modify the agent's behavior.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookType {
    /// Fired before a tool call. Can block with {"action": "block", "message": "..."}
    /// Kwargs: tool_name, args, session_key
    PreToolCall,

    /// Fired after a tool call.
    /// Kwargs: tool_name, args, result, session_key
    PostToolCall,

    /// Transform terminal output before displaying to user.
    /// First non-None result wins.
    /// Kwargs: output (str), session_key
    TransformTerminalOutput,

    /// Transform tool result before returning to model.
    /// Kwargs: tool_name, result, session_key
    TransformToolResult,

    /// Replace LLM response text. First non-None wins.
    /// Useful for vocabulary/personality transformation.
    /// Kwargs: text (str), session_key
    TransformLlmOutput,

    /// Inject context into user message before LLM call.
    /// Returns dict/string to inject context.
    /// Kwargs: messages, session_key
    PreLlmCall,

    /// Fired after LLM call completes.
    /// Kwargs: response, session_key
    PostLlmCall,

    /// Fired before API request (network).
    /// Kwargs: url, method, headers, body, session_key
    PreApiRequest,

    /// Fired after API response received.
    /// Kwargs: status_code, response, session_key
    PostApiRequest,

    /// Fired when session starts.
    /// Kwargs: session_key, session_id
    OnSessionStart,

    /// Fired when session ends (normal).
    /// Kwargs: session_key, session_id
    OnSessionEnd,

    /// Fired when session finalizes (cleanup).
    /// Kwargs: session_key, session_id
    OnSessionFinalize,

    /// Fired when session resets.
    /// Kwargs: session_key
    OnSessionReset,

    /// Fired when subagent stops.
    /// Kwargs: subagent_id, parent_session_key
    SubagentStop,

    /// Fired once per incoming gateway message before auth/pairing.
    /// Can return {"action": "skip"/"rewrite"}.
    /// Kwargs: event, gateway, session_store
    PreGatewayDispatch,

    /// Fired before approval request (dangerous command).
    /// Observers only: return values ignored.
    /// Kwargs: command, description, pattern_key, pattern_keys, session_key, surface
    PreApprovalRequest,

    /// Fired after approval response.
    /// Observers only: return values ignored.
    /// Kwargs: command, description, pattern_key, pattern_keys, session_key, surface, choice
    PostApprovalResponse,
}

impl HookType {
    /// Return all valid hook types.
    pub fn all() -> &'static [Self] {
        &[
            Self::PreToolCall,
            Self::PostToolCall,
            Self::TransformTerminalOutput,
            Self::TransformToolResult,
            Self::TransformLlmOutput,
            Self::PreLlmCall,
            Self::PostLlmCall,
            Self::PreApiRequest,
            Self::PostApiRequest,
            Self::OnSessionStart,
            Self::OnSessionEnd,
            Self::OnSessionFinalize,
            Self::OnSessionReset,
            Self::SubagentStop,
            Self::PreGatewayDispatch,
            Self::PreApprovalRequest,
            Self::PostApprovalResponse,
        ]
    }

    /// Return true if this hook can return a transformation value.
    pub fn is_transform(&self) -> bool {
        matches!(self, Self::TransformTerminalOutput | Self::TransformToolResult | Self::TransformLlmOutput)
    }

    /// Return true if this hook can block/prevent action.
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::PreToolCall | Self::PreGatewayDispatch)
    }
}

impl fmt::Display for HookType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreToolCall => write!(f, "pre_tool_call"),
            Self::PostToolCall => write!(f, "post_tool_call"),
            Self::TransformTerminalOutput => write!(f, "transform_terminal_output"),
            Self::TransformToolResult => write!(f, "transform_tool_result"),
            Self::TransformLlmOutput => write!(f, "transform_llm_output"),
            Self::PreLlmCall => write!(f, "pre_llm_call"),
            Self::PostLlmCall => write!(f, "post_llm_call"),
            Self::PreApiRequest => write!(f, "pre_api_request"),
            Self::PostApiRequest => write!(f, "post_api_request"),
            Self::OnSessionStart => write!(f, "on_session_start"),
            Self::OnSessionEnd => write!(f, "on_session_end"),
            Self::OnSessionFinalize => write!(f, "on_session_finalize"),
            Self::OnSessionReset => write!(f, "on_session_reset"),
            Self::SubagentStop => write!(f, "subagent_stop"),
            Self::PreGatewayDispatch => write!(f, "pre_gateway_dispatch"),
            Self::PreApprovalRequest => write!(f, "pre_approval_request"),
            Self::PostApprovalResponse => write!(f, "post_approval_response"),
        }
    }
}

impl FromStr for HookType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pre_tool_call" => Ok(Self::PreToolCall),
            "post_tool_call" => Ok(Self::PostToolCall),
            "transform_terminal_output" => Ok(Self::TransformTerminalOutput),
            "transform_tool_result" => Ok(Self::TransformToolResult),
            "transform_llm_output" => Ok(Self::TransformLlmOutput),
            "pre_llm_call" => Ok(Self::PreLlmCall),
            "post_llm_call" => Ok(Self::PostLlmCall),
            "pre_api_request" => Ok(Self::PreApiRequest),
            "post_api_request" => Ok(Self::PostApiRequest),
            "on_session_start" => Ok(Self::OnSessionStart),
            "on_session_end" => Ok(Self::OnSessionEnd),
            "on_session_finalize" => Ok(Self::OnSessionFinalize),
            "on_session_reset" => Ok(Self::OnSessionReset),
            "subagent_stop" => Ok(Self::SubagentStop),
            "pre_gateway_dispatch" => Ok(Self::PreGatewayDispatch),
            "pre_approval_request" => Ok(Self::PreApprovalRequest),
            "post_approval_response" => Ok(Self::PostApprovalResponse),
            _ => Err(anyhow::anyhow!("Unknown hook type: '{}'", s)),
        }
    }
}

/// Type alias for a hook callback function.
///
/// Callbacks receive hook arguments as `&Value` and return `Option<Value>`.
/// Returning `None` means "no effect", returning `Some(value)` means
/// "transform the output" (for transform hooks) or "inject context"
/// (for pre_llm_call).
///
/// Each callback is wrapped in a try/except by `invoke_hook()` so a
/// panic in one plugin doesn't break the agent loop.
pub type HookCallback = Arc<dyn Fn(&Value) -> Result<Option<Value>> + Send + Sync>;

/// Invoke all registered callbacks for a hook type.
///
/// Each callback is called with `args` and wrapped in try/except.
/// Returns a list of all non-None return values from callbacks.
///
/// For transform hooks (transform_llm_output, etc.), the first non-None
/// return value wins (like Hermes' "first non-None wins" semantics).
///
/// For pre_llm_call, return values are injected as context into the
/// user message (preserves prompt cache prefix).
pub fn invoke_hook(
    callbacks: &[HookCallback],
    args: &Value,
) -> Vec<Value> {
    let mut results = Vec::new();

    for cb in callbacks {
        // Wrap in catch_unwind to prevent panics from killing the agent
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cb(args)
        }));

        let value = match result {
            Ok(Ok(ret)) => ret,
            Ok(Err(e)) => {
                warn!("Hook callback raised: {}", e);
                None
            }
            Err(panic) => {
                let msg = panic.downcast_ref::<&str>()
                    .map(|s| *s)
                    .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                    .unwrap_or("unknown panic");
                warn!("Hook callback panicked: {:?}", msg);
                None
            }
        };

        if let Some(v) = value {
            results.push(v);
        }
    }

    results
}

/// Parse a hook type from string.
pub fn parse_hook_type(s: &str) -> Result<HookType> {
    HookType::from_str(s)
}

/// Check if a hook type string is valid.
pub fn is_valid_hook_type(s: &str) -> bool {
    HookType::from_str(s).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_hook_type_from_str() {
        /// given: hook type strings
        /// when: parse_hook_type() is called
        /// then: returns correct HookType variants
        assert_eq!(parse_hook_type("pre_tool_call").unwrap(), HookType::PreToolCall);
        assert_eq!(parse_hook_type("post_tool_call").unwrap(), HookType::PostToolCall);
        assert_eq!(parse_hook_type("transform_llm_output").unwrap(), HookType::TransformLlmOutput);
        assert!(parse_hook_type("invalid_hook").is_err());
    }

    #[test]
    fn test_hook_type_display() {
        /// given: HookType variants
        /// when: Display::fmt is called
        /// then: returns snake_case string
        assert_eq!(HookType::PreToolCall.to_string(), "pre_tool_call");
        assert_eq!(HookType::TransformLlmOutput.to_string(), "transform_llm_output");
    }

    #[test]
    fn test_invoke_hook_empty() {
        /// given: no callbacks
        /// when: invoke_hook() is called
        /// then: returns empty list
        let args = json!({"test": "value"});
        let results = invoke_hook(&[], &args);
        assert!(results.is_empty());
    }

    #[test]
    fn test_invoke_hook_collects_results() {
        /// given: two callbacks that return Some values
        /// when: invoke_hook() is called
        /// then: collects both results
        let callback1 = Arc::new(|_args: &Value| {
            Ok(Some(json!("result1")))
        });
        let callback2 = Arc::new(|_args: &Value| {
            Ok(Some(json!("result2")))
        });

        let args = json!({});
        let results = invoke_hook(&[callback1, callback2], &args);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "result1");
        assert_eq!(results[1], "result2");
    }

    #[test]
    fn test_invoke_hook_ignores_none() {
        /// given: callbacks that return None
        /// when: invoke_hook() is called
        /// then: None returns are skipped
        let callback = Arc::new(|_args: &Value| {
            Ok(None)
        });

        let args = json!({});
        let results = invoke_hook(&[callback], &args);
        assert!(results.is_empty());
    }

    #[test]
    fn test_invoke_hook_catches_panic() {
        /// given: a callback that panics
        /// when: invoke_hook() is called
        /// then: panic is caught and result is skipped
        let callback = Arc::new(|_args: &Value| {
            panic!("test panic");
        });

        let args = json!({});
        let results = invoke_hook(&[callback], &args);
        assert!(results.is_empty());
    }

    #[test]
    fn test_invoke_hook_catches_error() {
        /// given: a callback that returns Err
        /// when: invoke_hook() is called
        /// then: error is caught and result is skipped
        let callback = Arc::new(|_args: &Value| {
            Err(anyhow::anyhow!("test error"))
        });

        let args = json!({});
        let results = invoke_hook(&[callback], &args);
        assert!(results.is_empty());
    }

    #[test]
    fn test_hook_type_is_transform() {
        /// given: various hook types
        /// when: is_transform() is called
        /// then: returns true only for transform hooks
        assert!(HookType::TransformLlmOutput.is_transform());
        assert!(HookType::TransformTerminalOutput.is_transform());
        assert!(HookType::TransformToolResult.is_transform());
        assert!(!HookType::PreToolCall.is_transform());
        assert!(!HookType::OnSessionStart.is_transform());
    }

    #[test]
    fn test_hook_type_is_blocking() {
        /// given: various hook types
        /// when: is_blocking() is called
        /// then: returns true only for blocking hooks
        assert!(HookType::PreToolCall.is_blocking());
        assert!(HookType::PreGatewayDispatch.is_blocking());
        assert!(!HookType::PostToolCall.is_blocking());
        assert!(!HookType::OnSessionEnd.is_blocking());
    }

    #[test]
    fn test_all_hook_types() {
        /// given: HookType::all()
        /// when: called
        /// then: returns exactly 17 hook types
        let all = HookType::all();
        assert_eq!(all.len(), 17);
    }
}
