/// Post-turn hooks for the agent conversation loop.
///
/// `PostTurnHook` trait fires after each turn. Multiple implementations share
/// the trait:
/// - `NudgePostTurnHook` — memory/skill review turns
use crate::nudge::{build_nudge_prompt, should_trigger_nudge, NudgeConfig};
use oben_models::Message;

/// A hook that fires after each turn.
pub trait PostTurnHook {
    fn id(&self) -> &str;
    fn should_trigger(&mut self, msg_count: usize, turns_since: usize) -> bool;
    fn prepare_turn(&mut self) -> Message;
    fn handle_result(&mut self, response: &str);
    fn handle_error(&mut self);
}

/// Nudge post-turn hook — mirrors Hermes `_spawn_background_review`.
pub struct NudgePostTurnHook {
    config: NudgeConfig,
}

impl NudgePostTurnHook {
    pub fn new(config: NudgeConfig) -> Self {
        Self { config }
    }
}

impl PostTurnHook for NudgePostTurnHook {
    fn id(&self) -> &str {
        "nudge"
    }

    fn should_trigger(&mut self, msg_count: usize, turns_since: usize) -> bool {
        if !self.config.enabled() || turns_since == 0 {
            return false;
        }
        should_trigger_nudge(
            turns_since,
            self.config.memory_nudge_interval,
            msg_count > 0,
            false,
        )
    }

    fn prepare_turn(&mut self) -> Message {
        Message::user(&build_nudge_prompt(
            self.config.memory_enabled(),
            self.config.skill_enabled(),
        ))
    }

    fn handle_result(&mut self, response: &str) {
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

    fn handle_error(&mut self) {
        tracing::info!("Nudge review failed (non-fatal)");
    }
}
