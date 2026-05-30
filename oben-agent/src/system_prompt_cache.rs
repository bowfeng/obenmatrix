
/// System prompt cache backed by a session store.
pub struct SystemPromptCache {
    /// The cached system prompt.
    cached_prompt: Option<String>,
    /// Session ID this cache belongs to.
    session_id: Option<String>,
}

impl SystemPromptCache {
    pub fn new() -> Self {
        Self {
            cached_prompt: None,
            session_id: None,
        }
    }

    /// Initialize the cache for a session.
    pub fn on_session_start(&mut self, session_id: &str) {
        self.session_id = Some(session_id.to_string());
    }

    /// Update the cached prompt after building a new one.
    ///
    /// This should be called after a system prompt is built for a new session
    /// or after context compression changes the prompt.
    pub fn set_prompt(&mut self, prompt: &str) {
        self.cached_prompt = Some(prompt.to_string());
    }

    /// Get the cached prompt, if available.
    pub fn get_prompt(&self) -> Option<&str> {
        self.cached_prompt.as_deref()
    }

    /// Check if we have a cached prompt.
    pub fn has_prompt(&self) -> bool {
        self.cached_prompt.is_some()
    }

    /// Clear the cache (e.g., after session reset).
    pub fn clear(&mut self) {
        self.cached_prompt = None;
        self.session_id = None;
    }

    /// Get the session ID this cache belongs to.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

impl Default for SystemPromptCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cache_is_empty() {
        let cache = SystemPromptCache::new();
        assert!(!cache.has_prompt());
        assert!(cache.get_prompt().is_none());
    }

    #[test]
    fn test_set_and_get_prompt() {
        let mut cache = SystemPromptCache::new();
        cache.set_prompt("You are a helpful assistant.");
        assert!(cache.has_prompt());
        assert_eq!(cache.get_prompt(), Some("You are a helpful assistant."));
    }

    #[test]
    fn test_update_prompt() {
        let mut cache = SystemPromptCache::new();
        cache.set_prompt("Old prompt");
        cache.set_prompt("New prompt");
        assert_eq!(cache.get_prompt(), Some("New prompt"));
    }

    #[test]
    fn test_clear_removes_prompt() {
        let mut cache = SystemPromptCache::new();
        cache.set_prompt("Some prompt");
        cache.clear();
        assert!(!cache.has_prompt());
        assert!(cache.get_prompt().is_none());
    }

    #[test]
    fn test_empty_prompt_allowed() {
        let mut cache = SystemPromptCache::new();
        cache.set_prompt("");
        assert!(cache.has_prompt());
        assert_eq!(cache.get_prompt(), Some(""));
    }
}
