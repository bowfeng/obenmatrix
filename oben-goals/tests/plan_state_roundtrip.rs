use oben_goals::plan::{PlanNode, NodeStatus};

/// A plan state serializes and deserializes cleanly.
#[test]
fn test_plan_state_roundtrip_json() {
    let mut plan = oben_goals::plan_state::PlanState::new("build a web scraper");
    plan.add_node(
        PlanNode::new("research existing codebase")
            .description("Read the project files to understand structure")
    );
    plan.add_node(
        PlanNode::new("implement fetcher")
            .description("Create HTTP fetcher using reqwest")
    );

    let json = serde_json::to_string(&plan).unwrap();
    let back: oben_goals::plan_state::PlanState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.goal, "build a web scraper");
    assert_eq!(back.nodes.len(), 2);
    assert_eq!(back.nodes[0].title, "research existing codebase");
}

/// Plan state can save to a file and load back.
#[test]
fn test_plan_state_save_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");

    let mut plan = oben_goals::plan_state::PlanState::new("test goal");
    plan.add_node(PlanNode::new("step one"));
    plan.save_to_file(&path).unwrap();

    let loaded = oben_goals::plan_state::PlanState::load_from_file(&path).unwrap();
    assert_eq!(loaded.goal, "test goal");
    assert_eq!(loaded.nodes.len(), 1);
}

/// Plan state reports total pending nodes.
#[test]
fn test_plan_state_pending_count() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    plan.add_node(PlanNode::new("pending 1"));
    plan.add_node(PlanNode::new("pending 2"));

    let mut done = PlanNode::new("done 1");
    done.mark_done("done!");
    plan.add_node(done);

    assert_eq!(plan.pending_count(), 2);
    assert_eq!(plan.done_count(), 1);
    assert_eq!(plan.total_count(), 3);
}

/// Plan state reports total completed nodes.
#[test]
fn test_plan_state_done_count() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    let mut n1 = PlanNode::new("node 1");
    n1.mark_done("done 1");
    let mut n2 = PlanNode::new("node 2");
    n2.mark_done("done 2");
    plan.add_node(n1);
    plan.add_node(n2);

    assert_eq!(plan.done_count(), 2);
}

/// Plan state finds the next pending node.
#[test]
fn test_plan_state_next_pending() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    let mut done = PlanNode::new("skip me");
    done.mark_done("done!");
    plan.add_node(done);
    plan.add_node(PlanNode::new("do me next"));
    plan.add_node(PlanNode::new("later"));

    assert_eq!(plan.next_pending_node().map(|n| n.title.as_str()), Some("do me next"));
}

/// Plan state returns None when all nodes are done.
#[test]
fn test_plan_state_no_pending_nodes() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    let mut n = PlanNode::new("only one");
    n.mark_done("done!");
    plan.add_node(n);

    assert!(plan.next_pending_node().is_none());
}

/// Plan state marks a node as in-progress.
#[test]
fn test_plan_state_mark_in_progress() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    plan.add_node(PlanNode::new("task"));
    plan.mark_in_progress("task");

    let node = plan.find_node("task").unwrap();
    assert!(matches!(node.status, NodeStatus::InProgress));
}

/// Plan state marks a node as done.
#[test]
fn test_plan_state_mark_done() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    plan.add_node(PlanNode::new("task"));
    plan.mark_done("task", "completed successfully");

    let node = plan.find_node("task").unwrap();
    assert!(matches!(node.status, NodeStatus::Done));
    assert_eq!(node.completion_message, Some("completed successfully".to_string()));
}

/// Plan state marks a node as failed.
#[test]
fn test_plan_state_mark_failed() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    plan.add_node(PlanNode::new("task"));
    plan.mark_failed("task", "something broke");

    let node = plan.find_node("task").unwrap();
    assert!(matches!(node.status, NodeStatus::Failed));
    assert_eq!(node.failure_reason, Some("something broke".to_string()));
}

/// Plan state finds a node by title.
#[test]
fn test_plan_state_find_node() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    plan.add_node(PlanNode::new("research"));
    plan.add_node(PlanNode::new("implement"));

    assert!(plan.find_node("research").is_some());
    assert!(plan.find_node("implement").is_some());
    assert!(plan.find_node("nonexistent").is_none());
}

/// Plan state markdown includes all nodes.
#[test]
fn test_plan_state_markdown_format() {
    let mut plan = oben_goals::plan_state::PlanState::new("build scraper");
    plan.add_node(PlanNode::new("research"));
    let mut impl_node = PlanNode::new("implement").status(NodeStatus::Done);
    impl_node.mark_done("done");
    plan.add_node(impl_node);
    plan.add_node(PlanNode::new("test"));

    let md = plan.to_markdown();
    assert!(md.contains("build scraper"));
    assert!(md.contains("research"));
    assert!(md.contains("implement"));
    assert!(md.contains("test"));
    assert!(md.contains("[x]")); // done checkbox
    assert!(md.contains("[ ]")); // pending checkbox
}

/// Plan state markdown includes sub-nodes.
#[test]
fn test_plan_state_markdown_with_sub_nodes() {
    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    let mut test_node = PlanNode::new("testing").description("QA phase");
    test_node.push_sub_node(PlanNode::new("unit tests"));
    test_node.push_sub_node(PlanNode::new("integration tests"));
    plan.add_node(test_node);

    let md = plan.to_markdown();
    assert!(md.contains("testing"));
    assert!(md.contains("unit tests"));
    assert!(md.contains("integration tests"));
}

/// A plan state with no nodes has 0 pending.
#[test]
fn test_plan_state_empty() {
    let plan = oben_goals::plan_state::PlanState::new("empty goal");
    assert_eq!(plan.total_count(), 0);
    assert_eq!(plan.pending_count(), 0);
    assert!(plan.next_pending_node().is_none());
}

/// Plan state saves and loads preserves status.
#[test]
fn test_plan_state_preserves_status_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");

    let mut plan = oben_goals::plan_state::PlanState::new("complex goal");
    let mut n1 = PlanNode::new("done node");
    n1.mark_done("done");
    plan.add_node(n1);
    plan.add_node(PlanNode::new("pending node"));
    let mut n3 = PlanNode::new("failed node");
    n3.mark_failed("broken");
    plan.add_node(n3);
    plan.save_to_file(&path).unwrap();

    let loaded = oben_goals::plan_state::PlanState::load_from_file(&path).unwrap();
    assert!(matches!(loaded.nodes[0].status, NodeStatus::Done));
    assert!(matches!(loaded.nodes[1].status, NodeStatus::Pending));
    assert!(matches!(loaded.nodes[2].status, NodeStatus::Failed));
}

/// Plan state preserves sub-nodes on save/load.
#[test]
fn test_plan_state_preserves_sub_nodes_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");

    let mut plan = oben_goals::plan_state::PlanState::new("goal");
    let mut parent = PlanNode::new("parent").description("parent desc");
    parent.push_sub_node(PlanNode::new("child 1"));
    parent.push_sub_node(PlanNode::new("child 2"));
    plan.add_node(parent);
    plan.save_to_file(&path).unwrap();

    let loaded = oben_goals::plan_state::PlanState::load_from_file(&path).unwrap();
    let parent_loaded = loaded.find_node("parent").unwrap();
    assert_eq!(parent_loaded.sub_nodes.len(), 2);
    assert_eq!(parent_loaded.sub_nodes[0].title, "child 1");
}
