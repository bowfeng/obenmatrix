/// Goal loop — drives autonomous agent turns based on a goal.
///
/// The goal loop:
/// 1. Creates or loads a plan from the goal
/// 2. After each turn, parses completion/failure messages
/// 3. Updates the plan state
/// 4. Calls the judge to decide: done or continue?
/// 5. Auto-pauses when the judge fails N times in a row or budget is exhausted

pub mod goal_state;

pub use goal_state::{GoalState, GoalStatus};
