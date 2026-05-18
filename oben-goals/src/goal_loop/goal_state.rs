/// Goal state — the living state of an active goal loop.
///
/// Mirrors hermes-agent's `GoalState` dataclass: persists goal text,
/// status, turn budget, and judge verdict across turns.

use crate::judge::JudgeVerdict;

/// Status of an active goal.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GoalStatus {
    /// Goal is active and the agent is working on it.
    #[default]
    Active,
    /// Goal has been completed.
    Done,
    /// Goal is paused (budget exhausted, judge failures, etc.).
    Paused,
    /// Goal was cleared by the user.
    Cleared,
}

/// Serializable goal state stored per session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoalState {
    /// The user's goal statement.
    pub goal: String,
    /// Current status of the goal.
    #[serde(default)]
    pub status: GoalStatus,
    /// Turns consumed so far.
    #[serde(default)]
    pub turns_used: usize,
    /// Maximum turns allowed.
    #[serde(default = "default_max_turns")]
    pub max_turns: usize,
    /// When the goal was created.
    #[serde(default)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last turn timestamp.
    #[serde(default)]
    pub last_turn_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Last judge verdict.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verdict: Option<String>,
    /// Last judge reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reason: Option<String>,
    /// Why the goal was paused (if paused).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    /// Consecutive judge parse failures (auto-pause trigger).
    #[serde(default)]
    pub consecutive_parse_failures: usize,
}

const fn default_max_turns() -> usize {
    20
}

impl GoalState {
    /// Create a new active goal state.
    pub fn new(goal: impl Into<String>, max_turns: Option<usize>) -> Self {
        Self {
            goal: goal.into(),
            status: GoalStatus::Active,
            turns_used: 0,
            max_turns: max_turns.unwrap_or(default_max_turns()),
            created_at: chrono::Utc::now(),
            last_turn_at: None,
            last_verdict: None,
            last_reason: None,
            paused_reason: None,
            consecutive_parse_failures: 0,
        }
    }

    /// Create a new active goal state with default max turns (20).
    pub fn new_active(goal: impl Into<String>) -> Self {
        Self::new(goal, None)
    }

    /// Record the result of a turn.
    pub fn record_turn(&mut self) {
        self.turns_used += 1;
        self.last_turn_at = Some(chrono::Utc::now());
    }

    /// Record a judge verdict.
    pub fn record_verdict(&mut self, verdict: &JudgeVerdict) {
        let parse_ok = !matches!(verdict, JudgeVerdict::Continue(ref r) if r.contains("empty") || r.contains("not JSON"));
        if parse_ok {
            self.consecutive_parse_failures = 0;
        } else {
            self.consecutive_parse_failures += 1;
        }
        match verdict {
            JudgeVerdict::Done(reason) => {
                self.status = GoalStatus::Done;
                self.last_verdict = Some("done".to_string());
                self.last_reason = Some(reason.clone());
            }
            JudgeVerdict::Continue(reason) => {
                self.last_verdict = Some("continue".to_string());
                self.last_reason = Some(reason.clone());
            }
            JudgeVerdict::Skipped(reason) => {
                self.last_verdict = Some("skipped".to_string());
                self.last_reason = Some(reason.clone());
            }
        }
    }

    /// Check if the goal should continue (active and within budget).
    pub fn should_continue(&self) -> bool {
        matches!(self.status, GoalStatus::Active)
            && self.turns_used < self.max_turns
            && self.consecutive_parse_failures < 3
    }

    /// Check if the goal is done.
    pub fn is_done(&self) -> bool {
        matches!(self.status, GoalStatus::Done)
    }

    /// Check if the goal is active.
    pub fn is_active(&self) -> bool {
        matches!(self.status, GoalStatus::Active)
    }

    /// Pause the goal with a reason.
    pub fn pause(&mut self, reason: impl Into<String>) {
        self.status = GoalStatus::Paused;
        self.paused_reason = Some(reason.into());
    }

    /// Clear the goal.
    pub fn clear(&mut self) {
        self.status = GoalStatus::Cleared;
        self.paused_reason = None;
    }

    /// Get a human-readable status line.
    pub fn status_line(&self) -> String {
        let status_str = match self.status {
            GoalStatus::Active => "active",
            GoalStatus::Done => "done",
            GoalStatus::Paused => "paused",
            GoalStatus::Cleared => "cleared",
        };
        format!(
            "Goal ({}, {}/{}, parse_failures={})",
            status_str,
            self.turns_used,
            self.max_turns,
            self.consecutive_parse_failures,
        )
    }

    /// Save to a JSON file.
    pub fn save_to_file(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }

    /// Load from a JSON file.
    pub fn load_from_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&json)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_state_new_active() {
        let state = GoalState::new_active("build a scraper");
        assert!(state.is_active());
        assert!(!state.is_done());
        assert_eq!(state.turns_used, 0);
        assert_eq!(state.max_turns, 20);
        assert_eq!(state.goal, "build a scraper");
    }

    #[test]
    fn test_goal_state_custom_max_turns() {
        let state = GoalState::new("goal", Some(5));
        assert_eq!(state.max_turns, 5);
    }

    #[test]
    fn test_goal_state_record_turn() {
        let mut state = GoalState::new_active("goal");
        state.record_turn();
        assert_eq!(state.turns_used, 1);
        assert!(state.last_turn_at.is_some());
    }

    #[test]
    fn test_goal_state_should_continue() {
        let mut state = GoalState::new_active("goal");
        assert!(state.should_continue());
        state.record_turn();
        assert!(state.should_continue());
    }

    #[test]
    fn test_goal_state_budget_exhausted() {
        let mut state = GoalState::new("goal", Some(2));
        state.record_turn();
        state.record_turn();
        assert!(!state.should_continue());
        assert_eq!(state.turns_used, 2);
    }

    #[test]
    fn test_goal_state_parse_judge_done() {
        let mut state = GoalState::new_active("goal");
        state.record_verdict(&JudgeVerdict::Done("All done".to_string()));
        assert!(state.is_done());
        assert_eq!(state.last_verdict, Some("done".to_string()));
        assert_eq!(state.last_reason, Some("All done".to_string()));
    }

    #[test]
    fn test_goal_state_parse_judge_continue() {
        let mut state = GoalState::new_active("goal");
        state.record_verdict(&JudgeVerdict::Continue("Keep going".to_string()));
        assert!(state.is_active());
        assert_eq!(state.last_verdict, Some("continue".to_string()));
    }

    #[test]
    fn test_goal_state_parse_failures_count() {
        let mut state = GoalState::new_active("goal");
        // Two parse failures
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        assert_eq!(state.consecutive_parse_failures, 2);
        // One success resets counter
        state.record_verdict(&JudgeVerdict::Continue("valid".to_string()));
        assert_eq!(state.consecutive_parse_failures, 0);
    }

    #[test]
    fn test_goal_state_pause() {
        let mut state = GoalState::new_active("goal");
        state.pause("budget exhausted");
        assert!(!state.is_active());
        assert_eq!(state.paused_reason, Some("budget exhausted".to_string()));
    }

    #[test]
    fn test_goal_state_clear() {
        let mut state = GoalState::new_active("goal");
        state.clear();
        assert!(!state.is_active());
    }

    #[test]
    fn test_goal_state_status_line() {
        let state = GoalState::new_active("goal");
        let line = state.status_line();
        assert!(line.contains("active"));
        assert!(line.contains("0/20"));
    }

    #[test]
    fn test_goal_state_roundtrip_json() {
        let state = GoalState::new_active("save and restore");
        let json = serde_json::to_string(&state).unwrap();
        let back: GoalState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.goal, "save and restore");
        assert!(matches!(back.status, GoalStatus::Active));
    }

    #[test]
    fn test_goal_state_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goal.json");

        let mut state = GoalState::new_active("file persistence");
        state.record_turn();
        state.record_verdict(&JudgeVerdict::Continue("in progress".to_string()));
        state.save_to_file(&path).unwrap();

        let loaded = GoalState::load_from_file(&path).unwrap();
        assert_eq!(loaded.goal, "file persistence");
        assert_eq!(loaded.turns_used, 1);
    }

    #[test]
    fn test_goal_state_auto_pause_on_3_parse_failures() {
        let mut state = GoalState::new_active("goal");
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        assert!(!state.should_continue());
        assert_eq!(state.consecutive_parse_failures, 3);
    }
}
