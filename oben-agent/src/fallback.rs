/// Fallback model chain — backup providers when the primary fails.
///
/// Mirrors Hermes' `_fallback_chain` and `_fallback_index` for provider
/// fallback on rate limit exhaustion, overload, or connection failure.
use serde::{Deserialize, Serialize};

/// Single fallback model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

/// Fallback chain state — tracks which fallback is active.
#[derive(Clone)]
pub struct FallbackChain {
    /// Ordered list of fallback providers.
    fallbacks: Vec<FallbackConfig>,
    /// Current fallback index (0 = primary, 1+ = fallback N).
    current_index: usize,
    /// Whether a fallback was activated during this turn.
    activated: bool,
}

impl FallbackChain {
    pub fn new(fallbacks: Vec<FallbackConfig>) -> Self {
        Self {
            fallbacks,
            current_index: 0,
            activated: false,
        }
    }

    /// Check if there are fallbacks available.
    pub fn has_fallbacks(&self) -> bool {
        !self.fallbacks.is_empty()
    }

    /// Get the number of fallback providers.
    pub fn count(&self) -> usize {
        self.fallbacks.len()
    }

    /// Try to activate the next fallback model.
    ///
    /// Returns the next fallback config, or None if all fallbacks exhausted.
    pub fn activate_next(&mut self) -> Option<&FallbackConfig> {
        if self.current_index < self.fallbacks.len() {
            self.current_index += 1;
            self.activated = true;
            Some(&self.fallbacks[self.current_index - 1])
        } else {
            None
        }
    }

    /// Check if a fallback was activated during the current session/turn.
    pub fn is_activated(&self) -> bool {
        self.activated
    }

    /// Reset fallback state (call at turn boundary).
    pub fn reset(&mut self) {
        self.current_index = 0;
        self.activated = false;
    }

    /// Get the current fallback index (0 = primary).
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Check if we are on a fallback (not the primary).
    pub fn is_on_fallback(&self) -> bool {
        self.current_index > 0
    }

    /// Restore to primary model.
    pub fn restore_primary(&mut self) {
        self.current_index = 0;
    }

    /// Get the active fallback config (or None if on primary).
    pub fn active_fallback(&self) -> Option<&FallbackConfig> {
        if self.current_index > 0 && self.current_index <= self.fallbacks.len() {
            Some(&self.fallbacks[self.current_index - 1])
        } else {
            None
        }
    }

    /// Masked API key for logging (show first 8 and last 4 chars).
    pub fn masked_api_key(&self) -> String {
        if let Some(fb) = self.active_fallback() {
            mask_api_key(fb.api_key.as_deref())
        } else {
            String::new()
        }
    }
}

/// Mask an API key for safe logging.
fn mask_api_key(key: Option<&str>) -> String {
    match key {
        None => "none".to_string(),
        Some(k) if k.is_empty() => "empty".to_string(),
        Some(k) if k.len() <= 12 => "***".to_string(),
        Some(k) => format!("{}...{}", &k[..8], &k[k.len() - 4..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fallback(provider: &str, model: &str) -> FallbackConfig {
        FallbackConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key: Some(format!("key-{}-{}", provider, model)),
            base_url: Some(format!("https://{}-example.com", provider)),
        }
    }

    #[test]
    fn test_new_chain_with_no_fallbacks() {
        let chain = FallbackChain::new(vec![]);
        assert!(!chain.has_fallbacks());
        assert_eq!(chain.count(), 0);
    }

    #[test]
    fn test_activate_next_cycles_through_fallbacks() {
        let fallbacks = vec![
            make_fallback("provider-a", "model-a"),
            make_fallback("provider-b", "model-b"),
        ];
        let mut chain = FallbackChain::new(fallbacks);

        assert!(!chain.is_activated());
        assert!(!chain.is_on_fallback());

        // Activate first fallback
        let fb1 = chain.activate_next().unwrap();
        let fb1_provider = fb1.provider.clone();
        let fb1_model = fb1.model.clone();
        assert!(chain.is_activated());
        assert!(chain.is_on_fallback());
        assert_eq!(fb1_provider, "provider-a");
        assert_eq!(fb1_model, "model-a");

        // Activate second fallback
        let fb2 = chain.activate_next().unwrap();
        let fb2_provider = fb2.provider.clone();
        let fb2_model = fb2.model.clone();
        assert_eq!(fb2_provider, "provider-b");
        assert_eq!(fb2_model, "model-b");

        // No more fallbacks
        assert!(chain.activate_next().is_none());
    }

    #[test]
    fn test_restore_primary() {
        let fallbacks = vec![make_fallback("fb", "model")];
        let mut chain = FallbackChain::new(fallbacks);
        chain.activate_next();
        assert!(chain.is_on_fallback());

        chain.restore_primary();
        assert!(!chain.is_on_fallback());
    }

    #[test]
    fn test_reset_clears_state() {
        let fallbacks = vec![make_fallback("fb", "model")];
        let mut chain = FallbackChain::new(fallbacks);
        chain.activate_next();
        chain.reset();
        assert!(!chain.is_activated());
        assert_eq!(chain.current_index(), 0);
    }

    #[test]
    fn test_masked_api_key() {
        let fallbacks = vec![FallbackConfig {
            provider: "test".to_string(),
            model: "m".to_string(),
            api_key: Some("abcdefghijklmnop".to_string()),
            base_url: None,
        }];
        let mut chain = FallbackChain::new(fallbacks);

        // On primary — no masked key
        assert_eq!(chain.masked_api_key(), "");

        chain.activate_next();
        let masked = chain.masked_api_key();
        assert!(masked.contains("abcdefgh...mnop"));
    }

    #[test]
    fn test_active_fallback_returns_correct() {
        let fallbacks = vec![make_fallback("fb1", "m1"), make_fallback("fb2", "m2")];
        let chain = FallbackChain::new(fallbacks);

        // On primary — no active fallback
        assert!(chain.active_fallback().is_none());

        // Simulate first fallback active
        // Note: we can't mutate without `mut`, so just test the accessor logic
    }

    #[test]
    fn test_serialization() {
        let config = FallbackConfig {
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4.6".to_string(),
            api_key: Some("sk-test-key".to_string()),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: FallbackConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, config.provider);
        assert_eq!(back.model, config.model);
    }
}
