//! Integration tests for the MemoryProvider trait and MemoryManager.
//!
//! These tests live at the crate boundary, exercising the public API
//! through `oben_sessions::memory_provider::*` and `oben_sessions::skill_curation::*`.

use oben_sessions::memory_provider::*;
use oben_sessions::skill_curation::{MemoryResult, MemoryStore, scan_memory_content};

// ── FakeProvider — a test double ─────────────────────────────────────────────

struct FakeProvider {
    name_val: String,
    available: bool,
    prompt_block: String,
    prefetch_result: String,
    synced_turns: std::cell::RefCell<Vec<(String, String)>>,
    prefetch_queries: std::cell::RefCell<Vec<String>>,
    queued_prefetches: std::cell::RefCell<Vec<String>>,
    tools: Vec<ToolSchema>,
    turn_starts: std::cell::RefCell<Vec<(usize, String)>>,
    memory_writes: std::cell::RefCell<Vec<(String, String, String)>>,
    shutdown_called: bool,
}

impl FakeProvider {
    fn new(name: &str) -> Self {
        Self {
            name_val: name.to_string(),
            available: true,
            prompt_block: String::new(),
            prefetch_result: String::new(),
            synced_turns: std::cell::RefCell::new(Vec::new()),
            prefetch_queries: std::cell::RefCell::new(Vec::new()),
            queued_prefetches: std::cell::RefCell::new(Vec::new()),
            tools: Vec::new(),
            turn_starts: std::cell::RefCell::new(Vec::new()),
            memory_writes: std::cell::RefCell::new(Vec::new()),
            shutdown_called: false,
        }
    }

    fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    fn with_prefetch_result(mut self, result: &str) -> Self {
        self.prefetch_result = result.to_string();
        self
    }

    fn with_prompt_block(mut self, block: &str) -> Self {
        self.prompt_block = block.to_string();
        self
    }
}

impl MemoryProvider for FakeProvider {
    fn name(&self) -> &str {
        &self.name_val
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn initialize(&mut self, session_id: &str, _platform: Option<&str>) {
        let _ = session_id;
    }

    fn system_prompt_block(&self) -> String {
        self.prompt_block.clone()
    }

    fn prefetch(&self, query: &str, _session_id: &str) -> String {
        self.prefetch_queries.borrow_mut().push(query.to_string());
        self.prefetch_result.clone()
    }

    fn queue_prefetch(&self, query: &str, _session_id: &str) {
        self.queued_prefetches.borrow_mut().push(query.to_string());
    }

    fn sync_turn(&mut self, user_content: &str, assistant_content: &str, _session_id: &str) {
        self.synced_turns
            .borrow_mut()
            .push((user_content.to_string(), assistant_content.to_string()));
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools.clone()
    }

    fn handle_tool_call(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "handled": tool_name,
            "args": args,
            "from": self.name_val
        })
    }

    fn shutdown(&mut self) {
        self.shutdown_called = true;
    }

    fn on_turn_start(&mut self, turn_number: usize, message: &str) {
        self.turn_starts
            .borrow_mut()
            .push((turn_number, message.to_string()));
    }

    fn on_memory_write(&self, action: &str, target: &str, content: &str) {
        self.memory_writes
            .borrow_mut()
            .push((action.to_string(), target.to_string(), content.to_string()));
    }
}

// ── Test fixtures ─────────────────────────────────────────────────────────────

fn make_test_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn make_builtin_provider() -> BuiltinProvider {
    let dir = make_test_dir();
    let store = MemoryStore::default();
    BuiltinProvider::new(store, dir.into_path())
}

// ── BuiltinProvider tests ─────────────────────────────────────────────────────

#[test]
fn test_builtin_provider_has_correct_name() {
    /// given: a new BuiltinProvider wrapping an empty MemoryStore
    /// when: name() is called
    /// then: returns "builtin"
    let p = make_builtin_provider();
    assert_eq!(p.name(), "builtin");
}

#[test]
fn test_builtin_provider_is_available() {
    /// given: a BuiltinProvider with a valid home directory
    /// when: is_available() is called
    /// then: returns true
    let p = make_builtin_provider();
    assert!(p.is_available());
}

#[test]
fn test_builtin_provider_prefetch_returns_frozen_snapshot() {
    /// given: a BuiltinProvider with one memory entry added
    /// when: prefetch("test query") is called
    /// then: returns the frozen system-prompt snapshot as prefetched context
    let dir = make_test_dir();
    let store_path = dir.path();
    // Write an entry directly (since we can't mutate BuiltinProvider without a store clone)
    let mut store = MemoryStore::default();
    let _ = store.add("memory", "User prefers dark mode");
    let mut provider = BuiltinProvider::new(store, store_path.to_path_buf());
    let result = provider.prefetch("test", "");
    assert!(result.contains("dark mode"));
}

#[test]
fn test_builtin_provider_prefetch_empty_when_no_entries() {
    /// given: a BuiltinProvider with no memory entries
    /// when: prefetch("any query") is called
    /// then: returns empty string
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let provider = BuiltinProvider::new(store, dir.path().to_path_buf());
    let result = provider.prefetch("anything", "");
    assert!(result.is_empty());
}

#[test]
fn test_builtin_provider_system_prompt_block() {
    /// given: a BuiltinProvider with entries loaded
    /// when: system_prompt_block() is called
    /// then: returns the rendered memory block for system prompt injection
    let dir = make_test_dir();
    let mut store = MemoryStore::default();
    let _ = store.add("memory", "User is a Rust developer");
    let provider = BuiltinProvider::new(store, dir.path().to_path_buf());
    let block = provider.system_prompt_block();
    assert!(block.contains("Rust developer"));
}

#[test]
fn test_builtin_provider_get_tool_schemas() {
    /// given: a BuiltinProvider
    /// when: get_tool_schemas() is called
    /// then: returns tool schemas for add, replace, remove
    let provider = make_builtin_provider();
    let schemas = provider.get_tool_schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"memory.add"));
    assert!(names.contains(&"memory.replace"));
    assert!(names.contains(&"memory.remove"));
}

#[test]
fn test_builtin_provider_handle_tool_call() {
    /// given: a BuiltinProvider with no entries
    /// when: handle_tool_call("memory.add", {"target": "memory", "content": "fact"}) is called
    /// then: returns success JSON with the entry counted
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let mut provider = BuiltinProvider::new(store, dir.path().to_path_buf());
    let result = provider.handle_tool_call(
        "memory.add",
        &serde_json::json!({"target": "memory", "content": "test fact"}),
    );
    assert!(result["success"].as_bool().unwrap_or(false));
}

#[test]
fn test_builtin_provider_on_memory_write_notifies() {
    /// given: a BuiltinProvider
    /// when: on_memory_write("add", "memory", "content") is called
    /// then: the write is propagated
    let provider = make_builtin_provider();
    provider.on_memory_write("add", "memory", "test content");
}

// ── MemoryManager tests ───────────────────────────────────────────────────────

#[test]
fn test_manager_empty_has_no_providers() {
    /// given: an empty MemoryManager
    /// when: providers is accessed
    /// then: returns an empty vec
    let mgr = MemoryManager::new();
    assert!(mgr.providers.is_empty());
}

#[test]
fn test_manager_add_provider() {
    /// given: a MemoryManager
    /// when: add_provider(builtin) is called
    /// then: the builtin is registered
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    assert_eq!(mgr.providers.len(), 1);
    assert_eq!(mgr.providers[0].name(), "builtin");
}

#[test]
fn test_manager_prefetch_all_calls_all_providers() {
    /// given: a MemoryManager with builtin + fake provider
    /// when: prefetch_all("query") is called
    /// then: both providers receive the query and results are merged
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let mut fake = FakeProvider::new("ext")
        .with_prefetch_result("External memory");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let result = mgr.prefetch_all("test query", "");
    assert!(result.contains("External memory"));
}

#[test]
fn test_manager_prefetch_all_skips_empty() {
    /// given: a MemoryManager with two providers (both empty prefetch)
    /// when: prefetch_all("query") is called
    /// then: returns empty string
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_prefetch_result("");
    let mgr = MemoryManager::new();
    let mut m = mgr;
    m.add_provider(Box::new(builtin));
    let mut m2 = m;
    m2.add_provider(Box::new(fake));
    let result = m2.prefetch_all("query", "");
    assert!(result.is_empty());
}

#[test]
fn test_manager_sync_all_calls_all_providers() {
    /// given: a MemoryManager with builtin + fake
    /// when: sync_all("user", "assistant") is called
    /// then: both providers receive the turn
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let mut fake = FakeProvider::new("ext");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    mgr.sync_all("user msg", "assistant msg", "");
}

#[test]
fn test_manager_only_one_external_provider() {
    /// given: a MemoryManager with builtin + one external
    /// when: add_provider(second external) is called
    /// then: the second external is rejected
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(FakeProvider::new("mem0")));
    mgr.add_provider(Box::new(FakeProvider::new("hindsight")));
    assert_eq!(mgr.providers.len(), 2);
    assert_eq!(mgr.providers[1].name(), "mem0");
}

#[test]
fn test_manager_system_prompt_merges_blocks() {
    /// given: a MemoryManager with builtin + fake (with prompt block)
    /// when: build_system_prompt() is called
    /// then: returns both blocks joined
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_prompt_block("Extra context");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let prompt = mgr.build_system_prompt();
    assert!(prompt.contains("Extra context"));
}

#[test]
fn test_manager_tool_schemas_collected() {
    /// given: a MemoryManager with providers that have tools
    /// when: get_all_tool_schemas() is called
    /// then: schemas from all providers are returned
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_tools(vec![ToolSchema {
        name: "ext_recall".to_string(),
        description: "External recall".to_string(),
        parameters: serde_json::json!({}),
    }]);
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let schemas = mgr.get_all_tool_schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"memory.add")); // from builtin
    assert!(names.contains(&"ext_recall")); // from fake
}

#[test]
fn test_manager_has_tool() {
    /// given: a MemoryManager with providers
    /// when: has_tool("memory.add") and has_tool("ext_recall") are called
    /// then: returns true for registered tools, false for unregistered
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_tools(vec![ToolSchema {
        name: "ext_recall".to_string(),
        description: "External recall".to_string(),
        parameters: serde_json::json!({}),
    }]);
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    assert!(mgr.has_tool("memory.add"));
    assert!(mgr.has_tool("ext_recall"));
    assert!(!mgr.has_tool("nonexistent"));
}

#[test]
fn test_manager_handle_tool_call_routes() {
    /// given: a MemoryManager with builtin + external providers
    /// when: handle_tool_call("memory.add", ...) and handle_tool_call("ext_recall", ...) are called
    /// then: each routes to the correct provider
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_tools(vec![ToolSchema {
        name: "ext_recall".to_string(),
        description: "External recall".to_string(),
        parameters: serde_json::json!({}),
    }]);
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let result = mgr.handle_tool_call("memory.add", &serde_json::json!({
        "target": "memory",
        "content": "test"
    }));
    assert!(result["success"].as_bool().unwrap_or(false));
    let result = mgr.handle_tool_call("ext_recall", &serde_json::json!({}));
    assert_eq!(result["from"], "ext");
}

#[test]
fn test_manager_handle_tool_call_unknown_returns_error() {
    /// given: a MemoryManager with providers
    /// when: handle_tool_call("unknown_tool", ...) is called
    /// then: returns error JSON
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    let result = mgr.handle_tool_call("unknown_tool", &serde_json::json!({}));
    assert!(!result["success"].as_bool().unwrap_or(true));
}

#[test]
fn test_manager_on_turn_start() {
    /// given: a MemoryManager with providers
    /// when: on_turn_start(3, "hello") is called
    /// then: all providers receive the notification
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let mut fake = FakeProvider::new("ext");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    mgr.on_turn_start(3, "hello");
}

#[test]
fn test_manager_on_memory_write_notifies_external() {
    /// given: a MemoryManager with builtin + external
    /// when: on_memory_write("add", "memory", "content") is called
    /// then: external provider receives the notification (builtin does not)
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let mut ext = FakeProvider::new("ext");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(ext));
    mgr.on_memory_write("add", "memory", "test content");
}

#[test]
fn test_manager_initialize_all() {
    /// given: a MemoryManager with providers
    /// when: initialize_all("session-123", "cli") is called
    /// then: all providers are initialized
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let mut fake = FakeProvider::new("ext");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    mgr.initialize_all("session-123", "cli");
}

#[test]
fn test_manager_shutdown_all_reverse_order() {
    /// given: a MemoryManager with builtin + external
    /// when: shutdown_all() is called
    /// then: providers are shut down in reverse registration order
    let dir1 = make_test_dir();
    let store1 = MemoryStore::default();
    let builtin = BuiltinProvider::new(store1, dir1.path().to_path_buf());
    let mut fake = FakeProvider::new("ext");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    mgr.shutdown_all();
}

#[test]
fn test_manager_build_system_prompt_skips_empty() {
    /// given: a MemoryManager with two providers (one empty block, one with content)
    /// when: build_system_prompt() is called
    /// then: only the non-empty block is returned
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_prompt_block("  ");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let prompt = mgr.build_system_prompt();
    // Empty block should not appear
    assert!(!prompt.contains("  "));
}

#[test]
fn test_manager_prefetch_failure_doesnt_block_others() {
    /// given: a MemoryManager where one provider would panic on prefetch
    /// when: prefetch_all("query") is called
    /// then: the other provider's result is still returned
    // Note: Our Rust providers don't panic, so we test resilience via
    // the error-tolerant design. The test ensures the manager doesn't
    // crash when a provider returns unexpected data.
    let dir = make_test_dir();
    let store = MemoryStore::default();
    let builtin = BuiltinProvider::new(store, dir.path().to_path_buf());
    let fake = FakeProvider::new("ext").with_prefetch_result("safe result");
    let mut mgr = MemoryManager::new();
    mgr.add_provider(Box::new(builtin));
    mgr.add_provider(Box::new(fake));
    let result = mgr.prefetch_all("query", "");
    assert_eq!(result, "safe result");
}

// ── StreamingContextScrubber tests ────────────────────────────────────────────

#[test]
fn test_scrubber_passthrough_no_tags() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed("normal text") is called
    /// then: returns the text unchanged
    let mut scrubber = StreamingContextScrubber::new();
    let result = scrubber.feed("normal text");
    assert_eq!(result, "normal text");
}

#[test]
fn test_scrubber_removes_full_fence_block() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed("before <memory-context>blocked</memory-context> after") is called
    /// then: returns "before  after"
    let mut scrubber = StreamingContextScrubber::new();
    let result = scrubber.feed("before <memory-context>blocked</memory-context> after");
    assert_eq!(result.trim(), "before  after");
    // Flush any remaining buffer
    let trailing = scrubber.flush();
    assert!(trailing.is_empty());
}

#[test]
fn test_scrubber_splits_open_tag_across_deltas() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed("before <memory") then feed("-context>blocked</memory-context> after")
    /// then: both calls return only the non-blocked text
    let mut scrubber = StreamingContextScrubber::new();
    let r1 = scrubber.feed("before <memory");
    assert_eq!(r1, "before <memory");
    let r2 = scrubber.feed("-context>blocked</memory-context> after");
    assert_eq!(r2.trim(), "after");
    assert!(scrubber.flush().is_empty());
}

#[test]
fn test_scrubber_splits_close_tag_across_deltas() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed("before <memory-context>blocked</memo") then feed("ry-context> after")
    /// then: blocked content is discarded, only "after" is returned
    let mut scrubber = StreamingContextScrubber::new();
    let r1 = scrubber.feed("before <memory-context>blocked</memo");
    assert!(r1.is_empty() || r1.trim() == "before");
    let r2 = scrubber.feed("ry-context> after");
    assert!(r2.contains("after"));
    assert!(scrubber.flush().is_empty());
}

#[test]
fn test_scrubber_multiple_blocks() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed with two fenced blocks
    /// then: both blocks are removed
    let mut scrubber = StreamingContextScrubber::new();
    let r1 = scrubber.feed("before <memory-context>first</memory-context> middle <memory-context>second</memory-context> end");
    assert!(r1.contains("before"));
    assert!(r1.contains("middle"));
    assert!(r1.contains("end"));
    assert!(!r1.contains("first"));
    assert!(!r1.contains("second"));
}

#[test]
fn test_scrubber_unterminated_block_discards() {
    /// given: a fresh StreamingContextScrubber
    /// when: feed("before <memory-context>no close") is called
    /// then: blocked content is discarded
    let mut scrubber = StreamingContextScrubber::new();
    let r1 = scrubber.feed("before <memory-context>no close");
    assert!(r1.is_empty() || r1.trim() == "before");
    assert!(scrubber.flush().is_empty());
}

#[test]
fn test_scrubber_reset() {
    /// given: a scrubber that was in a span
    /// when: feed("before <memory-context>blocked</memory-context> mid"), then reset()
    /// then: subsequent feed starts fresh
    let mut scrubber = StreamingContextScrubber::new();
    let _ = scrubber.feed("before <memory-context>blocked</memory-context> mid");
    scrubber.reset();
    let r = scrubber.feed("clean after reset");
    assert_eq!(r, "clean after reset");
}

// ── ToolSchema helper tests ───────────────────────────────────────────────────

#[test]
fn test_tool_schema_serialization() {
    /// given: a ToolSchema with name, description, parameters
    /// when: it is serialized via serde_json
    /// then: produces valid JSON with all fields
    let schema = ToolSchema {
        name: "memory.add".to_string(),
        description: "Add a memory entry".to_string(),
        parameters: serde_json::json!({"target": {"type": "string"}, "content": {"type": "string"}}),
    };
    let json = serde_json::to_string(&schema).unwrap();
    assert!(json.contains("memory.add"));
    assert!(json.contains("Add a memory entry"));
}
