//! Integration tests for multi-agent system
//!
//! Tests verify:
//! 1. Multi agent startup from config
//! 2. AgentRegistry managing multiple agents
//! 3. Messaging channel for agent-to-agent communication
//!
//! All tests use in-memory/test-only agents - no real LLM calls

use std::sync::Arc;
use tokio::sync::Mutex;

use oben_agent::{AgentBuilder, AgentRegistry};
use oben_config::{AppConfig, NamedAgentConfig};

/// Minimal test agent - no real LLM calls
async fn make_test_agent(name: &str) -> Arc<Mutex<oben_agent::Agent>> {
    let agent = AgentBuilder::new()
        .with_config(AppConfig::default())
        .with_system_prompt(format!("You are test agent: {}", name))
        .with_tools(Arc::new(oben_tools::ToolRegistry::new()))
        .with_agent_name(Some(name.to_string()))
        .build()
        .await
        .unwrap();

    Arc::new(Mutex::new(agent))
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: Multi agent from config
// ─────────────────────────────────────────────────────────────────────────────

/// Given: AppConfig with multiple agents defined
/// When: AgentRegistry loads from config
/// Then: All agents are registered with correct names
#[tokio::test]
async fn test_multi_agent_from_config() {
    // Setup config with multiple agents
    let mut config = AppConfig::default();
    config.agents = vec![
        NamedAgentConfig {
            name: "worker".to_string(),
            role: "Worker agent - executes tasks".to_string(),
            model: "test-model".to_string(),
            tools: vec![],
            execution_discipline: None,
        },
        NamedAgentConfig {
            name: "manager".to_string(),
            role: "Manager agent - coordinates workers".to_string(),
            model: "test-model".to_string(),
            tools: vec![],
            execution_discipline: None,
        },
        NamedAgentConfig {
            name: "monitor".to_string(),
            role: "Monitor agent - tracks system health".to_string(),
            model: "test-model".to_string(),
            tools: vec![],
            execution_discipline: None,
        },
    ];

    // Load agents from config
    let registry = AgentRegistry::new();
    registry.from_config(&config).await.unwrap();

    // Verify all agents registered
    assert_eq!(registry.len(), 3);
    assert!(registry.get("worker").is_some());
    assert!(registry.get("manager").is_some());
    assert!(registry.get("monitor").is_some());

    let agent_names = registry.list_agents();
    assert!(agent_names.contains(&"worker".to_string()));
    assert!(agent_names.contains(&"manager".to_string()));
    assert!(agent_names.contains(&"monitor".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: AgentRegistry manages multiple agents
// ─────────────────────────────────────────────────────────────────────────────

/// Given: Empty registry
/// When: Multiple agents inserted
/// Then: Registry correctly stores and retrieves agents
#[tokio::test]
async fn test_agent_registry_manages_multiple() {
    let registry = AgentRegistry::new();

    // Insert multiple agents
    let worker = make_test_agent("worker").await;
    let manager = make_test_agent("manager").await;
    let monitor = make_test_agent("monitor").await;

    registry.insert("worker".to_string(), Arc::clone(&worker));
    registry.insert("manager".to_string(), Arc::clone(&manager));
    registry.insert("monitor".to_string(), Arc::clone(&monitor));

    // Verify count
    assert_eq!(registry.len(), 3);
    assert!(!registry.is_empty());

    // Verify get returns correct agents
    let retrieved_worker = registry.get("worker").unwrap();
    let retrieved_manager = registry.get("manager").unwrap();
    let retrieved_monitor = registry.get("monitor").unwrap();

    assert!(Arc::ptr_eq(&retrieved_worker, &worker));
    assert!(Arc::ptr_eq(&retrieved_manager, &manager));
    assert!(Arc::ptr_eq(&retrieved_monitor, &monitor));

    // Verify list_agents returns all names
    let names = registry.list_agents();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"worker".to_string()));
    assert!(names.contains(&"manager".to_string()));
    assert!(names.contains(&"monitor".to_string()));
}

/// Given: Registry with multiple agents
/// When: One agent is removed
/// Then: Remaining agents still accessible, count decremented
#[tokio::test]
async fn test_agent_registry_remove() {
    let registry = AgentRegistry::new();

    let agent1 = make_test_agent("agent1").await;
    let agent2 = make_test_agent("agent2").await;
    let agent3 = make_test_agent("agent3").await;

    registry.insert("agent1".to_string(), Arc::clone(&agent1));
    registry.insert("agent2".to_string(), Arc::clone(&agent2));
    registry.insert("agent3".to_string(), Arc::clone(&agent3));

    assert_eq!(registry.len(), 3);

    // Remove middle agent
    let removed = registry.remove("agent2");
    assert!(removed.is_some());
    assert_eq!(registry.len(), 2);

    // Verify removed agent is gone
    assert!(registry.get("agent2").is_none());

    // Verify remaining agents still accessible
    assert!(registry.get("agent1").is_some());
    assert!(registry.get("agent3").is_some());

    // Try remove non-existent
    let removed_again = registry.remove("agent2");
    assert!(removed_again.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: Messaging channel - agent-to-agent communication
// ─────────────────────────────────────────────────────────────────────────────

use oben_gateway::messaging::Channel;

/// Given: Channel with multiple subscribers
/// When: Message published to topic
/// Then: All subscribers receive the message
#[tokio::test]
async fn test_channel_multiple_subscribers() {
    let channel = Channel::new();

    // Multiple subscribers to same topic
    let mut sub1 = channel.subscribe("task-queue");
    let mut sub2 = channel.subscribe("task-queue");
    let mut sub3 = channel.subscribe("task-queue");

    // Publish message
    channel.publish("task-queue", "task-1".to_string());

    // All subscribers receive
    let msg1 = sub1.recv().await;
    let msg2 = sub2.recv().await;
    let msg3 = sub3.recv().await;

    assert_eq!(msg1, Some("task-1".to_string()));
    assert_eq!(msg2, Some("task-1".to_string()));
    assert_eq!(msg3, Some("task-1".to_string()));
}

/// Given: Multiple topics with subscribers
/// When: Messages published to different topics
/// Then: Each topic's subscribers receive only their topic's messages
#[tokio::test]
async fn test_channel_topic_isolation() {
    let channel = Channel::new();

    let mut worker_rx = channel.subscribe("worker:task");
    let mut manager_rx = channel.subscribe("manager:指令");

    // Worker gets task
    channel.publish("worker:task", "execute task A".to_string());
    
    // Manager gets instruction
    channel.publish("manager:指令", "review work".to_string());

    let worker_msg = worker_rx.recv().await;
    let manager_msg = manager_rx.recv().await;

    assert_eq!(worker_msg, Some("execute task A".to_string()));
    assert_eq!(manager_msg, Some("review work".to_string()));
}

/// Given: Agent subscribes to channel before other agents publish
/// When: Message published to subscribed topic
/// Then: Agent receives message via channel
#[tokio::test]
async fn test_agent_communication_flow() {
    let channel = Channel::new();

    // "Worker" subscribes to input
    let mut worker_input = channel.subscribe("worker:input");

    // "Manager" publishes task completion
    channel.publish("worker:input", "task completed".to_string());

    // Worker receives
    let msg = worker_input.recv().await;
    assert_eq!(msg, Some("task completed".to_string()));
}

/// Given: Channel with no subscribers to a topic
/// When: Message published to that topic
/// Then: No panic, message silently dropped
#[tokio::test]
async fn test_channel_no_leak_on_unsubscribed_topic() {
    let channel = Channel::new();

    // Publish to topic with no subscribers
    channel.publish("no-subscribers", "message".to_string());

    // Should not panic - just no receivers
    assert_eq!(channel.subscriber_count("no-subscribers"), 0);
}

/// Given: Broadcast called on channel
/// When: Multiple subscribers to different topics
/// Then: All subscribers receive broadcast message
#[tokio::test]
async fn test_channel_broadcast() {
    let channel = Channel::new();

    let mut sub_a = channel.subscribe("topic-a");
    let mut sub_b = channel.subscribe("topic-b");
    let mut sub_c = channel.subscribe("topic-c");

    // Broadcast to all
    channel.broadcast("global-alert".to_string());

    let msg_a = sub_a.recv().await;
    let msg_b = sub_b.recv().await;
    let msg_c = sub_c.recv().await;

    assert_eq!(msg_a, Some("global-alert".to_string()));
    assert_eq!(msg_b, Some("global-alert".to_string()));
    assert_eq!(msg_c, Some("global-alert".to_string()));
}
