use oben_goals::plan::{NodeStatus, PlanNode};

/// A plan node serializes and deserializes cleanly to/from JSON.
#[test]
fn test_plan_node_roundtrip_json() {
    let node = PlanNode::new("implement fetcher")
        .description("Write the HTTP fetcher module")
        .status(NodeStatus::InProgress);

    let json = serde_json::to_string(&node).unwrap();
    let back: PlanNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title, "implement fetcher");
    assert_eq!(
        back.description,
        Some("Write the HTTP fetcher module".to_string())
    );
    assert!(matches!(back.status, NodeStatus::InProgress));
}

/// A plan node can have sub-nodes.
#[test]
fn test_plan_node_with_sub_nodes() {
    let child = PlanNode::new("write test script");
    let mut parent = PlanNode::new("integration test");
    parent.push_sub_node(child);

    let json = serde_json::to_string(&parent).unwrap();
    let back: PlanNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title, "integration test");
    assert_eq!(back.sub_nodes.len(), 1);
    assert_eq!(back.sub_nodes[0].title, "write test script");
}

/// A plan node starts as pending by default.
#[test]
fn test_plan_node_default_status() {
    let node = PlanNode::new("some task");
    assert!(matches!(node.status, NodeStatus::Pending));
}

/// A plan node can transition from pending to done (mutable).
#[test]
fn test_plan_node_transition_to_done() {
    let mut node = PlanNode::new("task");
    assert!(matches!(node.status, NodeStatus::Pending));
    node.mark_done("completed successfully");
    assert!(matches!(node.status, NodeStatus::Done));
}

/// A plan node can transition from pending to failed (mutable).
#[test]
fn test_plan_node_transition_to_failed() {
    let mut node = PlanNode::new("task");
    node.mark_failed("something broke");
    assert!(matches!(node.status, NodeStatus::Failed));
}

/// Done nodes record a completion message.
#[test]
fn test_plan_node_completion_message() {
    let mut node = PlanNode::new("task");
    node.mark_done("created main.rs");
    assert_eq!(node.completion_message, Some("created main.rs".to_string()));
}

/// Failed nodes record a failure reason.
#[test]
fn test_plan_node_failure_reason() {
    let mut node = PlanNode::new("task");
    node.mark_failed("file not found");
    assert_eq!(node.failure_reason, Some("file not found".to_string()));
}

/// A plan node can have a list of artifacts produced.
#[test]
fn test_plan_node_artifacts() {
    let mut node = PlanNode::new("build feature");
    node.push_artifact("src/main.rs");
    node.push_artifact("tests/integration.rs");

    let json = serde_json::to_string(&node).unwrap();
    let back: PlanNode = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.artifacts,
        vec![
            "src/main.rs".to_string(),
            "tests/integration.rs".to_string()
        ]
    );
}

/// A plan node creates an empty sub-nodes list.
#[test]
fn test_plan_node_empty_sub_nodes() {
    let node = PlanNode::new("task");
    assert!(node.sub_nodes.is_empty());
}

/// A plan node's created_at timestamp is set on creation.
#[test]
fn test_plan_node_created_at() {
    let node = PlanNode::new("task");
    assert!(node.created_at > chrono::Utc::now() - chrono::Duration::hours(1));
}

/// A plan node can be set done via builder, then round-trips.
#[test]
fn test_plan_node_full_roundtrip() {
    let mut node = PlanNode::new("full test node")
        .description("A node with everything set")
        .status(NodeStatus::Done);
    node.push_artifact("file1.txt");
    node.mark_done("all done");

    let json = serde_json::to_string(&node).unwrap();
    let back: PlanNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title, "full test node");
    assert_eq!(
        back.description,
        Some("A node with everything set".to_string())
    );
    assert!(matches!(back.status, NodeStatus::Done));
    assert_eq!(back.completion_message, Some("all done".to_string()));
    assert_eq!(back.artifacts.len(), 1);
}

/// PlanNode::is_leaf returns true for nodes with no sub-nodes.
#[test]
fn test_plan_node_is_leaf() {
    let node = PlanNode::new("leaf");
    assert!(node.is_leaf());

    let mut parent = PlanNode::new("parent");
    parent.push_sub_node(PlanNode::new("child"));
    assert!(!parent.is_leaf());
}

/// PlanNode::total_count includes descendants.
#[test]
fn test_plan_node_total_count() {
    let mut node = PlanNode::new("root");
    let mut child = PlanNode::new("child");
    child.push_sub_node(PlanNode::new("grandchild"));
    child.push_sub_node(PlanNode::new("grandchild2"));
    node.push_sub_node(child);
    node.push_sub_node(PlanNode::new("sibling"));

    assert_eq!(node.total_count(), 5);
}

/// PlanNode markdown format.
#[test]
fn test_plan_node_markdown() {
    let mut node = PlanNode::new("task one").description("Description here");
    node.mark_done("done!");

    let md = node.to_markdown(0);
    assert!(md.contains("task one"));
    assert!(md.contains("Description here"));
    assert!(md.contains("[x]"));
}

/// PlanNode markdown with sub-nodes uses indentation.
#[test]
fn test_plan_node_markdown_indent() {
    let mut parent = PlanNode::new("parent");
    parent.push_sub_node(PlanNode::new("child"));

    let md = parent.to_markdown(0);
    assert!(md.contains("child"));
    assert!(md.contains("  - [ ] **child**"));
}
