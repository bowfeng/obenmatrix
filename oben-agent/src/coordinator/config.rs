/// Shared conversation configuration — extracted from AppConfig.
///
/// This is the common set of settings used by both CLI and TUI conversation
/// loops. It covers retry behavior, iteration limits, and concurrent tool
/// dispatch.
use crate::concurrent_dispatch::ConcurrentDispatchConfig;
use crate::retry::RetryConfig;

/// Conversational loop configuration — pure values, no runtime state.
/// `Agent.fallback_chains` provides mutable fallback chain state at run time.
#[derive(Debug, Clone, Default)]
pub struct ConversationConfig {
    pub retry_config: RetryConfig,
    pub max_iterations: usize,
    pub fallback_configs: Vec<oben_config::FallbackConfig>,
    pub dispatch_config: Option<ConcurrentDispatchConfig>,
    pub max_spawn_depth: u32,
}

impl ConversationConfig {
    /// Build from the top-level AppConfig.
    pub fn from_app_config(app_config: &oben_config::AppConfig) -> Self {
        Self {
            retry_config: RetryConfig {
                max_retries: app_config.retry.max_retries,
                base_delay_ms: app_config.retry.base_delay_ms,
                max_delay_ms: app_config.retry.max_delay_ms,
                jitter_factor: app_config.retry.jitter_factor,
                retryable_codes: app_config.retry.retryable_codes.clone(),
            },
            max_iterations: app_config.max_iterations.unwrap_or(50),
            fallback_configs: app_config.fallback_models.clone(),
            dispatch_config: Some(ConcurrentDispatchConfig {
                max_concurrency: app_config.concurrency.max_concurrency,
                serial_only_tools: app_config.concurrency.serial_only_tools.clone(),
                destructive_tools: app_config.concurrency.destructive_tools.clone(),
            }),
            max_spawn_depth: app_config.max_spawn_depth.unwrap_or(3) as u32,
        }
    }
}

/// Builder-friendly wrapper — allows CLI/TUI to construct with overrides.
pub struct ConversationConfigBuilder {
    config: ConversationConfig,
}

impl ConversationConfigBuilder {
    pub fn from_app_config(app_config: &oben_config::AppConfig) -> Self {
        Self {
            config: ConversationConfig::from_app_config(app_config),
        }
    }

    pub fn with_retry(self, retry: RetryConfig) -> Self {
        Self {
            config: ConversationConfig {
                retry_config: retry,
                ..self.config
            },
        }
    }

    pub fn with_max_iterations(self, max: usize) -> Self {
        Self {
            config: ConversationConfig {
                max_iterations: max,
                ..self.config
            },
        }
    }

    pub fn with_max_spawn_depth(self, max: u32) -> Self {
        Self {
            config: ConversationConfig {
                max_spawn_depth: max,
                ..self.config
            },
        }
    }

    pub fn build(self) -> ConversationConfig {
        self.config
    }
}
