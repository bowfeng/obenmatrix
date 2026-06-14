/// Hook pipeline — configurable post-turn hooks and observers.
///
/// The hook pipeline provides two kinds of extensibility:
///
/// **PostTurnHook** — fires after each turn and can execute a sub-turn (LLM call).
/// Used by e.g. `NudgePostTurnHook` (memory/skill review).
///
/// **PostTurnObserver** — fires after each turn but does NOT execute an LLM call.
/// Used by e.g. auto-save, telemetry, logging, custom actions.
///
/// The `HookFactory` builds both lists from `HooksConfig`, replacing the hardcoded
/// single-nudge-hook pattern in `ConversationLoop::run_loop`.
///
/// ## Functional Lifecycle Hooks
///
/// The `hooks::kind` module defines 10 domain-specific hook traits with no-op defaults.
/// The `hooks::adapters` module provides adapter types that bridge closure-style
/// callbacks to the new trait system.
///
/// Re-exports from the functional hook system:

use crate::nudge::NudgeConfig;
use crate::post_turn_hook::NudgePostTurnHook;
use crate::PostTurnHook;
use tracing;

use oben_config::HooksConfig;

// Re-export functional hook traits and adapters
pub mod kind;
pub mod adapters;

// ---------------------------------------------------------------------------
// PostTurnObserver trait
// ---------------------------------------------------------------------------

/// A hook that fires after each turn without executing an LLM call.
///
/// Use this for lightweight actions: event logging, auto-save triggers,
/// custom metrics, telemetry emission, etc.
pub trait PostTurnObserver {
    /// Unique identifier for this observer (used in logs).
    fn id(&self) -> &str;

    /// Called after a successful turn.
    fn on_turn_complete(
        &mut self,
        response: &str,
        msg_count: usize,
        turns_since_nudge: usize,
    );

    /// Called after a failed turn.
    fn on_turn_error(&mut self, error: &anyhow::Error, msg_count: usize, turns_since_nudge: usize);
}

/// Nudge post-turn observer — fires after each turn and can trigger a review.
pub struct NudgeObserver {
    config: NudgeConfig,
}

impl NudgeObserver {
    pub fn new(config: NudgeConfig) -> Self {
        Self { config }
    }
}

impl PostTurnObserver for NudgeObserver {
    fn id(&self) -> &str {
        "nudge"
    }

    fn on_turn_complete(
        &mut self,
        response: &str,
        _msg_count: usize,
        _turns_since_nudge: usize,
    ) {
        let tl = response.to_lowercase();
        let noop = || {
            tl.contains("nothing to")
                || tl.contains("nothing worth")
                || tl.contains("no changes needed")
        };
        if noop() {
            tracing::info!("Nudge: nothing worth saving.");
        } else {
            tracing::info!("Nudge: checked memory — may have updated.");
        }
    }

    fn on_turn_error(&mut self, error: &anyhow::Error, _msg_count: usize, _turns_since_nudge: usize) {
        tracing::info!("Nudge review failed (non-fatal): {}", error);
    }
}

// ---------------------------------------------------------------------------
// HookFactory — builds hooks and observers from config
// ---------------------------------------------------------------------------

/// Builds the hook and observer list from configuration.
///
/// For each hook type in `config.enabled`, looks up its deserialization config
/// in `config.configs` and builds the corresponding hook. Unknown types are skipped.
pub struct HookFactory;

impl HookFactory {
    /// Build all post-turn hooks from config.
    pub fn build_hooks(config: &HooksConfig) -> Vec<Box<dyn PostTurnHook>> {
        if config.enabled.is_empty() {
            return vec![];
        }

        let mut hooks = Vec::with_capacity(config.enabled.len());

        for hook_type in &config.enabled {
            let Some(val) = config.configs.get(hook_type) else {
                tracing::debug!("Hook type '{}' in enabled but no config found — skipping", hook_type);
                continue;
            };
            hooks.push(Self::build_hook(hook_type.as_str(), val));
        }

        hooks
    }

    /// Build all post-turn observers from config.
    pub fn build_observers(config: &HooksConfig) -> Vec<Box<dyn PostTurnObserver>> {
        if config.enabled.is_empty() {
            return vec![];
        }

        let mut observers = Vec::with_capacity(config.enabled.len());

        for hook_type in &config.enabled {
            let Some(val) = config.configs.get(hook_type) else {
                continue;
            };
            observers.push(Self::build_observer(hook_type.as_str(), val));
        }

        observers
    }

    /// Build a single hook instance from its type name and config value.
    fn build_hook(name: &str, config_value: &serde_yaml::Value) -> Box<dyn PostTurnHook> {
        match name {
            "nudge" => {
                let nc = Self::deserialize_hook_config::<NudgeConfig>(config_value)
                    .unwrap_or_default();
                Box::new(NudgePostTurnHook::new(nc))
            }
            other => {
                tracing::warn!("Unknown hook type '{}', cannot build", other);
                Box::new(NoopHook::default())
            }
        }
    }

    /// Build a single observer instance from its type name and config value.
    fn build_observer(name: &str, config_value: &serde_yaml::Value) -> Box<dyn PostTurnObserver> {
        match name {
            "nudge" => {
                let nc = Self::deserialize_hook_config::<NudgeConfig>(config_value)
                    .unwrap_or_default();
                Box::new(NudgeObserver::new(nc))
            }
            other => {
                tracing::warn!("Unknown observer type '{}', cannot build", other);
                Box::new(NoopObserver::default())
            }
        }
    }

    fn deserialize_hook_config<T: serde::de::DeserializeOwned>(value: &serde_yaml::Value) -> Option<T> {
        serde_yaml::from_value::<T>(value.clone()).ok()
    }
}

/// A no-op hook that does nothing — fallback for unknown types.
struct NoopHook;

impl Default for NoopHook {
    fn default() -> Self {
        Self
    }
}

impl crate::PostTurnHook for NoopHook {
    fn id(&self) -> &str {
        "noop"
    }
    fn should_trigger(&mut self, _msg_count: usize, _turns_since: usize) -> bool {
        false
    }
    fn prepare_turn(&mut self) -> oben_models::Message {
        oben_models::Message::system(String::from("noop"))
    }
    fn handle_result(&mut self, _response: &str) {}
    fn handle_error(&mut self) {}
}

/// A no-op observer that does nothing — fallback for unknown types.
struct NoopObserver;

impl Default for NoopObserver {
    fn default() -> Self {
        Self
    }
}

impl PostTurnObserver for NoopObserver {
    fn id(&self) -> &str {
        "noop"
    }
    fn on_turn_complete(&mut self, _response: &str, _msg_count: usize, _turns_since: usize) {}
    fn on_turn_error(&mut self, _error: &anyhow::Error, _msg_count: usize, _turns_since: usize) {}
}

// Re-export all hook traits
pub use kind::*;

// Re-export all adapter types
pub use adapters::*;
