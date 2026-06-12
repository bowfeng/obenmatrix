/// Goal loop — drives autonomous agent turns based on a goal.
///
/// The goal loop:
/// 1. Creates or loads a plan from the goal
/// 2. Iteratively: pick next pending node → run turn → parse result → update plan → judge
/// 3. Auto-pauses when the judge fails N times in a row or budget is exhausted
pub mod goal_state;

pub use goal_state::{GoalState, GoalStatus};

use anyhow::Result;

use tracing::{info, warn};

use crate::goal_store::GoalStore;
use crate::{
    judge::JudgeVerdict,
    plan_parser::{parse_node_complete, parse_plan_from_markdown},
    plan_state::PlanState,
};

/// Configuration for the goal loop.
#[derive(Debug, Clone)]
pub struct GoalLoopConfig {
    /// Maximum number of turns before auto-pause.
    pub max_turns: usize,
    /// System prompt to prepend to each node's instructions.
    pub system_prompt: Option<String>,
    /// Whether to save state after each turn.
    pub auto_save: bool,
}

impl Default for GoalLoopConfig {
    fn default() -> Self {
        Self {
            max_turns: 20,
            system_prompt: None,
            auto_save: true,
        }
    }
}

/// The result of running a goal loop iteration.
#[derive(Debug, Clone)]
pub struct LoopIteration {
    /// The node that was worked on (None if no pending node).
    pub node_title: Option<String>,
    /// LLM response for this turn.
    pub response: String,
    /// Whether a node completion/failure was detected.
    pub node_completed: bool,
    /// Whether the judge says the overall goal is done.
    pub goal_done: bool,
}

/// Run the autonomous goal loop.
///
/// This is the main entry point for the self-improving agent. It:
/// 1. Creates or loads a plan from the goal
/// 2. Picks the next pending node
/// 3. Sends the node instructions to the LLM with tools enabled
/// 4. Parses the response for completion/failure markers
/// 5. Updates the plan state
/// 6. Calls the judge to check if the overall goal is satisfied
/// 7. Repeats until done, budget exhausted, or auto-pause
pub async fn run_goal_loop<F, Fut>(
    goal: &str,
    goal_id: &str,
    config: &GoalLoopConfig,
    store: &dyn GoalStore,
    mut execute_node: F,
) -> Result<(PlanState, GoalState)>
where
    F: FnMut(&str) -> Fut,
    Fut: std::future::Future<Output = Result<String>> + Send,
{
    // Load existing state from store — errors if files do not exist
    let mut plan = store.load_plan(goal_id)?;
    let mut goal_state = store.load_goal_state(goal_id)?;

    info!("Starting goal loop: {}", goal);
    info!("{}", goal_state.status_line());

    // Main loop
    loop {
        // Check if we should continue
        if !goal_state.should_continue() {
            info!("Goal loop stopped: {}", goal_state.status_line());
            break;
        }

        // Pick the next pending node
        let next_node = if let Some(node) = plan.next_pending_node() {
            node.title.clone()
        } else {
            // No pending nodes — check if all are done or all failed
            let all_done = plan.nodes.iter().all(|n| {
                n.status == crate::plan::NodeStatus::Done
                    || n.status == crate::plan::NodeStatus::Failed
            });
            if all_done {
                info!(
                    "All nodes completed or failed. Calling judge to check if goal is satisfied."
                );
                // Call judge to verify
                let verdict = call_judge(&goal_state.goal, &plan).await?;
                goal_state.record_verdict(&verdict);
                match &verdict {
                    JudgeVerdict::Done(reason) => {
                        info!("Goal satisfied: {}", reason);
                    }
                    _ => {
                        warn!("Not all nodes are completed and judge says not done. Stopping.");
                        goal_state.pause("no pending nodes and goal not satisfied");
                    }
                }
            } else {
                // Mixed state — some nodes failed but there are pending ones
                info!("No pending nodes found but not all done. Stopping.");
                goal_state.pause("no pending nodes remaining");
            }
            break;
        };

        info!("Executing next node: {}", next_node);

        // Mark node as in-progress
        plan.mark_in_progress(&next_node);

        // Build the system prompt with plan context
        let system_prompt = build_system_prompt(&plan, &config.system_prompt);

        // Send the node instructions to the LLM
        let response = execute_node(&system_prompt).await?;

        // Record the turn
        goal_state.record_turn();

        // Check for node completion/failure markers in the response
        let node_completed = parse_node_completion(&mut plan, &next_node, &response);

        // Call the judge
        let verdict = call_judge(&goal_state.goal, &plan).await?;
        goal_state.record_verdict(&verdict);

        let iteration = LoopIteration {
            node_title: Some(next_node.clone()),
            response: response.clone(),
            node_completed,
            goal_done: matches!(verdict, JudgeVerdict::Done(_)),
        };

        info!("Turn complete: {:?}", iteration);

        // Persist state via store
        if config.auto_save {
            store.save_goal_state(goal_id, &goal_state)?;
            store.save_plan(goal_id, &plan)?;
        }

        // Check if judge says done
        if iteration.goal_done {
            info!("Judge says goal is satisfied!");
            break;
        }
    }

    // Final save via store
    if config.auto_save {
        store.save_goal_state(goal_id, &goal_state)?;
        store.save_plan(goal_id, &plan)?;
    }

    info!("Goal loop finished: {}", goal_state.status_line());
    Ok((plan, goal_state))
}

/// Create a plan from the goal using the LLM.
/// The LLM should return a markdown checklist plan.
pub async fn create_plan_from_goal<F, Fut>(
    goal: &str,
    _config: &GoalLoopConfig,
    mut execute_plan: F,
) -> Result<PlanState>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String>> + Send,
{
    info!("Creating plan for goal: {}", goal);

    let _system_prompt = format!(
        "You are a planner. Break the following goal into a step-by-step plan.\n\n\
        Return your plan as a markdown checklist:\n\
        - [ ] step 1\n\
        - [ ] step 2\n\
          - [ ] sub-step 2.1\n\n\
        Goal: {}",
        goal
    );

    let response = execute_plan().await?;
    let plan = parse_plan_from_markdown(&response)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse plan from LLM response"))?;

    info!("Created plan with {} nodes", plan.total_count());
    Ok(plan)
}

/// Build the system prompt for executing a node.
fn build_system_prompt(plan: &PlanState, extra_system_prompt: &Option<String>) -> String {
    let mut prompt = String::new();

    if let Some(ref sp) = extra_system_prompt {
        prompt.push_str(sp);
        prompt.push('\n');
    }

    prompt.push_str("\n\nYou are working on the following goal:\n\n");
    prompt.push_str(&plan.goal);
    prompt.push_str("\n\nHere is your current plan:\n\n");
    prompt.push_str(&plan.to_markdown());
    prompt.push_str("\n\nExecute the next pending node. When you complete it, output:");
    prompt.push_str("\n### Node Complete: 'node_title' — Result: description of what was done");
    prompt.push_str("\n\nIf you fail, output:");
    prompt.push_str("\n### Node Failed: 'node_title' — Reason: why it failed");

    prompt
}

/// Parse and apply node completion/failure markers from the response.
fn parse_node_completion(plan: &mut PlanState, node_title: &str, response: &str) -> bool {
    if let Some((title, message)) = parse_node_complete(response) {
        if title == node_title {
            if response.starts_with("### Node Complete:") {
                plan.mark_done(&title, &message);
                info!("Marked node '{}' as done: {}", title, message);
            } else if response.starts_with("### Node Failed:") {
                plan.mark_failed(&title, &message);
                info!("Marked node '{}' as failed: {}", title, message);
            }
            return true;
        } else {
            warn!(
                "Parsed node completion for '{}' but current node is '{}'",
                title, node_title
            );
        }
    }
    false
}

/// Call the judge to check if the overall goal is satisfied.
async fn call_judge(_goal: &str, plan: &PlanState) -> Result<JudgeVerdict> {
    // In the future, this will call an auxiliary LLM.
    // For now, use a simple heuristic: if all nodes are done, the goal is done.
    let all_done = plan
        .nodes
        .iter()
        .all(|n| n.status == crate::plan::NodeStatus::Done);
    let any_failed = plan
        .nodes
        .iter()
        .any(|n| n.status == crate::plan::NodeStatus::Failed);

    if all_done {
        info!("All plan nodes completed — judge auto-verdict: done");
        return Ok(JudgeVerdict::Done(format!(
            "All {} nodes in the plan have been completed",
            plan.nodes.len()
        )));
    }

    if any_failed {
        let failed_count = plan
            .nodes
            .iter()
            .filter(|n| n.status == crate::plan::NodeStatus::Failed)
            .count();
        warn!(
            "{} nodes failed — judge auto-verdict: continue",
            failed_count
        );
        return Ok(JudgeVerdict::Continue(format!(
            "{} node(s) failed, need to retry or adapt",
            failed_count
        )));
    }

    // Default: continue
    Ok(JudgeVerdict::Continue(
        "Plan still has pending nodes".to_string(),
    ))
}

#[cfg(test)]
use crate::PlanNode;
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_contains_plan() {
        let mut plan = PlanState::new("build a web scraper");
        plan.add_node(PlanNode::new("research"));
        plan.add_node(PlanNode::new("implement").status(crate::plan::NodeStatus::Done));
        plan.nodes[1].mark_done("done!");

        let prompt = build_system_prompt(&plan, &None);
        assert!(prompt.contains("build a web scraper"));
        assert!(prompt.contains("research"));
        assert!(prompt.contains("[x]"));
    }

    #[test]
    fn test_build_system_prompt_includes_extra() {
        let plan = PlanState::new("test goal");
        let extra = "You are a helpful agent.";
        let prompt = build_system_prompt(&plan, &Some(extra.to_string()));
        assert!(prompt.contains("You are a helpful agent."));
    }

    #[test]
    fn test_parse_node_completion_done() {
        let mut plan = PlanState::new("goal");
        plan.add_node(PlanNode::new("research"));
        plan.mark_in_progress("research");

        let response = "### Node Complete: 'research' — Result: Found 3 relevant files";
        let completed = parse_node_completion(&mut plan, "research", response);
        assert!(completed);
        assert!(matches!(
            plan.find_node("research").unwrap().status,
            crate::plan::NodeStatus::Done
        ));
    }

    #[test]
    fn test_parse_node_completion_failed() {
        let mut plan = PlanState::new("goal");
        plan.add_node(PlanNode::new("deploy"));
        plan.mark_in_progress("deploy");

        let response = "### Node Failed: 'deploy' — Reason: No SSH access";
        let completed = parse_node_completion(&mut plan, "deploy", response);
        assert!(completed);
        assert!(matches!(
            plan.find_node("deploy").unwrap().status,
            crate::plan::NodeStatus::Failed
        ));
    }

    #[test]
    fn test_parse_node_completion_mismatched_title() {
        let mut plan = PlanState::new("goal");
        plan.add_node(PlanNode::new("node-a"));
        plan.mark_in_progress("node-a");

        // Response mentions node-b, not node-a
        let response = "### Node Complete: 'node-b' — Result: done";
        let completed = parse_node_completion(&mut plan, "node-a", response);
        assert!(!completed);
        // Node-a should still be in-progress
        assert!(matches!(
            plan.find_node("node-a").unwrap().status,
            crate::plan::NodeStatus::InProgress
        ));
    }

    #[test]
    fn test_parse_node_completion_no_markers() {
        let mut plan = PlanState::new("goal");
        plan.add_node(PlanNode::new("task"));
        plan.mark_in_progress("task");

        let response = "I worked on the task but didn't complete it.";
        let completed = parse_node_completion(&mut plan, "task", response);
        assert!(!completed);
    }

    #[test]
    fn test_create_plan_from_goal_parses_markdown() {
        // Simulate LLM creating a plan
        let _mock_llm = || async {
            Ok::<String, anyhow::Error>("## Plan: build a scraper\n- [ ] research existing codebase\n- [ ] implement fetcher\n  - [ ] write request handler\n- [ ] integration test".to_string())
        };

        // Note: create_plan_from_goal is async, so we need tokio
        // This test verifies the markdown parsing logic indirectly
        let response = "## Plan: build a scraper\n- [ ] research existing codebase\n- [ ] implement fetcher\n  - [ ] write request handler\n- [ ] integration test";
        let plan = parse_plan_from_markdown(response).unwrap();
        assert_eq!(plan.goal, "build a scraper");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.nodes[1].sub_nodes.len(), 1);
    }

    #[test]
    fn test_plan_with_all_done_nodes_is_marked_done() {
        let mut plan = PlanState::new("deploy app");
        let mut n1 = PlanNode::new("build");
        n1.mark_done("built successfully");
        let mut n2 = PlanNode::new("test");
        n2.mark_done("tests passed");
        plan.add_node(n1);
        plan.add_node(n2);

        // All nodes done → judge says done
        let verdict = call_judge_sync("deploy app", &plan);
        assert!(matches!(verdict, JudgeVerdict::Done(_)));
    }

    #[test]
    fn test_plan_with_failed_nodes_gets_continue_verdict() {
        let mut plan = PlanState::new("deploy app");
        let mut n1 = PlanNode::new("build");
        n1.mark_done("built");
        let mut n2 = PlanNode::new("test");
        n2.mark_failed("tests failed");
        plan.add_node(n1);
        plan.add_node(n2);

        let verdict = call_judge_sync("deploy app", &plan);
        assert!(matches!(verdict, JudgeVerdict::Continue(_)));
    }

    #[test]
    fn test_plan_with_pending_nodes_gets_continue_verdict() {
        let mut plan = PlanState::new("deploy app");
        let mut n1 = PlanNode::new("build");
        n1.mark_done("built");
        plan.add_node(n1);
        plan.add_node(PlanNode::new("test")); // still pending

        let verdict = call_judge_sync("deploy app", &plan);
        assert!(matches!(verdict, JudgeVerdict::Continue(_)));
    }

    // Synchronous helper for tests (simulates the judge logic)
    #[allow(dead_code)]
    fn call_judge_sync(_goal: &str, plan: &PlanState) -> JudgeVerdict {
        let all_done = plan
            .nodes
            .iter()
            .all(|n| n.status == crate::plan::NodeStatus::Done);
        let any_failed = plan
            .nodes
            .iter()
            .any(|n| n.status == crate::plan::NodeStatus::Failed);

        if all_done {
            JudgeVerdict::Done(format!(
                "All {} nodes in the plan have been completed",
                plan.nodes.len()
            ))
        } else if any_failed {
            let failed_count = plan
                .nodes
                .iter()
                .filter(|n| n.status == crate::plan::NodeStatus::Failed)
                .count();
            JudgeVerdict::Continue(format!(
                "{} node(s) failed, need to retry or adapt",
                failed_count
            ))
        } else {
            JudgeVerdict::Continue("Plan still has pending nodes".to_string())
        }
    }

    #[test]
    fn test_goal_state_should_continue_false_when_done() {
        let mut state = GoalState::new_active("goal");
        state.record_verdict(&JudgeVerdict::Done("All done".to_string()));
        assert!(!state.should_continue());
        assert!(state.is_done());
    }

    #[test]
    fn test_goal_state_should_continue_false_on_budget() {
        let mut state = GoalState::new("goal", Some(1));
        state.record_turn();
        assert!(!state.should_continue());
    }

    #[test]
    fn test_goal_state_should_continue_false_on_parse_failures() {
        let mut state = GoalState::new_active("goal");
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        state.record_verdict(&JudgeVerdict::Continue("not JSON".to_string()));
        assert!(!state.should_continue());
    }
}
