/// LLM-based judge for goal completion evaluation.
///
/// Provides the judge prompt, LLM call, and response parsing for assessing
/// whether a goal has been completed by the agent.
pub mod verdict;
use super::plan::NodeStatus;
use super::plan_state::PlanState;
use oben_models::Message;
use verdict::parse_judge_response;

/// Re-export for consumers of the judge module.
pub use verdict::JudgeVerdict;

/// Prompt used to ask the judge LLM whether the goal is complete.
fn build_judge_prompt(goal: &str, plan: &PlanState) -> String {
    let nodes_status: String = plan
        .nodes
        .iter()
        .map(|n| {
            let status_char = match n.status {
                NodeStatus::Pending => 'P',
                NodeStatus::InProgress => 'I',
                NodeStatus::Done => 'D',
                NodeStatus::Failed => 'F',
            };
            format!("- [{}] {}", status_char, n.title)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are a goal completion judge. Evaluate whether the agent has completed the following goal.\n\n\
         GOAL: {}\n\n\
         CURRENT PLAN STATUS:\n{}\n\n\
         \n\
         Consider:\n\
         - All nodes marked [D] (done) are completed\n\
         - Nodes marked [F] (failed) need retry\n\
         - Nodes marked [I] (in progress) are being worked on\n\
         - Nodes marked [P] (pending) have not started\n\n\
         \n\
         Respond with JSON:\n\
         {{\"done\": true|false, \"reason\": \"explanation\"}}\n\
         \n\
         Respond \"done\": false if:\n\
         - There are pending [P] or in-progress [I] nodes\n\
         - There are failed [F] nodes that need retry\n\
         - The goal's purpose is not clearly achieved\n\
         \n\
         Respond \"done\": true only when ALL nodes are done [D].",
        goal, nodes_status
    )
}

/// Async judge function that calls the LLM to evaluate goal completion.
///
/// The `transport` is used to make an auxiliary LLM call. The `goal` and `plan`
/// provide context for the evaluation. Returns a `JudgeVerdict` parsed from the
/// LLM's response, or falls back to the heuristic if the call fails.
pub async fn call_judge(
    transport: &dyn oben_models::TransportProvider,
    goal: &str,
    plan: &PlanState,
) -> super::judge::JudgeVerdict {
    let prompt = build_judge_prompt(goal, plan);
    let messages = vec![Message::user(&prompt)];

    // Make the LLM call with a bounded budget to avoid infinite loops
    let call_mode = oben_models::CallMode::Fresh(String::new());

    // Try to get a response from the LLM
    let response_text = match transport.chat(&messages, &call_mode).await {
        Ok(resp) => resp.text,
        Err(e) => {
            tracing::warn!("LLM judge call failed, falling back to heuristic: {}", e);
            return heuristic_judge_verdict(goal, plan);
        }
    };

    // Parse the LLM's JSON response into a verdict
    let verdict = parse_judge_response(&response_text);

    // Validate the verdict - if it's Skipped due to parse failure,
    // check if the heuristic gives us a better signal
    match &verdict {
        JudgeVerdict::Skipped(reason) => {
            tracing::warn!("LLM judge returned Skipped: {}", reason);
            heuristic_judge_verdict(goal, plan)
        }
        _ => verdict,
    }
}

/// Heuristic fallback judge when LLM is unavailable.
///
/// This simple heuristic checks plan node statuses to determine if the
/// goal is likely complete. Used as a fallback when the LLM call fails.
fn heuristic_judge_verdict(_goal: &str, plan: &PlanState) -> JudgeVerdict {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlanNode;

    #[test]
    fn test_build_judge_prompt_contains_goal_and_plan() {
        let mut plan = PlanState::new("Test goal");
        plan.add_node(PlanNode::new("Do step 1"));
        plan.add_node(PlanNode::new("Do step 2"));

        let prompt = build_judge_prompt("Test goal", &plan);

        assert!(prompt.contains("Test goal"));
        assert!(prompt.contains("Do step 1"));
        assert!(prompt.contains("Do step 2"));
        assert!(prompt.contains("{\"done\":"));
        assert!(prompt.contains("\"reason\":"));
    }
}
