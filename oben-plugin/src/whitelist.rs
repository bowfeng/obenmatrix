//! Thread-local tool whitelist for plugin-controlled tool access.
//!
//! Maps to Hermes' `set_thread_tool_whitelist()` / `clear_thread_tool_whitelist()`
//! and the `get_pre_tool_call_block_message()` blocking mechanism.
//!
/// A thread-local whitelist that restricts which tools a plugin context
/// (e.g., sub-agent) is allowed to call.

use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    /// Current thread's tool whitelist.
    /// None = no restriction (all tools allowed).
    /// Some(set) = only tools in the set are allowed.
    static TOOL_WHITELIST: RefCell<Option<HashSet<String>>> = RefCell::new(None);
}

/// Set the tool whitelist for the current thread.
///
/// When set, only tools whose names appear in the whitelist can be called.
/// Returns a Guard that clears the whitelist when dropped.
pub fn set_thread_tool_whitelist(tools: HashSet<String>) -> WhitelistGuard {
    TOOL_WHITELIST.with(|whitelist| {
        *whitelist.borrow_mut() = Some(tools);
    });
    WhitelistGuard
}

/// Clear the tool whitelist for the current thread.
pub fn clear_thread_tool_whitelist() {
    TOOL_WHITELIST.with(|whitelist| {
        *whitelist.borrow_mut() = None;
    });
}

/// Check if a tool is allowed by the current thread's whitelist.
///
/// Returns `Ok(())` if the tool is allowed, or an error with the
/// block message if the whitelist exists and the tool is not in it.
pub fn check_tool_allowed(tool_name: &str) -> Result<(), BlockMessage> {
    TOOL_WHITELIST.with(|whitelist| {
        if let Some(ref allowed) = *whitelist.borrow() {
            if !allowed.contains(tool_name) {
                let allowed_list: Vec<String> = allowed.iter().cloned().collect();
                return Err(BlockMessage {
                    tool: tool_name.to_string(),
                    allowed: allowed_list.clone(),
                    message: format!(
                        "Tool '{}' is not allowed. Allowed tools: [{}]",
                        tool_name,
                        allowed_list.join(", ")
                    ),
                });
            }
        }
        Ok(())
    })
}

/// Get the current block message for a tool call.
///
/// Used by `pre_tool_call` hook handlers to return blocking messages.
pub fn get_block_message(tool_name: &str) -> Option<String> {
    check_tool_allowed(tool_name).err().map(|b| b.message)
}

/// Guard that clears the tool whitelist when dropped.
pub struct WhitelistGuard;

impl Drop for WhitelistGuard {
    fn drop(&mut self) {
        clear_thread_tool_whitelist();
    }
}

/// Error message for a blocked tool call.
#[derive(Debug, Clone)]
pub struct BlockMessage {
    /// The tool that was blocked.
    pub tool: String,

    /// The list of allowed tools.
    pub allowed: Vec<String>,

    /// Human-readable block message.
    pub message: String,
}

impl BlockMessage {
    /// Convert to a JSON value for hook return.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "action": "block",
            "tool": self.tool,
            "message": self.message,
            "allowed_tools": self.allowed.iter().collect::<Vec<_>>()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_whitelist_allows_all() {
        /// given: no whitelist set
        /// when: check_tool_allowed() is called
        /// then: any tool is allowed
        let result = check_tool_allowed("any_tool");
        assert!(result.is_ok());
    }

    #[test]
    fn test_whitelist_allows_included() {
        /// given: whitelist with specific tools
        /// when: check_tool_allowed() is called for an included tool
        /// then: tool is allowed
        let _guard = set_thread_tool_whitelist(HashSet::from(["read".into(), "write".into()]));

        assert!(check_tool_allowed("read").is_ok());
        assert!(check_tool_allowed("write").is_ok());
    }

    #[test]
    fn test_whitelist_blocks_excluded() {
        /// given: whitelist with specific tools
        /// when: check_tool_allowed() is called for an excluded tool
        /// then: returns BlockMessage
        let _guard = set_thread_tool_whitelist(HashSet::from(["read".into()]));

        let result = check_tool_allowed("execute");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("execute"));
        assert!(err.message.contains("read"));
    }

    #[test]
    fn test_whitelist_guard_clears_on_drop() {
        /// given: a whitelist is set
        /// when: guard is dropped
        /// then: whitelist is cleared
        let tools = HashSet::from(["read".into()]);
        let guard = set_thread_tool_whitelist(tools.clone());
        drop(guard);

        assert!(check_tool_allowed("any").is_ok());
    }

    #[test]
    fn test_clear_thread_tool_whitelist() {
        /// given: a whitelist is set
        /// when: clear_thread_tool_whitelist() is called
        /// then: whitelist is cleared
        let _guard = set_thread_tool_whitelist(HashSet::from(["read".into()]));
        clear_thread_tool_whitelist();

        assert!(check_tool_allowed("any").is_ok());
    }

    #[test]
    fn test_block_message_to_json() {
        /// given: a BlockMessage
        /// when: to_json() is called
        /// then: returns correct JSON with action=block
        let msg = BlockMessage {
            tool: "write".into(),
            allowed: vec!["read".into()],
            message: "write not allowed".into(),
        };

        let json = msg.to_json();
        assert_eq!(json["action"], "block");
        assert_eq!(json["tool"], "write");
        assert_eq!(json["message"], "write not allowed");
    }
}
