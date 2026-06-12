/// Goal manager — the service layer for goal-driven autonomous execution.
///
/// `GoalManager` manages goal lifecycle via its own entity `goal_id`.
/// Goals are independent of sessions — a goal can be owned by one session
/// and referenced by multiple sessions via `session_goal_refs`.
///
/// The **internal seam** hides the goal loop, judge evaluation, and
/// continuation logic behind a small interface.
///
/// Persistence is abstracted behind the `GoalStore` trait.
use super::goal_loop::GoalState;
use super::goal_loop::GoalStatus;
use super::goal_store::GoalStore;
use super::judge::{self, JudgeVerdict};
use super::plan::NodeStatus;
use super::plan_state::PlanState;
use anyhow::Result;
use std::sync::Arc;

/// Result of evaluating a goal after a turn completes.
#[derive(Debug, Clone)]
pub enum GoalResult {
    Done { reason: String, message: String },
    Continue { reason: String, prompt: String },
    Stopped { reason: String, message: String },
}

/// Manages a single goal by its ID.
pub struct GoalManager {
    inner: Arc<std::sync::Mutex<GoalManagerInner>>,
    store: Box<dyn GoalStore>,
}

struct GoalManagerInner {
    goal_id: String,
    goal_state: Option<GoalState>,
    plan_state: Option<PlanState>,
}

impl GoalManager {
    /// Create a new GoalManager bound to a goal ID.
    ///
    /// The store handles all persistence (plans, states, goal records).
    pub fn new(goal_id: &str, store: Box<dyn GoalStore>) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(GoalManagerInner {
                goal_id: goal_id.to_string(),
                goal_state: None,
                plan_state: None,
            })),
            store,
        }
    }

    pub fn goal_id(&self) -> String {
        self.inner.lock().unwrap().goal_id.clone()
    }

    pub fn has_goal(&self) -> bool {
        self.inner.lock().unwrap().goal_state.is_some()
    }

    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .goal_state
            .as_ref()
            .is_some_and(|s| s.is_active())
    }

    /// Start a new goal.
    ///
    /// Creates the goal record, initializes plan and state in memory,
    /// and persists them to the store.
    pub fn set_goal(
        &self,
        goal: &str,
        max_turns: Option<usize>,
        owner: Option<&str>,
    ) -> Result<GoalState> {
        let max_turns = max_turns.unwrap_or(20);
        let goal_id = self.goal_id();

        // Create goal record in store
        self.store.create_goal(&goal_id, goal, max_turns, owner)?;

        // Load from store to sync
        self.load_from_store()?;

        // Initialize plan in memory
        {
            let mut inner = self.inner.lock().unwrap();
            inner.plan_state = Some(PlanState::new(goal));
        }

        let state = GoalState {
            goal: goal.to_string(),
            status: GoalStatus::Active,
            turns_used: 0,
            max_turns,
            created_at: chrono::Utc::now(),
            last_turn_at: None,
            last_verdict: None,
            last_reason: None,
            paused_reason: None,
            consecutive_parse_failures: 0,
        };

        // Persist initial state file
        self.store.save_goal_state(&goal_id, &state)?;

        Ok(state)
    }

    /// Pause the goal with a reason.
    pub fn pause_goal(&self, reason: &str) -> Result<()> {
        self.store.pause_goal(&self.goal_id(), reason)?;

        let mut inner = self.inner.lock().unwrap();
        if let Some(mut state) = inner.goal_state.take() {
            state.pause(reason);
            inner.goal_state = Some(state);
        }
        Ok(())
    }

    /// Resume a paused goal.
    pub fn resume_goal(&self, reset_budget: bool) -> Result<()> {
        self.store.resume_goal(&self.goal_id(), reset_budget)?;

        let mut inner = self.inner.lock().unwrap();
        if let Some(mut state) = inner.goal_state.take() {
            state.resume(reset_budget);
            inner.goal_state = Some(state);
        }
        Ok(())
    }

    /// Clear (delete) the goal entirely.
    pub fn clear_goal(&self) -> Result<()> {
        self.store.delete_goal(&self.goal_id())?;
        let mut inner = self.inner.lock().unwrap();
        inner.goal_state = None;
        inner.plan_state = None;
        Ok(())
    }

    /// Get the goal text.
    pub fn goal_text(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .goal_state
            .as_ref()
            .map(|s| s.goal.clone())
    }

    /// Get a status line.
    pub fn status_line(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .goal_state
            .as_ref()
            .map(|s| s.status_line())
    }

    /// Load goal state from store into the manager.
    pub fn load_from_store(&self) -> Result<()> {
        let goal_id = self.goal_id();

        // Load goal text and status from goal record
        let (goal_text, goal_status, max_turns, turns_used, paused_reason) =
            match self.store.get_goal(&goal_id)? {
                Some(v) => v,
                None => {
                    tracing::warn!("Goal {} not found in store", goal_id);
                    return Ok(());
                }
            };

        // Reconstruct GoalState from store fields
        let mut state = GoalState::new(&goal_text, Some(max_turns));
        state.turns_used = turns_used;
        match goal_status.as_str() {
            "done" => state.status = GoalStatus::Done,
            "paused" | "failed" => {
                state.status = GoalStatus::Paused;
                state.paused_reason =
                    paused_reason.or(Some(format!("Store status: {}", goal_status)));
            }
            _ => state.status = GoalStatus::Active,
        }

        {
            let mut inner = self.inner.lock().unwrap();
            inner.goal_state = Some(state);
        }

        // Load plan state
        {
            let mut inner = self.inner.lock().unwrap();
            inner.plan_state = Some(self.load_plan()?);
        }

        Ok(())
    }

    /// Evaluate the goal after a turn completes.
    ///
    /// Records the turn, calls the judge (LLM when a transport is provided,
    /// falling back to the heuristic stub otherwise), and updates the store.
    pub fn evaluate_after_turn(
        &self,
        _turn_response: &str,
        transport: Option<&dyn oben_models::TransportProvider>,
    ) -> Result<Option<GoalResult>> {
        let mut inner = self.inner.lock().unwrap();
        let mut state = match inner.goal_state.take() {
            Some(s) if s.is_active() => s,
            _ => return Ok(None),
        };

        let mut plan = match inner.plan_state.take() {
            Some(p) => p,
            None => {
                // No plan — fall back to goal text
                inner.goal_state = Some(state.clone());
                return Ok(Some(GoalResult::Continue {
                    reason: "no plan loaded".to_string(),
                    prompt: format!(
                        "\nContinue working toward the goal: {}\nTake the next concrete step.",
                        state.goal
                    ),
                }));
            }
        };

        state.record_turn();

        // Check budget
        if state.turns_used >= state.max_turns {
            state.pause("turn budget exhausted");
            self.persist_and_update(&mut state, &mut plan)?;
            return Ok(Some(GoalResult::Stopped {
                reason: format!(
                    "Goal '{}' budget exhausted ({} turns)",
                    state.goal, state.turns_used
                ),
                message: format!(
                    "Goal budget exhausted: {} turns of {} max.",
                    state.turns_used, state.max_turns
                ),
            }));
        }

        // Judge — use LLM judge when transport provided, fall back to heuristic.
        let verdict: JudgeVerdict = match (transport, tokio::runtime::Handle::try_current()) {
            (Some(transport), Ok(handle)) => {
                handle.block_on(judge::call_judge(transport, &state.goal, &plan))
            }
            _ => {
                if let Some(_t) = transport {
                    tracing::warn!(
                        "No tokio runtime available for judge LLM call, using heuristic"
                    );
                }
                self.judge_heuristic(&state.goal, &plan)
            }
        };
        state.record_verdict(&verdict);

        // Auto-pause on judge failures
        if state.consecutive_parse_failures >= 3 {
            state.pause("judge returning unusable output");
            self.persist_and_update(&mut state, &mut plan)?;
            return Ok(Some(GoalResult::Stopped {
                reason: "judge parse failures".to_string(),
                message: "Judge model returning unusable output. Goal paused.".to_string(),
            }));
        }

        self.persist_and_update(&mut state, &mut plan)?;

        match verdict {
            JudgeVerdict::Done(reason) => Ok(Some(GoalResult::Done {
                reason: reason.clone(),
                message: format!("Goal achieved: {}", reason),
            })),
            JudgeVerdict::Continue(reason) => {
                Ok(Some(GoalResult::Continue {
                    reason,
                    prompt: format!(
                        "[Continuing toward your standing goal]\nGoal: {}\nContinue working toward this goal.",
                        state.goal
                    ),
                }))
            }
            JudgeVerdict::Skipped(reason) => {
                Ok(Some(GoalResult::Continue {
                    reason,
                    prompt: format!("\nContinue working toward the goal: {}\nTake the next concrete step.", state.goal),
                }))
            }
        }
    }

    /// Load plan state from store.
    pub fn load_plan(&self) -> Result<PlanState> {
        let goal_id = self.goal_id();
        self.store.load_plan(&goal_id)
    }

    /// Save current plan state to store.
    pub fn save_plan(&self, plan: &PlanState) -> Result<()> {
        let goal_id = self.goal_id();
        self.store.save_plan(&goal_id, plan)
    }

    /// Heuristic judge — sync fallback when no LLM transport is available.
    fn judge_heuristic(&self, _goal: &str, plan: &PlanState) -> JudgeVerdict {
        let any_failed = plan.nodes.iter().any(|n| n.status == NodeStatus::Failed);
        let all_done = plan.nodes.iter().all(|n| n.status == NodeStatus::Done);

        if all_done {
            JudgeVerdict::Done(format!(
                "All {} nodes in the plan completed",
                plan.nodes.len()
            ))
        } else if any_failed {
            let count = plan
                .nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Failed)
                .count();
            JudgeVerdict::Continue(format!("{} node(s) failed, need to retry", count))
        } else {
            JudgeVerdict::Continue("Plan still has pending nodes".to_string())
        }
    }

    /// Helper to save state and update inner, also syncs plan to store.
    fn persist_and_update(&self, state: &mut GoalState, plan: &mut PlanState) -> Result<()> {
        // Update goal in store — persist turns/verdict
        let verdict_text = state.last_verdict.as_deref();
        let reason_text = state.last_reason.as_deref();
        let status_str = match state.status {
            GoalStatus::Active => "active",
            GoalStatus::Done => "done",
            GoalStatus::Paused => "paused",
            GoalStatus::Cleared => "cleared",
        };
        self.store.update_goal(
            &self.goal_id(),
            status_str,
            state.turns_used,
            verdict_text,
            reason_text,
        )?;

        // Save plan to store
        self.store.save_plan(&self.goal_id(), plan)?;

        // Save goal state to store
        self.store.save_goal_state(&self.goal_id(), state)?;

        // Update plan in memory
        {
            let mut inner = self.inner.lock().unwrap();
            inner.goal_state = Some(state.clone());
            inner.plan_state = Some(plan.clone());
        }

        Ok(())
    }
}
