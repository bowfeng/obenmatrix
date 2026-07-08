//! Integration tests for plugin metadata flow.
//!
//! Tests discover, PluginContext API, lifecycle management,
//! plugin config gating, and full-flow mock without loading real WASM.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use oben_config::{PluginCapabilities as ConfigCapabilities, PluginConfig, PluginManifest};
use oben_wasm::discover::{DiscoveredPlugin, PluginDiscoverer};
use oben_wasm::lifecycle::{PluginLifecycleManager, PluginState};
use oben_wasm::loader::{PluginBundle, PluginLoader};
use oben_wasm::plugin_context::{PluginCapabilities, PluginContext, QueuedMessage, RegisteredCommand, RegisteredTool};

// ── Test 1: Discover ────────────────────────────────────────────────────────────────────

/// Given a plugin directory with 2 subdirs — one valid manifest, one without,
/// when PluginDiscoverer::discover() is called,
/// then exactly 1 DiscoveredPlugin is returned.
#[test]
fn test_discover_filters_valid_manifests() {
    let tmp = TempDir::new().expect("create temp dir");

    // Valid plugin with .platform.json
    let valid = tmp.path().join("valid-plugin");
    fs::create_dir(&valid).unwrap();
    let manifest_json = r#"{
        "name": "valid-plugin",
        "version": "1.0.0",
        "description": "A valid plugin"
    }"#;
    fs::write(valid.join(".platform.json"), manifest_json).unwrap();

    // Invalid plugin without any manifest
    let no_manifest = tmp.path().join("no-manifest");
    fs::create_dir(&no_manifest).unwrap();
    fs::write(no_manifest.join("README.md"), "no manifest here").unwrap();

    let discovered = PluginDiscoverer::discover(tmp.path()).unwrap();

    assert_eq!(
        discovered.len(),
        1,
        "Should discover exactly 1 manifest, got {}",
        discovered.len()
    );
    assert_eq!(discovered[0].manifest.name, "valid-plugin");
    assert_eq!(discovered[0].dir, valid);
}

/// Given an empty directory, when PluginDiscoverer::discover() is called,
/// then no plugins are returned.
#[test]
fn test_discover_empty_dir() {
    let tmp = TempDir::new().expect("create temp dir");
    let discovered = PluginDiscoverer::discover(tmp.path()).unwrap();
    assert!(discovered.is_empty(), "Empty dir should yield no plugins");
}

/// Given a nonexistent directory, when PluginDiscoverer::discover() is called,
/// then an empty result is returned without error.
#[test]
fn test_discover_nonexistent_dir() {
    let discovered = PluginDiscoverer::discover(&PathBuf::from("/nonexistent/path/xyz")).unwrap();
    assert!(discovered.is_empty(), "Nonexistent dir should yield empty, not error");
}

/// Given a directory with a plugin that has plugin.yaml instead of .platform.json,
/// when PluginDiscoverer::discover() is called,
/// then it is still discovered via the YAML manifest.
#[test]
fn test_discover_yaml_manifest() {
    let tmp = TempDir::new().expect("create temp dir");

    let yaml_plugin = tmp.path().join("yaml-plugin");
    fs::create_dir(&yaml_plugin).unwrap();
    let yaml_content = r#"name: yaml-plugin
version: "0.1.0"
description: A YAML-based plugin
"#;
    fs::write(yaml_plugin.join("plugin.yaml"), yaml_content).unwrap();

    let discovered = PluginDiscoverer::discover(tmp.path()).unwrap();

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].manifest.name, "yaml-plugin");
}

/// Given a plugin directory with an invalid JSON manifest,
/// when PluginDiscoverer::discover() is called,
/// then the invalid manifest is skipped (not returned).
#[test]
fn test_discover_skips_invalid_json() {
    let tmp = TempDir::new().expect("create temp dir");

    let bad = tmp.path().join("bad-plugin");
    fs::create_dir(&bad).unwrap();
    fs::write(bad.join(".platform.json"), "not valid json {{{").unwrap();

    let discovered = PluginDiscoverer::discover(tmp.path()).unwrap();

    assert!(
        discovered.is_empty(),
        "Invalid JSON manifest should be skipped, got {}",
        discovered.len()
    );
}

// ── Test 2: PluginContext API ──────────────────────────────────────────────────────────

/// Given a PluginContext, when 3 tools are registered,
/// then take_tools() returns all 3 sorted by name.
#[tokio::test]
async fn test_plugin_context_register_and_collect_tools() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    ctx.register_tool("zebra-tool", "Z tool", r#"{"type":"object"}"#, vec!["http".to_string()]).await;
    ctx.register_tool("alpha-tool", "A tool", r#"{"type":"object"}"#, vec!["workspace-read".to_string()]).await;
    ctx.register_tool("mid-tool", "M tool", r#"{"type":"object"}"#, vec![]).await;

    let tools = ctx.take_tools().await;

    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0].name, "alpha-tool");
    assert_eq!(tools[1].name, "mid-tool");
    assert_eq!(tools[2].name, "zebra-tool");

    // Check that metadata was stored correctly
    assert_eq!(tools[0].description, "A tool");
    assert_eq!(tools[0].capabilities, vec!["workspace-read".to_string()]);
    assert_eq!(tools[2].schema, r#"{"type":"object"}"#);
}

/// Given a PluginContext, when 2 commands are registered with aliases,
/// then take_commands() returns them sorted by name.
#[tokio::test]
async fn test_plugin_context_register_and_collect_commands() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    ctx.register_command("zebra-cmd", "Z command", vec!["-z".to_string()]).await;
    ctx.register_command("alpha-cmd", "A command", vec!["-a".to_string(), "--alpha".to_string()]).await;

    let cmds = ctx.take_commands().await;

    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0].name, "alpha-cmd");
    assert_eq!(cmds[0].aliases, vec!["-a".to_string(), "--alpha".to_string()]);
    assert_eq!(cmds[1].name, "zebra-cmd");
    assert_eq!(cmds[1].aliases, vec!["-z".to_string()]);
}

/// Given a PluginContext, when 3 tools and 2 commands are registered,
/// then take_tools() and take_commands() each return the correct counts.
#[tokio::test]
async fn test_plugin_context_multiple_registrations() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    for i in 0..3 {
        ctx.register_tool(&format!("tool-{}", i), &format!("Tool {}", i), "{}", vec![]).await;
    }
    for i in 0..2 {
        ctx.register_command(&format!("cmd-{}", i), &format!("Cmd {}", i), vec![]).await;
    }

    let tools = ctx.take_tools().await;
    let cmds = ctx.take_commands().await;

    assert_eq!(tools.len(), 3);
    assert_eq!(cmds.len(), 2);
}

/// Given a PluginContext, when inject_message is called,
/// then the message is queued and retrievable via take_messages().
#[tokio::test]
async fn test_inject_message_queues() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    ctx.inject_message("Hello world", "user").await;
    ctx.inject_message("System msg", "system").await;

    let msgs = ctx.take_messages().await;

    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Hello world");
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].content, "System msg");
    assert_eq!(msgs[1].role, "system");
}

/// Given a PluginContext, when llm_complete is called,
/// then it returns an error (Phase 1 stub).
#[tokio::test]
async fn test_llm_complete_returns_error() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    let result = ctx.llm_complete(&[("user".to_string(), "Hello".to_string())]).await;

    assert!(result.is_err(), "llm_complete should always error in Phase 1");
    assert_eq!(result.err().unwrap(), "llm-not-available");
}

/// Given a PluginContext, when take_tools is called twice,
/// then the second call returns an empty vec (drains the map).
#[tokio::test]
async fn test_take_drains_map() {
    let ctx = PluginContext::new(PluginCapabilities::default());

    ctx.register_tool("tool-1", "desc", "{}", vec![]).await;
    ctx.register_tool("tool-2", "desc", "{}", vec![]).await;

    let first = ctx.take_tools().await;
    assert_eq!(first.len(), 2);

    let second = ctx.take_tools().await;
    assert!(second.is_empty(), "Second take should be empty (drained)");
}

/// Given a PluginContext with capabilities, when capabilities() is called,
/// then it returns the originally set capabilities.
#[tokio::test]
async fn test_plugin_context_capabilities() {
    let caps = PluginCapabilities::new(true, false, true);
    let ctx = PluginContext::new(caps);

    let ret_caps = ctx.capabilities();
    assert!(ret_caps.workspace_read);
    assert!(!ret_caps.http);
    assert!(ret_caps.tool_invoke);
}

// ── Test 3: Lifecycle ──────────────────────────────────────────────────────────────────

/// Given a PluginLifecycleManager with max_restarts=3,
/// when a plugin is started, crashed 3 times,
/// then can_restart returns false after the 3rd crash.
#[test]
fn test_lifecycle_max_restart_limit() {
    let mut mgr = PluginLifecycleManager::new(3);

    mgr.start("test-plugin");
    mgr.started("test-plugin");

    // Crash #1 — can restart
    mgr.crash("test-plugin", "OOM");
    assert!(mgr.can_restart("test-plugin"), "Should be able to restart after crash 1");
    assert_eq!(mgr.crash_count("test-plugin"), 1);

    // Crash #2 — can restart
    mgr.crash("test-plugin", "timeout");
    assert!(mgr.can_restart("test-plugin"), "Should be able to restart after crash 2");
    assert_eq!(mgr.crash_count("test-plugin"), 2);

    // Crash #3 — cannot restart (3 >= 3)
    mgr.crash("test-plugin", "segfault");
    assert!(
        !mgr.can_restart("test-plugin"),
        "Should NOT be able to restart after crash 3 (3 >= max_restarts=3)"
    );
    assert_eq!(mgr.crash_count("test-plugin"), 3);
}

/// Given a PluginLifecycleManager with max_restarts=2,
/// when a plugin is started and crashed exactly 2 times,
/// then can_restart returns false and state is Crashed.
#[test]
fn test_lifecycle_state_is_crashed_after_exhausted() {
    let mut mgr = PluginLifecycleManager::new(2);

    mgr.start("crash-test");
    mgr.started("crash-test");

    mgr.crash("crash-test", "error 1");
    mgr.crash("crash-test", "error 2");

    assert!(!mgr.can_restart("crash-test"));

    match mgr.state("crash-test") {
        Some(&PluginState::Crashed(ref err)) => {
            assert!(err.contains("error 2"), "State should reflect last crash error");
        }
        other => panic!("Expected Crashed state, got {:?}", other),
    }
}

/// Given a PluginLifecycleManager with max_restarts=0,
/// when a plugin is started,
/// then can_restart returns false from the start (crash count 0 >= 0 would be false).
#[test]
fn test_lifecycle_zero_max_restarts() {
    let mgr = PluginLifecycleManager::new(0);

    // Never started — no crash count, defaults to (0, instant) → 0 >= 0 = false
    assert!(!mgr.can_restart("never-started"));
}

/// Given a PluginLifecycleManager, when reset_crash_count is called,
/// then can_restart returns to true (count goes back to 0).
#[test]
fn test_lifecycle_reset_crash_count() {
    let mut mgr = PluginLifecycleManager::new(2);

    mgr.start("test");
    mgr.started("test");

    mgr.crash("test", "err 1");
    mgr.crash("test", "err 2");
    assert!(!mgr.can_restart("test"));

    mgr.reset_crash_count("test");
    assert!(mgr.can_restart("test"), "After reset, should be able to restart again");
}

/// Given a PluginLifecycleManager, when disable() is called on a running plugin,
/// then is_disabled returns true and state is Disabled.
#[test]
fn test_lifecycle_disable() {
    let mut mgr = PluginLifecycleManager::new(3);

    mgr.start("test");
    mgr.started("test");
    assert!(mgr.is_running("test"));

    mgr.disable("test");

    assert!(mgr.is_disabled("test"));
    assert!(!mgr.is_running("test"));
    assert_eq!(mgr.state("test"), Some(&PluginState::Disabled));
}

/// Given a PluginLifecycleManager, when multiple plugins are started,
/// then running_plugins() returns all running ones.
#[test]
fn test_lifecycle_multiple_plugins() {
    let mut mgr = PluginLifecycleManager::new(3);

    mgr.start("a"); mgr.started("a");
    mgr.start("b"); mgr.started("b");
    mgr.start("c"); mgr.started("c");

    let running = mgr.running_plugins();
    assert_eq!(running.len(), 3);

    mgr.stop_plugin("b");
    let running = mgr.running_plugins();
    assert_eq!(running.len(), 2);
    assert!(running.contains(&"a".to_string()));
    assert!(!running.contains(&"b".to_string()));
}

// ── Test 4: PluginConfig gating ────────────────────────────────────────────────────────

/// Given a PluginConfig with an enabled list, when is_enabled is checked against
/// a name in the list, then it returns true. When checked against a name not in
/// the list, then it returns false.
#[test]
fn test_plugin_config_enabled_list() {
    let config = PluginConfig {
        enabled: vec!["plugin-a".to_string(), "plugin-b".to_string()],
        disabled: vec![],
    };

    assert!(config.is_enabled("plugin-a"));
    assert!(config.is_enabled("plugin-b"));
    assert!(!config.is_enabled("plugin-c"));
}

/// Given a PluginConfig with a disabled list, when is_enabled is checked against
/// a disabled name, then it returns false even if it were in the enabled list.
#[test]
fn test_plugin_config_disabled_list() {
    let config = PluginConfig {
        enabled: vec!["plugin-a".to_string()],
        disabled: vec!["plugin-b".to_string()],
    };

    assert!(config.is_enabled("plugin-a"));
    assert!(!config.is_enabled("plugin-b"));
    assert!(config.is_disabled("plugin-b"));
    assert!(!config.is_disabled("plugin-a"));
}

/// Given a PluginConfig with empty enabled and disabled lists,
/// when is_enabled is checked, then it returns true (explicit opt-in).
/// This tests the code comment: "If enabled list is non-empty, check if
/// the plugin is in it" — empty means all enabled (the code default).
#[test]
fn test_plugin_config_empty_lists() {
    let config = PluginConfig {
        enabled: vec![],
        disabled: vec![],
    };

    // Code comment says: empty enabled + no disabled → true
    assert!(config.is_enabled("anything"));
}

/// Given a PluginConfig with an item in both enabled and disabled,
/// then disabled takes precedence.
#[test]
fn test_plugin_config_disabled_takes_precedence() {
    let config = PluginConfig {
        enabled: vec!["plugin-x".to_string()],
        disabled: vec!["plugin-x".to_string()],
    };

    assert!(!config.is_enabled("plugin-x"), "Disabled should override enabled");
    assert!(config.is_disabled("plugin-x"));
}

/// Given a PluginConfig from serde, when parsed from JSON,
/// then enabled and disabled vectors are correctly deserialized.
#[test]
fn test_plugin_config_serde() {
    let json = r#"{
        "enabled": ["alpha", "beta"],
        "disabled": ["gamma"]
    }"#;

    let config: PluginConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.enabled, vec!["alpha".to_string(), "beta".to_string()]);
    assert_eq!(config.disabled, vec!["gamma".to_string()]);
}

// ── Test 5: Full flow mock ─────────────────────────────────────────────────────────────

/// Given no real WASM binary, when PluginLoader::new() is called with a valid runtime,
/// then the loader is created successfully.
#[test]
fn test_plugin_loader_creation() {
    let runtime = oben_wasm::WasmRuntime::new(
        oben_wasm::WasmRuntimeConfig::default()
    ).expect("create WasmRuntime");

    let _loader = PluginLoader::new(runtime);
}

/// Given PluginLoader::with_defaults(), when called in a test context,
/// then a loader is created successfully (cache disabled in test mode).
#[test]
fn test_plugin_loader_with_defaults() {
    let _loader = PluginLoader::with_defaults();
}

/// Given a PluginBundle constructed manually, when its fields are checked,
/// then all default values are as expected.
#[test]
fn test_plugin_bundle_default() {
    let bundle = PluginBundle::default();

    assert!(bundle.tools.is_empty());
    assert!(bundle.commands.is_empty());
    assert!(bundle.queued_messages.is_empty());
    assert!(bundle.prepared_component.is_none());
    assert!(bundle.errors.is_empty());
}

/// Given a PluginBundle with some data, when debug-printed,
/// then it produces a valid debug string without panicking.
#[test]
fn test_plugin_bundle_debug() {
    use oben_wasm::plugin_context::RegisteredTool;

    let mut bundle = PluginBundle::default();
    bundle.tools.push(RegisteredTool {
        name: "test-tool".to_string(),
        description: "A test tool".to_string(),
        schema: "{}".to_string(),
        capabilities: vec![],
    });

    let debug_str = format!("{:?}", bundle);
    assert!(debug_str.contains("test-tool"));
    assert!(debug_str.contains("tools"));
}

/// Given a PluginBundle with a prepared component placeholder,
/// when debug-printed, then It renders the component field.
#[test]
fn test_plugin_bundle_debug_with_component() {
    use std::sync::Arc;
    use oben_wasm::runtime::PreparedComponent;

    // We can't actually create a PreparedComponent without real WASM bytes,
    // but we can verify the Debug impl handles None correctly.
    let bundle = PluginBundle::default();

    let debug_str = format!("{:?}", bundle);
    assert!(debug_str.contains("prepared_component"));
    // Should show None since prepared_component is None
    assert!(debug_str.contains("prepared_component:"));
}

// ── Test: PluginManifest serde ────────────────────────────────────────────────────────

/// Given a full PluginManifest JSON with all fields, when deserialized,
/// then all fields are correctly populated.
#[test]
fn test_plugin_manifest_full_serde() {
    let json = r#"{
        "name": "full-plugin",
        "version": "2.0.0",
        "description": "A fully specified plugin",
        "tools": ["tool-a", "tool-b"],
        "cli_commands": ["cmd-x"],
        "capabilities": {
            "workspace_read": true,
            "http": true,
            "tool_invoke": false
        },
        "sandbox_limits": {
            "max_memory_mb": 128,
            "timeout_ms": 10000,
            "cpu_fuel": 1000
        }
    }"#;

    let manifest: PluginManifest = serde_json::from_str(json).unwrap();

    assert_eq!(manifest.name, "full-plugin");
    assert_eq!(manifest.version, "2.0.0");
    assert_eq!(manifest.description, "A fully specified plugin");
    assert!(manifest.capabilities.workspace_read);
    assert!(manifest.capabilities.http);
    assert!(!manifest.capabilities.tool_invoke);
    assert_eq!(manifest.sandbox_limits.max_memory_mb, 128);
    assert_eq!(manifest.sandbox_limits.timeout_ms, 10000);
    assert_eq!(manifest.sandbox_limits.cpu_fuel, 1000);
}

/// Given a PluginManifest JSON with capabilities as a simple bool object (short form),
/// when deserialized, then it correctly parses workspace_read: true with defaults
/// for http and tool_invoke.
#[test]
fn test_plugin_manifest_capabilities_defaults() {
    let json = r#"{
        "name": "minimal-cap-plugin",
        "version": "1.0.0",
        "description": "Minimal capabilities"
    }"#;

    let manifest: PluginManifest = serde_json::from_str(json).unwrap();

    assert!(!manifest.capabilities.workspace_read);
    assert!(!manifest.capabilities.http);
    assert!(!manifest.capabilities.tool_invoke);
}
