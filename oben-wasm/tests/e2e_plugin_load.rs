//! End-to-end tests for WASM plugin discovery.
//!
//! These tests exercise the `PluginLoader::discover_plugins` and
//! `PluginLoader::discover_only` methods against real filesystem directories,
//! verifying that file extension filtering and empty-directory behavior match
//! the expected contract.

use std::path::PathBuf;

use tempfile::TempDir;

use oben_wasm::PluginLoader;

/// Given an empty directory, when discover_plugins is called,
/// then no plugins are discovered (empty result).
///
/// This verifies that a completely empty plugins directory does not
/// cause errors — it returns an empty Vec, which is the expected
/// behavior for a fresh deploy where no .wasm files exist yet.
#[tokio::test]
async fn test_discover_plugins_empty_dir() {
    let tmp = TempDir::new().expect("should create temp directory");
    let plugins_dir = tmp.path().to_path_buf();

    let discovered = PluginLoader::discover_only(&plugins_dir).expect("discover_only should succeed");

    assert!(
        discovered.is_empty(),
        "Empty directory should yield no discovered plugins, got {}",
        discovered.len()
    );
}

/// Given a directory containing only non-.wasm files, when discover_plugins
/// is called, then no plugins are discovered.
///
/// This verifies that the loader filters strictly on the `.wasm` extension.
/// Non-WASM files such as .txt, .json, or .md must not appear in results.
#[tokio::test]
async fn test_discover_plugins_no_wasm_files() {
    let tmp = TempDir::new().expect("should create temp directory");

    // Create several non-WASM files in the directory.
    std::fs::write(tmp.path().join("readme.txt"), "hello").unwrap();
    std::fs::write(tmp.path().join("config.json"), "{\"key\":\"value\"}").unwrap();
    std::fs::write(tmp.path().join("notes.md"), "# Notes").unwrap();
    std::fs::create_dir_all(tmp.path().join("subdir")).unwrap();

    let discovered = PluginLoader::discover_only(tmp.path()).expect("discover_only should succeed");

    assert!(
        discovered.is_empty(),
        "Directory with only non-.wasm files should yield no plugins, got {}",
        discovered.len()
    );
}

/// Given a directory with a .platform.json sidecar beside a non-.wasm file,
/// when discover_plugins is called, then still no plugins are discovered.
///
/// This ensures the .wasm extension check happens before any .platform.json
/// inspection, preventing false positives from metadata-only entries.
#[tokio::test]
async fn test_discover_plugins_platform_json_without_wasm() {
    let tmp = TempDir::new().expect("should create temp directory");

    // A valid .platform.json without a corresponding .wasm file.
    let json = r#"{"name":"test-plugin","version":"1.0.0"}"#;
    std::fs::write(tmp.path().join("test-plugin.json"), json).unwrap();

    let discovered = PluginLoader::discover_only(tmp.path()).expect("discover_only should succeed");

    assert!(
        discovered.is_empty(),
        ".platform.json without .wasm should not be discovered as a plugin, got {}",
        discovered.len()
    );
}

/// Given a non-existent directory path, when discover_plugins is called,
/// then no plugins are discovered without error.
///
/// This verifies the defensive early-exit when `plugins_dir.exists()` is false.
#[tokio::test]
async fn test_discover_plugins_nonexistent_dir() {
    let nonexistent = PathBuf::from("/tmp/nonexistent_oben_wasm_dir_42xyz");
    let discovered = PluginLoader::discover_only(&nonexistent).expect("discover_only should not error for nonexistent dir");

    assert!(
        discovered.is_empty(),
        "Nonexistent directory should yield no plugins, got {}",
        discovered.len()
    );
}

/// Given a directory with a .wasm file AND its .platform.json sidecar,
/// when discover_plugins is called, then the plugin is discovered with the
/// name from the platform JSON.
#[tokio::test]
async fn test_discover_plugins_with_platform_json_sidecar() {
    let tmp = TempDir::new().expect("should create temp directory");

    // discover_only scans subdirectories for manifest files, so wrap
    // the .wasm + .platform.json inside a plugin subdirectory.
    let plugin_dir = tmp.path().join("my-plugin");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.wasm"), [] as [u8; 0]).unwrap();
    std::fs::write(
        plugin_dir.join(".platform.json"),
        r#"{"name":"my-platform-plugin","version":"0.1.0","description":"A test plugin"}"#,
    )
    .unwrap();

    let discovered = PluginLoader::discover_only(tmp.path()).expect("discover_only should succeed");

    assert_eq!(discovered.len(), 1, "Should discover exactly one plugin");
    assert_eq!(
        discovered[0].manifest.name, "my-platform-plugin",
        "Plugin name should come from .platform.json"
    );
}
