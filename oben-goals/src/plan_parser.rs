/// Parse a markdown checklist into a PlanState.

use crate::plan::{NodeStatus, PlanNode};
use crate::plan_state::PlanState;

/// A parsed checklist item with its nesting level.
struct ListItem {
    level: usize,
    status: NodeStatus,
    title: String,
}

/// Parse the agent's plan creation output into a structured PlanState.
pub fn parse_plan_from_markdown(text: &str) -> Option<PlanState> {
    let mut lines = text.lines();

    // Look for the goal line
    let mut goal = None;
    for line in &mut lines {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("## Plan:") {
            goal = Some(rest.trim().to_string());
            break;
        }
        if let Some(rest) = line.strip_prefix("Plan:") {
            goal = Some(rest.trim().to_string());
            break;
        }
    }

    let goal = goal?;
    let mut state = PlanState::new(goal);

    // Collect items
    let mut items: Vec<ListItem> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let raw_indent = line.len() - line.ltrim_whitespace().len();
        let level = raw_indent / 2;

        let (status_str, title_raw) = match trimmed {
            s if s.starts_with("- [ ] ") => ("pending", &s["- [ ] ".len()..]),
            s if s.starts_with("- [x] ") => ("done", &s["- [x] ".len()..]),
            s if s.starts_with("- [X] ") => ("done", &s["- [X] ".len()..]),
            s if s.starts_with("- [~] ") => ("in_progress", &s["- [~] ".len()..]),
            s if s.starts_with("- [!] ") => ("failed", &s["- [!] ".len()..]),
            _ => continue,
        };

        let title = title_raw.strip_prefix("**").unwrap_or(title_raw);
        let title = title.strip_suffix("**").unwrap_or(title);
        let title = title.trim().to_string();

        let status = match status_str {
            "pending" => NodeStatus::Pending,
            "done" => NodeStatus::Done,
            "in_progress" => NodeStatus::InProgress,
            "failed" => NodeStatus::Failed,
            _ => NodeStatus::Pending,
        };

        items.push(ListItem { level, status, title });
    }

    if items.is_empty() {
        return Some(state);
    }

    // Build tree recursively. Each call processes items starting at the given level.
    state.nodes = build_subtree(&items, 0, 0);
    Some(state)
}

/// Build a subtree of nodes starting at `level`, beginning at index `start` in `items`.
/// Returns a Vec of PlanNodes.
/// Each node gets sub-nodes from items at `level + 1` until we hit an item at a lower level.
fn build_subtree(items: &[ListItem], level: usize, start: usize) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    let mut i = start;

    while i < items.len() && items[i].level == level {
        let item = &items[i];

        // Find the end of this item's children (next item at level <= current, or end)
        let child_start = i + 1;
        let mut child_end = child_start;
        while child_end < items.len() && items[child_end].level > level {
            child_end += 1;
        }

        // Build children recursively
        let sub_nodes = if child_start < child_end {
            build_subtree(items, level + 1, child_start)
        } else {
            Vec::new()
        };

        let mut node = PlanNode::new(item.title.clone()).status(item.status.clone());
        node.sub_nodes = sub_nodes;
        nodes.push(node);

        i = child_end;
    }

    nodes
}

/// Strip leading whitespace from a string.
trait LtrimWhitespace {
    fn ltrim_whitespace(&self) -> &str;
}

impl LtrimWhitespace for str {
    fn ltrim_whitespace(&self) -> &str {
        self.find(|c: char| !c.is_whitespace()).map_or("", |i| &self[i..])
    }
}

/// Parse a completion or failure message from the agent's response.
pub fn parse_node_complete(message: &str) -> Option<(String, String)> {
    if let Some(rest) = message.strip_prefix("### Node Complete:") {
        let rest = rest.trim();
        let title = extract_title(rest);
        let result = extract_field(rest, "result")
            .or_else(|| extract_field(rest, "Result"))
            .or_else(|| extract_after_dash(rest));
        result.map(|r| (title, r))
    } else if let Some(rest) = message.strip_prefix("### Node Failed:") {
        let rest = rest.trim();
        let title = extract_title(rest);
        let reason = extract_field(rest, "reason")
            .or_else(|| extract_field(rest, "Reason"))
            .or_else(|| extract_after_dash(rest));
        reason.map(|r| (title, r))
    } else {
        None
    }
}

fn extract_after_dash(s: &str) -> Option<String> {
    if let Some(pos) = s.find("—") {
        let after = s[pos + 1..].trim();
        if !after.is_empty() {
            return Some(after.to_string());
        }
    }
    None
}

fn extract_title(s: &str) -> String {
    if let Some(start) = s.find("**") {
        if let Some(end) = s[start + 2..].find("**") {
            return s[start + 2..start + 2 + end].to_string();
        }
    }
    if let Some(start) = s.find('\'') {
        if let Some(end) = s[start + 1..].find('\'') {
            return s[start + 1..start + 1 + end].to_string();
        }
    }
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn extract_field(s: &str, field: &str) -> Option<String> {
    let marker = format!("{}:", field);

    if let Some(pos) = s.find(&marker) {
        let after = &s[pos + marker.len()..];
        let end = after.find('—').map(|i| i).unwrap_or(after.len());
        let value = after[..end].trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_simple() {
        let input = "## Plan: build a scraper\n- [ ] research existing codebase\n- [ ] implement fetcher\n- [ ] integration test";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.goal, "build a scraper");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.nodes[0].title, "research existing codebase");
    }

    #[test]
    fn test_parse_plan_with_sub_nodes() {
        let input = "## Plan: build a scraper\n- [ ] research\n- [ ] implement fetcher\n  - [ ] write request handler\n  - [ ] parse HTML\n- [ ] test";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.goal, "build a scraper");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.nodes[1].sub_nodes.len(), 2);
        assert_eq!(plan.nodes[1].sub_nodes[0].title, "write request handler");
    }

    #[test]
    fn test_parse_plan_with_done_nodes() {
        let input = "## Plan: deploy\n- [x] write code\n- [ ] test\n- [ ] deploy";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert!(matches!(plan.nodes[0].status, NodeStatus::Done));
        assert!(matches!(plan.nodes[1].status, NodeStatus::Pending));
    }

    #[test]
    fn test_parse_plan_with_failed_nodes() {
        let input = "## Plan: deploy\n- [!] build\n- [ ] test";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert!(matches!(plan.nodes[0].status, NodeStatus::Failed));
    }

    #[test]
    fn test_parse_plan_with_bold_titles() {
        let input = "## Plan: test\n- [ ] **research existing codebase**\n- [ ] implement";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.nodes[0].title, "research existing codebase");
    }

    #[test]
    fn test_parse_plan_invalid_text() {
        assert!(parse_plan_from_markdown("This is not a plan").is_none());
        assert!(parse_plan_from_markdown("").is_none());
    }

    #[test]
    fn test_parse_plan_deeply_nested() {
        let input = "## Plan: nested\n- [ ] level 0\n  - [ ] level 1\n    - [ ] level 2\n      - [ ] level 3";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.nodes.len(), 1);
        assert_eq!(plan.nodes[0].sub_nodes.len(), 1);
        assert_eq!(plan.nodes[0].sub_nodes[0].sub_nodes.len(), 1);
        assert_eq!(plan.nodes[0].sub_nodes[0].sub_nodes[0].sub_nodes.len(), 1);
    }

    #[test]
    fn test_parse_node_complete_success() {
        let msg = "### Node Complete: 'implement fetcher' — Result: Created scraper.rs with fetch_url()";
        let (title, result) = parse_node_complete(msg).unwrap();
        assert_eq!(title, "implement fetcher");
        assert_eq!(result, "Created scraper.rs with fetch_url()");
    }

    #[test]
    fn test_parse_node_complete_with_result_field() {
        let msg = "### Node Complete: 'research' — Result: Found 3 relevant files";
        let (title, result) = parse_node_complete(msg).unwrap();
        assert_eq!(title, "research");
        assert_eq!(result, "Found 3 relevant files");
    }

    #[test]
    fn test_parse_node_complete_failed() {
        let msg = "### Node Failed: 'deploy' — Reason: No SSH access";
        let (title, reason) = parse_node_complete(msg).unwrap();
        assert_eq!(title, "deploy");
        assert_eq!(reason, "No SSH access");
    }

    #[test]
    fn test_parse_node_complete_no_match() {
        assert!(parse_node_complete("The agent is working").is_none());
    }

    #[test]
    fn test_parse_plan_preserves_sub_node_titles() {
        let input = "## Plan: build\n- [ ] core\n  - [ ] parser\n  - [ ] formatter\n- [ ] tests";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.nodes[0].sub_nodes[0].title, "parser");
        assert_eq!(plan.nodes[0].sub_nodes[1].title, "formatter");
    }

    #[test]
    fn test_parse_plan_2space_indent() {
        let input = "## Plan: test\n- [ ] top\n  - [ ] child";
        let plan = parse_plan_from_markdown(input).unwrap();
        assert_eq!(plan.nodes.len(), 1);
        assert_eq!(plan.nodes[0].sub_nodes.len(), 1);
    }
}
