//! Gateway fallbacks - fallback strategies for gateway operations
//!
//! FallbackManager implements fallback strategies to handle failures gracefully

use std::collections::HashMap;

/// Fallback strategy for gateway operations
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackStrategy {
    BackupPlatform(String),
    Cached,
    Default,
    Queue,
    None,
}

/// Fallback result for gateway operations
#[derive(Clone, Debug)]
pub struct FallbackResult<T> {
    pub fallback_used: bool,
    pub value: Option<T>,
    pub strategy: Option<FallbackStrategy>,
    pub original_error: Option<String>,
}

impl<T> FallbackResult<T> {
    pub fn success(value: T) -> Self {
        Self {
            fallback_used: false,
            value: Some(value),
            strategy: None,
            original_error: None,
        }
    }

    pub fn fallback(value: T, strategy: FallbackStrategy, original_error: Option<String>) -> Self {
        Self {
            fallback_used: true,
            value: Some(value),
            strategy: Some(strategy),
            original_error,
        }
    }

    pub fn failure(original_error: String) -> Self {
        Self {
            fallback_used: false,
            value: None,
            strategy: None,
            original_error: Some(original_error),
        }
    }

    pub fn is_success(&self) -> bool {
        self.value.is_some()
    }

    pub fn unwrap_or(self, default: T) -> T {
        self.value.unwrap_or(default)
    }
}

/// Fallback manager - manages fallback strategies for platforms
pub struct FallbackManager {
    default_strategy: FallbackStrategy,
    platform_strategies: HashMap<String, FallbackStrategy>,
    cached_responses: HashMap<String, String>,
}

impl FallbackManager {
    pub fn new() -> Self {
        Self {
            default_strategy: FallbackStrategy::None,
            platform_strategies: HashMap::new(),
            cached_responses: HashMap::new(),
        }
    }

    pub fn with_default_strategy(strategy: FallbackStrategy) -> Self {
        Self {
            default_strategy: strategy,
            platform_strategies: HashMap::new(),
            cached_responses: HashMap::new(),
        }
    }

    pub fn set_platform_strategy(&mut self, platform: &str, strategy: FallbackStrategy) {
        self.platform_strategies.insert(platform.to_string(), strategy);
    }

    pub fn get_platform_strategy(&self, platform: &str) -> FallbackStrategy {
        self.platform_strategies
            .get(platform)
            .cloned()
            .unwrap_or(self.default_strategy.clone())
    }

    pub fn cache_response(&mut self, platform: &str, response: &str) {
        self.cached_responses
            .insert(platform.to_string(), response.to_string());
    }

    pub fn get_cached_response(&self, platform: &str) -> Option<&String> {
        self.cached_responses.get(platform)
    }

    pub fn apply_fallback(&self, platform: &str, original_error: Option<String>) -> FallbackResult<String> {
        let strategy = self.get_platform_strategy(platform);

        match strategy.clone() {
            FallbackStrategy::BackupPlatform(backup_platform) => {
                FallbackResult::fallback(
                    format!("Using backup platform: {}", backup_platform),
                    strategy,
                    original_error,
                )
            }
            FallbackStrategy::Cached => {
                if let Some(cached) = self.get_cached_response(platform) {
                    FallbackResult::fallback(cached.clone(), strategy, original_error)
                } else {
                    FallbackResult::failure(original_error.unwrap_or_else(|| "No cached response".to_string()))
                }
            }
            FallbackStrategy::Default => {
                FallbackResult::fallback(
                    "Service temporarily unavailable".to_string(),
                    strategy,
                    original_error,
                )
            }
            FallbackStrategy::Queue => {
                FallbackResult::fallback(
                    "Message queued for later retry".to_string(),
                    strategy,
                    original_error,
                )
            }
            FallbackStrategy::None => {
                FallbackResult::failure(original_error.unwrap_or_else(|| "No fallback available".to_string()))
            }
        }
    }

    pub fn clear_cache(&mut self) {
        self.cached_responses.clear();
    }

    pub fn cache_size(&self) -> usize {
        self.cached_responses.len()
    }
}

impl Default for FallbackManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_result_success() {
        let result: FallbackResult<String> = FallbackResult::success("ok".to_string());
        assert!(result.is_success());
        assert!(!result.fallback_used);
        assert_eq!(result.value.unwrap(), "ok");
    }

    #[test]
    fn test_fallback_result_fallback() {
        let result: FallbackResult<String> = FallbackResult::fallback(
            "fallback".to_string(),
            FallbackStrategy::Cached,
            Some("original error".to_string()),
        );
        assert!(result.is_success());
        assert!(result.fallback_used);
        assert_eq!(result.strategy.unwrap(), FallbackStrategy::Cached);
    }

    #[test]
    fn test_fallback_result_failure() {
        let result: FallbackResult<String> = FallbackResult::failure("failed".to_string());
        assert!(!result.is_success());
        assert!(result.original_error.is_some());
    }

    #[test]
    fn test_fallback_manager_strategies() {
        let mut manager = FallbackManager::new();
        
        manager.set_platform_strategy("telegram", FallbackStrategy::BackupPlatform("discord".to_string()));
        manager.set_platform_strategy("discord", FallbackStrategy::Cached);
        
        assert_eq!(
            manager.get_platform_strategy("telegram"),
            FallbackStrategy::BackupPlatform("discord".to_string())
        );
        assert_eq!(
            manager.get_platform_strategy("unknown"),
            FallbackStrategy::None
        );
    }

    #[test]
    fn test_fallback_manager_cache() {
        let mut manager = FallbackManager::new();
        
        manager.cache_response("telegram", "cached response");
        assert_eq!(manager.get_cached_response("telegram"), Some(&"cached response".to_string()));
        
        manager.clear_cache();
        assert!(manager.get_cached_response("telegram").is_none());
    }

    #[test]
    fn test_fallback_manager_apply_fallback() {
        let manager = FallbackManager::new();
        
        let result = manager.apply_fallback("test", Some("error".to_string()));
        assert!(!result.is_success());
    }
}
