/// Goal persistence storage trait.
///
/// Abstracts goal, plan, and state persistence. The default
/// implementation (`JsonGoalStore`) uses JSON files on disk.
use super::goal_loop::GoalState;
use super::plan_state::PlanState;
use anyhow::Result;
use std::path::PathBuf;

/// A single goal record returned from listing operations.
#[derive(Debug, Clone)]
pub struct GoalRecord {
    pub id: String,
    pub text: String,
    pub status: String,
    pub turns_used: usize,
    pub max_turns: usize,
    pub owner: Option<String>,
}

/// Trait for persisting goal data, plan state, and goal state.
pub trait GoalStore: Send + Sync {
    /// Create a new goal record.
    fn create_goal(
        &self,
        goal_id: &str,
        goal_text: &str,
        max_turns: usize,
        owner: Option<&str>,
    ) -> Result<()>;

    /// Retrieve a single goal record.
    fn get_goal(
        &self,
        goal_id: &str,
    ) -> Result<Option<(String, String, usize, usize, Option<String>)>>;

    /// List goals, optionally filtered by status.
    fn list_goals(&self, status: Option<&str>) -> Result<Vec<GoalRecord>>;

    /// Update goal status, turns, verdict, and reason.
    fn update_goal(
        &self,
        goal_id: &str,
        status: &str,
        turns_used: usize,
        verdict: Option<&str>,
        reason: Option<&str>,
    ) -> Result<()>;

    /// Pause a goal with a reason.
    fn pause_goal(&self, goal_id: &str, reason: &str) -> Result<()>;

    /// Resume a paused goal, optionally resetting turn budget.
    fn resume_goal(&self, goal_id: &str, reset_budget: bool) -> Result<()>;

    /// Delete a goal entirely.
    fn delete_goal(&self, goal_id: &str) -> Result<()>;

    /// Save plan state to storage.
    fn save_plan(&self, goal_id: &str, plan: &PlanState) -> Result<()>;

    /// Load plan state from storage.
    ///
    /// Errors if the file does not exist — the caller must ensure the plan
    /// was created before attempting to load it.
    fn load_plan(&self, goal_id: &str) -> Result<PlanState>;

    /// Save goal state to storage.
    fn save_goal_state(&self, goal_id: &str, state: &GoalState) -> Result<()>;

    /// Load goal state from storage.
    ///
    /// Errors if the file does not exist — the caller must ensure the
    /// goal state was created before attempting to load it.
    fn load_goal_state(&self, goal_id: &str) -> Result<GoalState>;
}

// ── JSON File Implementation ──────────────────────────────────────────

/// Local JSON-file based goal store.
///
/// Stores files in a configurable directory (default: `~/.oben-goals/`):
/// - `{goal_id}.json` — goal record metadata
/// - `plan-{goal_id}.json` — plan state
/// - `state-{goal_id}.json` — goal state

#[derive(Clone)]
pub struct JsonGoalStore {
    dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GoalFile {
    goal_id: String,
    goal_text: String,
    status: String,
    max_turns: usize,
    turns_used: usize,
    owner: Option<String>,
    verdict: Option<String>,
    reason: Option<String>,
}

impl JsonGoalStore {
    /// Create a new `JsonGoalStore` backed by the given directory.
    /// Ensures the directory exists.
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Create a new store from the `OBEN_GOALS_STATE` env var or default to `~/.oben-goals`.
    pub fn default_store() -> Result<Self> {
        Self::new_with_agent(None)
    }

    /// Create a new store with agent isolation.
    /// Each agent gets its own goal storage directory: `~/.oben-goals/<agent_name>/`
    pub fn new_with_agent(agent_name: Option<&str>) -> Result<Self> {
        let base = std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".oben-goals"))
            .unwrap_or_else(|| PathBuf::from(".oben-goals"));
        
        let agent_dir = agent_name
            .and_then(|n| if n.is_empty() { None } else { Some(n) })
            .unwrap_or("default");
        
        let dir = base.join(agent_dir);
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn goal_path(&self, goal_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", goal_id))
    }

    fn plan_path(&self, goal_id: &str) -> PathBuf {
        self.dir.join(format!("plan-{}.json", goal_id))
    }

    fn state_path(&self, goal_id: &str) -> PathBuf {
        self.dir.join(format!("state-{}.json", goal_id))
    }
}

impl GoalStore for JsonGoalStore {
    fn create_goal(
        &self,
        goal_id: &str,
        goal_text: &str,
        max_turns: usize,
        owner: Option<&str>,
    ) -> Result<()> {
        let goal_file = GoalFile {
            goal_id: goal_id.to_string(),
            goal_text: goal_text.to_string(),
            status: "active".to_string(),
            max_turns,
            turns_used: 0,
            owner: owner.map(String::from),
            verdict: None,
            reason: None,
        };

        let json = serde_json::to_string_pretty(&goal_file)?;
        std::fs::write(self.goal_path(goal_id), &json)?;
        Ok(())
    }

    fn get_goal(
        &self,
        goal_id: &str,
    ) -> Result<Option<(String, String, usize, usize, Option<String>)>> {
        let path = self.goal_path(goal_id);
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(path)?;
        let gf: GoalFile = serde_json::from_str(&json)?;
        Ok(Some((
            gf.goal_text,
            gf.status,
            gf.turns_used,
            gf.max_turns,
            gf.reason,
        )))
    }

    fn list_goals(&self, status: Option<&str>) -> Result<Vec<GoalRecord>> {
        let mut records = Vec::new();
        for entry in self.dir.read_dir()? {
            let entry = entry?;
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            // Only process {goal_id}.json files
            if !file_name_str.ends_with(".json") {
                continue;
            }
            let base = &file_name_str[..file_name_str.len() - 5];
            // Skip plan and state files
            if base.starts_with("plan-") || base.starts_with("state-") {
                continue;
            }
            let goal_path = entry.path();
            let json = std::fs::read_to_string(&goal_path)?;
            let gf: GoalFile = serde_json::from_str(&json)?;
            if let Some(status) = status {
                if gf.status != status {
                    continue;
                }
            }
            records.push(GoalRecord {
                id: gf.goal_id,
                text: gf.goal_text,
                status: gf.status,
                turns_used: gf.turns_used,
                max_turns: gf.max_turns,
                owner: gf.owner,
            });
        }
        Ok(records)
    }

    fn update_goal(
        &self,
        goal_id: &str,
        status: &str,
        turns_used: usize,
        verdict: Option<&str>,
        reason: Option<&str>,
    ) -> Result<()> {
        let path = self.goal_path(goal_id);
        let json = std::fs::read_to_string(&path)?;
        let mut gf: GoalFile = serde_json::from_str(&json)?;
        gf.status = status.to_string();
        gf.turns_used = turns_used;
        gf.verdict = verdict.map(String::from);
        gf.reason = reason.map(String::from);
        let json_out = serde_json::to_string_pretty(&gf)?;
        std::fs::write(&path, json_out)?;
        Ok(())
    }

    fn pause_goal(&self, goal_id: &str, _reason: &str) -> Result<()> {
        self.update_goal(goal_id, "paused", 0, None, None)
    }

    fn resume_goal(&self, goal_id: &str, _reset_budget: bool) -> Result<()> {
        // Resume: set active, optionally reset turns in the trait caller
        self.update_goal(goal_id, "active", 0, None, None)
    }

    fn delete_goal(&self, goal_id: &str) -> Result<()> {
        let goal_path = self.goal_path(goal_id);
        if goal_path.exists() {
            std::fs::remove_file(&goal_path)?;
        }
        let plan_path = self.plan_path(goal_id);
        if plan_path.exists() {
            std::fs::remove_file(&plan_path)?;
        }
        let state_path = self.state_path(goal_id);
        if state_path.exists() {
            std::fs::remove_file(&state_path)?;
        }
        Ok(())
    }

    fn save_plan(&self, goal_id: &str, plan: &PlanState) -> Result<()> {
        let path = self.plan_path(goal_id);
        let json = serde_json::to_string_pretty(plan)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn load_plan(&self, goal_id: &str) -> Result<PlanState> {
        let path = self.plan_path(goal_id);
        if !path.exists() {
            anyhow::bail!(
                "Plan file not found for goal '{}': {}",
                goal_id,
                path.display()
            );
        }
        let json = std::fs::read_to_string(path)?;
        let plan: PlanState = serde_json::from_str(&json)?;
        Ok(plan)
    }

    fn save_goal_state(&self, goal_id: &str, state: &GoalState) -> Result<()> {
        let path = self.state_path(goal_id);
        let json = serde_json::to_string_pretty(state)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn load_goal_state(&self, goal_id: &str) -> Result<GoalState> {
        let path = self.state_path(goal_id);
        if !path.exists() {
            anyhow::bail!(
                "Goal state file not found for goal '{}': {}",
                goal_id,
                path.display()
            );
        }
        let json = std::fs::read_to_string(path)?;
        let state: GoalState = serde_json::from_str(&json)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (JsonGoalStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonGoalStore::new(dir.path()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_create_and_get_goal() {
        let (store, _dir) = temp_store();
        store
            .create_goal("g1", "test goal", 10, Some("session-1"))
            .unwrap();

        let result = store.get_goal("g1").unwrap().unwrap();
        assert_eq!(result.0, "test goal");
        assert_eq!(result.1, "active");
        assert_eq!(result.2, 0);
        assert_eq!(result.3, 10);
        assert_eq!(result.4, None);
    }

    #[test]
    fn test_get_nonexistent_goal() {
        let (store, _dir) = temp_store();
        assert!(store.get_goal("g0").unwrap().is_none());
    }

    #[test]
    fn test_update_goal() {
        let (store, _dir) = temp_store();
        store.create_goal("g1", "test goal", 10, None).unwrap();
        store
            .update_goal("g1", "paused", 3, Some("judg"), Some("budget"))
            .unwrap();

        let result = store.get_goal("g1").unwrap().unwrap();
        assert_eq!(result.1, "paused");
        assert_eq!(result.2, 3);
    }

    #[test]
    fn test_pause_and_resume() {
        let (store, _dir) = temp_store();
        store.create_goal("g1", "test goal", 10, None).unwrap();
        store.pause_goal("g1", "user").unwrap();
        assert_eq!(store.get_goal("g1").unwrap().unwrap().1, "paused");

        store.resume_goal("g1", false).unwrap();
        assert_eq!(store.get_goal("g1").unwrap().unwrap().1, "active");
    }

    #[test]
    fn test_save_and_load_plan() {
        let (store, _dir) = temp_store();
        let mut plan = PlanState::new("goal");
        plan.add_node(super::super::PlanNode::new("step 1"));

        store.save_plan("g1", &plan).unwrap();
        let loaded = store.load_plan("g1").unwrap();
        assert_eq!(loaded.goal, "goal");
        assert_eq!(loaded.nodes.len(), 1);
    }

    #[test]
    fn test_save_and_load_goal_state() {
        let (store, _dir) = temp_store();
        let state = GoalState::new_active("my goal");

        store.save_goal_state("g1", &state).unwrap();
        let loaded = store.load_goal_state("g1").unwrap();
        assert_eq!(loaded.goal, "my goal");
        assert!(loaded.is_active());
    }

    #[test]
    fn test_delete_goal() {
        let (store, _dir) = temp_store();
        store.create_goal("g1", "test", 5, None).unwrap();
        store.save_plan("g1", &PlanState::new("test")).unwrap();
        store
            .save_goal_state("g1", &GoalState::new_active("test"))
            .unwrap();

        store.delete_goal("g1").unwrap();
        assert!(store.get_goal("g1").unwrap().is_none());
        assert!(store.load_plan("g1").is_err());
        assert!(store.load_goal_state("g1").is_err());
    }

    #[test]
    fn test_list_goals() {
        let (store, _dir) = temp_store();
        store.create_goal("g1", "goal 1", 10, None).unwrap();
        store.create_goal("g2", "goal 2", 5, None).unwrap();
        store.update_goal("g2", "done", 2, None, None).unwrap();

        let all = store.list_goals(None).unwrap();
        assert_eq!(all.len(), 2);

        let active = store.list_goals(Some("active")).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].text, "goal 1");

        let done = store.list_goals(Some("done")).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].text, "goal 2");
    }
}
