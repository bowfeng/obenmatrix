/// Turn nudge — background memory/skill review after every N turns.
///
/// Mirrors Hermes' `_should_review_memory` trigger + `_spawn_background_review`
/// pipeline: the nudge checks a turn-count threshold (default 10 by Hermes),
/// then triggers a review using the agent's existing memory tools.
///
/// This module provides pure types and helpers. The actual review execution
/// is done via `Agent::trigger_nudge()` which shares the same prompt templates
/// and trigger detection from the turn cycle.

// ── Config ──────────────────────────────────────────────────────────────────

/// Configuration for nudge triggers.
#[derive(Debug, Clone)]
pub struct NudgeConfig {
    /// How many user turns to wait before triggering a memory review.
    /// Set to `0` to disable.
    pub memory_nudge_interval: usize,
    /// How many tool-call iterations within a single turn to wait
    /// before suggesting skill creation. Set to `0` to disable.
    pub skill_nudge_interval: usize,
}

impl Default for NudgeConfig {
    fn default() -> Self {
        Self {
            memory_nudge_interval: 10, // default: every 10 turns (matches Hermes)
            skill_nudge_interval: 10,
        }
    }
}

impl NudgeConfig {
    /// Create with custom intervals.
    pub fn new(memory_interval: usize, skill_interval: usize) -> Self {
        Self {
            memory_nudge_interval: memory_interval,
            skill_nudge_interval: skill_interval,
        }
    }

    /// Check whether nudge review is enabled.
    pub fn enabled(&self) -> bool {
        self.memory_nudge_interval > 0 || self.skill_nudge_interval > 0
    }

    /// Check whether memory nudge is enabled.
    pub fn memory_enabled(&self) -> bool {
        self.memory_nudge_interval > 0
    }

    /// Check whether skill nudge is enabled.
    pub fn skill_enabled(&self) -> bool {
        self.skill_nudge_interval > 0
    }
}

// ── Result ──────────────────────────────────────────────────────────────────

/// A nudge review result.
#[derive(Debug, Clone)]
pub struct NudgeResult {
    pub memory_updated: bool,
    pub skill_suggested: bool,
    pub summary: String,
}

// ── Trigger check ───────────────────────────────────────────────────────────

/// Check if the memory/skill nudge should have triggered.
///
/// Mirrors `conversation_loop.py:384-393`.
pub fn should_trigger_nudge(
    turns_since_nudge: usize,
    interval: usize,
    has_memory_tools: bool,
    is_resumed_session: bool,
) -> bool {
    if interval == 0 {
        return false;
    }
    // During session resume, don't trigger a nudge on the very first turn —
    // that turn is reserved for loading context / restoring state.
    if is_resumed_session && turns_since_nudge == 0 {
        return false;
    }
    turns_since_nudge >= interval && has_memory_tools
}

// ── Prompt building ─────────────────────────────────────────────────────────

/// Build the MEMORY_REVIEW_PROMPT from Hermes.
///
/// Mirrors `hermes-agent/agent/background_review.py:_MEMORY_REVIEW_PROMPT`.
pub fn build_nudge_prompt(memory_enabled: bool, skill_enabled: bool) -> String {
    match (memory_enabled, skill_enabled) {
        (true, true) => {
            format!(
                "Review the conversation above and consider the following:\n\n\
                 MEMORY REVIEW:\n\
                 1. Has the user revealed things about themselves — their persona, desires, \
                 preferences, or personal details worth remembering?\n\
                 2. Has the user expressed expectations about how you should behave, their work \
                 style, or ways they want you to operate?\n\n\
                 SKILL REVIEW:\n\
                 3. Were there repetitive tasks or workflows that could be turned into reusable \
                 skills or shortcuts?\n\
                 4. Are there patterns in how the user works that could be encoded as skills?\n\n\
                 If anything is worth saving, use the memory tool. \
                 If any skills should be created, note that for the user. \
                 If nothing is worth saving or creating, just say 'Nothing to save.' and stop."
            )
        }
        (true, false) => {
            format!(
                "Review the conversation above and consider saving to memory if appropriate.\n\n\
                 Focus on:\n\
                 1. Has the user revealed things about themselves — their persona, desires, \
                 preferences, or personal details worth remembering?\n\
                 2. Has the user expressed expectations about how you should behave, their work \
                 style, or ways they want you to operate?\n\n\
                 If something stands out, save it using the memory tool. \
                 If nothing is worth saving, just say 'Nothing to save.' and stop."
            )
        }
        (false, true) => {
            format!(
                "Review the conversation above. Were there repetitive tasks or workflows that \
                 could be turned into reusable skills or shortcuts? Are there patterns in how \
                 the user works that could be encoded as skills? If so, note these for the user. \
                 If not, just say 'Nothing to note.' and stop."
            )
        }
        (false, false) => "Review the conversation and note anything worth saving.".into(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_nudge_config() {
        let cfg = NudgeConfig::default();
        assert_eq!(cfg.memory_nudge_interval, 10);
        assert_eq!(cfg.skill_nudge_interval, 10);
    }

    #[test]
    fn test_enabled_config() {
        let cfg = NudgeConfig::new(10, 10);
        assert!(cfg.enabled());
        assert!(cfg.memory_enabled());
        assert!(cfg.skill_enabled());
    }

    #[test]
    fn test_disabled_config() {
        let cfg = NudgeConfig::new(0, 0);
        assert!(!cfg.enabled());
    }

    #[test]
    fn test_should_trigger_nudge() {
        // Below threshold — no trigger.
        assert!(!should_trigger_nudge(5, 10, true, false));

        // At threshold — trigger.
        assert!(should_trigger_nudge(10, 10, true, false));

        // Well over — trigger.
        assert!(should_trigger_nudge(20, 10, true, false));

        // No memory tools — no trigger.
        assert!(!should_trigger_nudge(15, 10, false, false));

        // Disabled interval — no trigger.
        assert!(!should_trigger_nudge(999, 0, true, false));

        // Resumed session, first turn (0) — no trigger.
        assert!(!should_trigger_nudge(0, 10, true, true));

        // Resumed session, second turn (1) — no trigger (still below threshold).
        assert!(!should_trigger_nudge(1, 10, true, true));

        // Resumed session, turned 10 — trigger.
        assert!(should_trigger_nudge(10, 10, true, true));
    }

    #[test]
    fn test_build_nudge_prompt_memory_only() {
        let prompt = build_nudge_prompt(true, false);
        // memory_only prompt does NOT have "MEMORY REVIEW:" (that's only in 'both')
        // Instead it has the shorter "Review the conversation above..." text
        assert!(prompt.contains("Review the conversation above and consider saving"));
        assert!(prompt.contains("memory tool"));
        assert!(!prompt.contains("SKILL REVIEW"));
    }

    #[test]
    fn test_build_nudge_prompt_skill_only() {
        let prompt = build_nudge_prompt(false, true);
        assert!(prompt.contains("skills"));
        assert!(!prompt.contains("MEMORY"));
    }

    #[test]
    fn test_build_nudge_prompt_both() {
        let prompt = build_nudge_prompt(true, true);
        assert!(prompt.contains("MEMORY REVIEW:"));
        assert!(prompt.contains("SKILL REVIEW:"));
    }
}
