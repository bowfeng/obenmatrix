//! Pluggable memory provider trait, built-in provider, and manager.
//!
//! Architecture:
//!
//! ```
//! MemoryProvider (trait)
//! ├── BuiltinProvider { MemoryStore }  — always present
//! └── ExternalProvider (pluggable)      — registered via config
//!
//! MemoryManager
//! ├── add_provider()                    — builtin + 1 external max
//! ├── prefetch_all(query)               — fan-out to all providers
//! ├── sync_all(user, assistant)         — fan-out to all providers
//! ├── build_system_prompt()             — merge prompt blocks
//! ├── get_all_tool_schemas() / has_tool() / handle_tool_call()
//! ├── on_turn_start, on_session_end, on_session_switch, on_pre_compress
//! ├── on_memory_write                   — bridge to external providers
//! └── shutdown_all() / initialize_all()
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── ToolSchema ────────────────────────────────────────────────────────────────

/// A tool schema exposed by a memory provider (OpenAI function-calling format).
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── MemoryProvider trait ─────────────────────────────────────────────────────

/// Abstract interface for memory providers.
///
/// Providers give the agent persistent recall across sessions.
/// The built-in provider is named `"builtin"` and always wraps `MemoryStore`.
pub trait MemoryProvider {
    /// Short identifier (e.g. `"builtin"`, `"honcho"`, `"hindsight"`).
    fn name(&self) -> &str;

    /// Whether the provider is configured and ready.
    fn is_available(&self) -> bool;

    /// Initialize for a session.
    fn initialize(&mut self, session_id: &str, platform: Option<&str>);

    /// Static text to include in the system prompt.
    fn system_prompt_block(&self) -> String;

    /// Recall relevant context for the upcoming turn.
    fn prefetch(&self, query: &str, session_id: &str) -> String;

    /// Queue a background prefetch for the NEXT turn.
    fn queue_prefetch(&self, query: &str, session_id: &str) {
        let _ = (query, session_id);
    }

    /// Persist a completed turn.
    fn sync_turn(&mut self, user_content: &str, assistant_content: &str, session_id: &str);

    /// Tool schemas this provider exposes.
    fn get_tool_schemas(&self) -> Vec<ToolSchema>;

    /// Handle a tool call. Must return a JSON string.
    fn handle_tool_call(&mut self, tool_name: &str, args: &serde_json::Value) -> serde_json::Value;

    /// Clean shutdown.
    fn shutdown(&mut self) {}

    /// Optional: per-turn tick with runtime context.
    fn on_turn_start(&mut self, _turn_number: usize, _message: &str) {}

    /// Optional: end-of-session extraction.
    fn on_session_end(&mut self, _messages: &[serde_json::Value]) {}

    /// Optional: session_id rotation.
    fn on_session_switch(&mut self, _new_session_id: &str, _parent_session_id: &str, _reset: bool) {}

    /// Optional: extract insights before compression.
    fn on_pre_compress(&self, _messages: &[serde_json::Value]) -> String {
        String::new()
    }

    /// Called when the built-in memory tool writes.
    fn on_memory_write(&self, _action: &str, _target: &str, _content: &str) {}
}

// ── BuiltinProvider ───────────────────────────────────────────────────────────

/// Wraps `MemoryStore` as a `MemoryProvider`.
///
/// Memory entries are frozen at load time to preserve the prefix cache.
/// `prefetch()` and `system_prompt_block()` return the frozen snapshot.
pub struct BuiltinProvider {
    store: crate::skill_curation::MemoryStore,
    store_path: std::path::PathBuf,
}

impl BuiltinProvider {
    pub fn new(store: crate::skill_curation::MemoryStore, store_path: std::path::PathBuf) -> Self {
        Self { store, store_path }
    }
}

impl BuiltinProvider {
    fn render_block(&self, target: &str, entries: &[String]) -> String {
        if entries.is_empty() {
            return String::new();
        }
        let content = entries.join("\n§\n");
        let limit = if target == "user" { 1375usize } else { 2200usize };
        let current = content.len();
        let pct = (current as f64 / limit as f64 * 100.0).min(100.0) as usize;
        let header = if target == "user" {
            format!(
                "USER PROFILE (who the user is) [{}% — {}/{} chars]",
                pct, current, limit
            )
        } else {
            format!(
                "MEMORY (your personal notes) [{}% — {}/{} chars]",
                pct, current, limit
            )
        };
        format!("{}\n{}\n{}\n{}", "═".repeat(46), header, "═".repeat(46), content)
    }
}

impl MemoryProvider for BuiltinProvider {
    fn name(&self) -> &str {
        "builtin"
    }

    fn is_available(&self) -> bool {
        std::fs::create_dir_all(&self.store_path).is_ok()
    }

    fn initialize(&mut self, _session_id: &str, _platform: Option<&str>) {
        // Builtin loads from disk on construction; no extra init needed.
    }

    fn system_prompt_block(&self) -> String {
        let mem = self.render_block("memory", self.store.get_entries("memory"));
        let usr = self.render_block("user", self.store.get_entries("user"));
        if mem.is_empty() && usr.is_empty() {
            String::new()
        } else {
            format!("{}\n\n{}", mem, usr)
        }
    }

    fn prefetch(&self, _query: &str, _session_id: &str) -> String {
        // Return the current entries as prefetched context.
        // Uses the frozen system-prompt snapshot for prefix-cache stability.
        let mem = self.store.format_for_system_prompt("memory");
        let usr = self.store.format_for_system_prompt("user");
        let mut parts = Vec::new();
        if let Some(s) = mem {
            if !s.is_empty() {
                parts.push(s.to_string());
            }
        }
        if let Some(s) = usr {
            if !s.is_empty() {
                parts.push(s.to_string());
            }
        }
        // If no frozen snapshot (store loaded fresh), render directly
        if parts.is_empty() {
            let mem = self.render_block("memory", self.store.get_entries("memory"));
            let usr = self.render_block("user", self.store.get_entries("user"));
            if !mem.is_empty() {
                parts.push(mem);
            }
            if !usr.is_empty() {
                parts.push(usr);
            }
        }
        parts.join("\n\n")
    }

    fn sync_turn(&mut self, _user_content: &str, _assistant_content: &str, _session_id: &str) {
        // Builtin doesn't need per-turn persistence — writes are immediate via add/replace/remove.
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                name: "memory.add".into(),
                description: "Add a memory entry. When the user corrects you, shares a preference, or says 'remember this'."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "enum": ["memory", "user"] },
                        "content": { "type": "string", "description": "The memory entry to add." }
                    },
                    "required": ["target", "content"]
                }),
            },
            ToolSchema {
                name: "memory.replace".into(),
                description: "Replace an existing memory entry. Provide the old text to find and the new content."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "enum": ["memory", "user"] },
                        "old_text": { "type": "string", "description": "Partial text to match the existing entry." },
                        "new_content": { "type": "string", "description": "The replacement content." }
                    },
                    "required": ["target", "old_text", "new_content"]
                }),
            },
            ToolSchema {
                name: "memory.remove".into(),
                description: "Remove a memory entry. Provide partial text that matches the entry to delete."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "enum": ["memory", "user"] },
                        "old_text": { "type": "string", "description": "Partial text to match the entry to remove." }
                    },
                    "required": ["target", "old_text"]
                }),
            },
        ]
    }

    fn handle_tool_call(&mut self, tool_name: &str, args: &serde_json::Value) -> serde_json::Value {
        match tool_name {
            "memory.add" => {
                let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("memory");
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if content.is_empty() {
                    return serde_json::json!({ "success": false, "error": "Content cannot be empty." });
                }
                let scan_err = crate::skill_curation::scan_memory_content(content);
                if let Some(err) = scan_err {
                    return serde_json::json!({ "success": false, "error": err });
                }
                let result = self.store.add(target, content);
                match result {
                    crate::skill_curation::MemoryResult::Success { success, target, usage_percent: _, usage_text, entry_count, message } => {
                        serde_json::json!({
                            "success": success,
                            "target": target,
                            "usage": usage_text,
                            "entry_count": entry_count,
                            "message": message.unwrap_or_default()
                        })
                    }
                    crate::skill_curation::MemoryResult::Error { error } => {
                        serde_json::json!({ "success": false, "error": error })
                    }
                    crate::skill_curation::MemoryResult::Exceeded { current, limit, new_content_len: _, entries, error } => {
                        serde_json::json!({
                            "success": false,
                            "error": error,
                            "current_entries": entries,
                            "usage": format!("{}/{}", current, limit)
                        })
                    }
                    crate::skill_curation::MemoryResult::Ambiguous { error, matches } => {
                        serde_json::json!({ "success": false, "error": error, "matches": matches })
                    }
                }
            }
            "memory.replace" => {
                let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("memory");
                let old_text = args.get("old_text").and_then(|v| v.as_str()).unwrap_or("");
                let new_content = args.get("new_content").and_then(|v| v.as_str()).unwrap_or("");
                let result = self.store.replace(target, old_text, new_content);
                match result {
                    crate::skill_curation::MemoryResult::Success { success, target, usage_percent: _, usage_text, entry_count, message } => {
                        serde_json::json!({
                            "success": success,
                            "target": target,
                            "usage": usage_text,
                            "entry_count": entry_count,
                            "message": message.unwrap_or_default()
                        })
                    }
                    crate::skill_curation::MemoryResult::Error { error } => {
                        serde_json::json!({ "success": false, "error": error })
                    }
                    crate::skill_curation::MemoryResult::Exceeded { current, limit, new_content_len: _, entries, error } => {
                        serde_json::json!({
                            "success": false,
                            "error": error,
                            "current_entries": entries,
                            "usage": format!("{}/{}", current, limit)
                        })
                    }
                    crate::skill_curation::MemoryResult::Ambiguous { error, matches } => {
                        serde_json::json!({ "success": false, "error": error, "matches": matches })
                    }
                }
            }
            "memory.remove" => {
                let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("memory");
                let old_text = args.get("old_text").and_then(|v| v.as_str()).unwrap_or("");
                let result = self.store.remove(target, old_text);
                match result {
                    crate::skill_curation::MemoryResult::Success { success, target, usage_percent: _, usage_text, entry_count, message } => {
                        serde_json::json!({
                            "success": success,
                            "target": target,
                            "usage": usage_text,
                            "entry_count": entry_count,
                            "message": message.unwrap_or_default()
                        })
                    }
                    crate::skill_curation::MemoryResult::Error { error } => {
                        serde_json::json!({ "success": false, "error": error })
                    }
                    crate::skill_curation::MemoryResult::Exceeded { current, limit, new_content_len: _, entries, error } => {
                        serde_json::json!({
                            "success": false,
                            "error": error,
                            "current_entries": entries,
                            "usage": format!("{}/{}", current, limit)
                        })
                    }
                    crate::skill_curation::MemoryResult::Ambiguous { error, matches } => {
                        serde_json::json!({ "success": false, "error": error, "matches": matches })
                    }
                }
            }
            _ => {
                serde_json::json!({
                    "success": false,
                    "error": format!("Unknown tool: {}", tool_name)
                })
            }
        }
    }

    fn on_memory_write(&self, _action: &str, _target: &str, _content: &str) {
        // Builtin IS the source of writes — no self-notification needed.
    }
}

// ── MemoryManager ─────────────────────────────────────────────────────────────

/// Orchestrates memory providers. Always accepts builtin + at most one external.
pub struct MemoryManager {
    pub providers: Vec<Box<dyn MemoryProvider>>,
    tool_to_provider: std::collections::HashMap<String, usize>,
    has_external: bool,
}

impl MemoryManager {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            tool_to_provider: HashMap::new(),
            has_external: false,
        }
    }

    /// Register a provider. Only one non-builtin provider is allowed.
    pub fn add_provider(&mut self, provider: Box<dyn MemoryProvider>) {
        let is_builtin = provider.name() == "builtin";
        if !is_builtin && self.has_external {
            tracing::warn!(
                "Rejected memory provider '{}' — external provider already registered",
                provider.name()
            );
            return;
        }
        if !is_builtin {
            self.has_external = true;
        }
        let idx = self.providers.len();
        self.providers.push(provider);
        // Index tool names → provider index
        if let Some(p) = self.providers.last() {
            for schema in p.get_tool_schemas() {
                let name = schema.name;
                if !self.tool_to_provider.contains_key(&name) {
                    self.tool_to_provider.insert(name, idx);
                }
            }
        }
        tracing::info!(
            "Memory provider '{}' registered ({} tools)",
            self.providers[idx].name(),
            self.providers[idx].get_tool_schemas().len()
        );
    }

    pub fn build_system_prompt(&self) -> String {
        let blocks: Vec<String> = self
            .providers
            .iter()
            .filter_map(|p| {
                let block = p.system_prompt_block();
                if block.trim().is_empty() { None } else { Some(block) }
            })
            .collect();
        blocks.join("\n\n")
    }

    pub fn prefetch_all(&self, query: &str, session_id: &str) -> String {
        let parts: Vec<String> = self
            .providers
            .iter()
            .filter_map(|p| {
                let result = p.prefetch(query, session_id);
                if result.trim().is_empty() { None } else { Some(result) }
            })
            .collect();
        parts.join("\n\n")
    }

    pub fn queue_prefetch_all(&self, query: &str, session_id: &str) {
        for p in &self.providers {
            p.queue_prefetch(query, session_id);
        }
    }

    pub fn sync_all(&mut self, user_content: &str, assistant_content: &str, session_id: &str) {
        for p in &mut self.providers {
            p.sync_turn(user_content, assistant_content, session_id);
        }
    }

    pub fn get_all_tool_schemas(&self) -> Vec<ToolSchema> {
        let mut schemas = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for p in &self.providers {
            for schema in p.get_tool_schemas() {
                if seen.insert(schema.name.clone()) {
                    schemas.push(schema);
                }
            }
        }
        schemas
    }

    pub fn get_all_tool_names(&self) -> std::collections::HashSet<String> {
        self.tool_to_provider.keys().cloned().collect()
    }

    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tool_to_provider.contains_key(tool_name)
    }

    pub fn handle_tool_call(&mut self, tool_name: &str, args: &serde_json::Value) -> serde_json::Value {
        match self.tool_to_provider.get(tool_name) {
            Some(&idx) => self.providers[idx].handle_tool_call(tool_name, args),
            None => serde_json::json!({
                "success": false,
                "error": format!("No memory provider handles tool '{}'", tool_name)
            }),
        }
    }

    pub fn on_turn_start(&mut self, turn_number: usize, message: &str) {
        for p in &mut self.providers {
            p.on_turn_start(turn_number, message);
        }
    }

    pub fn on_session_end(&mut self, messages: &[serde_json::Value]) {
        for p in &mut self.providers {
            p.on_session_end(messages);
        }
    }

    pub fn on_session_switch(&mut self, new_session_id: &str, parent_session_id: &str, reset: bool) {
        for p in &mut self.providers {
            p.on_session_switch(new_session_id, parent_session_id, reset);
        }
    }

    pub fn on_pre_compress(&self, messages: &[serde_json::Value]) -> String {
        let parts: Vec<String> = self
            .providers
            .iter()
            .filter_map(|p| {
                let result = p.on_pre_compress(messages);
                if result.trim().is_empty() { None } else { Some(result) }
            })
            .collect();
        parts.join("\n\n")
    }

    pub fn on_memory_write(&self, action: &str, target: &str, content: &str) {
        for p in &self.providers {
            if p.name() != "builtin" {
                p.on_memory_write(action, target, content);
            }
        }
    }

    pub fn shutdown_all(&mut self) {
        // Reverse order for clean teardown
        for p in self.providers.iter_mut().rev() {
            p.shutdown();
        }
    }

    pub fn initialize_all(&mut self, session_id: &str, platform: &str) {
        for p in &mut self.providers {
            p.initialize(session_id, Some(platform));
        }
    }
}

// ── StreamingContextScrubber ──────────────────────────────────────────────────

/// Stateful scrubber for streaming text that may contain split
/// `<memory-context>` fence tags across multiple stream deltas.
///
/// Holds back partial-tag fragments at chunk boundaries and discards
/// everything inside a span (safer: leaking partial memory context
/// is worse than a truncated answer).
pub struct StreamingContextScrubber {
    in_span: bool,
    buf: String,
    at_block_boundary: bool,
}

impl StreamingContextScrubber {
    pub fn new() -> Self {
        Self {
            in_span: false,
            buf: String::new(),
            at_block_boundary: true,
        }
    }

    /// Return the visible portion of `text` after scrubbing.
    ///
    /// Key design: partial tags at chunk boundaries are BOTH returned
    /// (so the user sees them) AND held back internally (so they can
    /// be concatenated with the next chunk to detect complete tags).
    pub fn feed(&mut self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }

        let mut buf = std::mem::take(&mut self.buf) + text;
        let mut out = String::new();

        while !buf.is_empty() {
            if self.in_span {
                // Inside a <memory-context> block — discard until </memory-context>
                if let Some(idx) = buf.to_lowercase().find("</memory-context>") {
                    buf = buf[idx + 17..].to_string();
                    self.in_span = false;
                } else {
                    // No complete close tag — check for partial close suffix
                    let partial = Self::max_partial_close_suffix(&buf);
                    if partial > 0 {
                        // We're in a span and might be about to exit.
                        // Can't safely emit any new text since we don't know
                        // where the partial close tag ends and content begins.
                        // Hold everything back for the next call.
                        self.buf = buf;
                        break;
                    }
                    // No partial either — just hold back (no close tag at all)
                    self.buf = buf;
                    break;
                }
            } else {
                // Not in a span — look for <memory-context>
                if let Some(idx) = buf.to_lowercase().find("<memory-context>") {
                    out.push_str(&buf[..idx]);
                    buf = buf[idx + 16..].to_string();
                    self.in_span = true;
                } else {
                    // No complete open tag — check for partial open suffix
                    let partial = Self::max_partial_open_suffix(&buf);
                    if partial > 0 {
                        // Return everything (including the partial tag)
                        // but hold back the partial internally for concatenation
                        out.push_str(&buf);
                        self.buf = buf[buf.len() - partial..].to_string();
                        break;
                    }
                    // No tag at all — emit everything
                    out.push_str(&buf);
                    break;
                }
            }
        }

        let last = out.chars().last();
        self.at_block_boundary = last == Some('\n') || out.is_empty();

        out
    }

    /// Emit any held-back buffer at end-of-stream.
    pub fn flush(&mut self) -> String {
        if self.in_span {
            self.buf.clear();
            self.in_span = false;
            String::new()
        } else {
            let tail = std::mem::take(&mut self.buf);
            self.at_block_boundary = tail.is_empty() || tail.ends_with('\n');
            tail
        }
    }

    /// Reset the scrubber state (for new top-level responses).
    pub fn reset(&mut self) {
        self.in_span = false;
        self.buf.clear();
        self.at_block_boundary = true;
    }

    /// Return the length of the longest buf-suffix that is a tag-prefix (partial or complete).
    fn max_partial_suffix(buf: &str, tag: &str) -> usize {
        let tag_lower = tag.to_lowercase();
        let buf_lower = buf.to_lowercase();
        let max_check = buf_lower.len().min(tag_lower.len());
        for i in (1..=max_check).rev() {
            if tag_lower.starts_with(&buf_lower[buf_lower.len() - i..]) {
                return i;
            }
        }
        0
    }

    /// Return the length of the longest buf-suffix that is an open-tag prefix.
    /// e.g. for buffer "text <memor", returns 7 ("<memor" is a prefix).
    fn max_partial_open_suffix(buf: &str) -> usize {
        let open_tag = "<memory-context>";
        let tag_lower = open_tag.to_lowercase();
        let buf_lower = buf.to_lowercase();
        for suffix_len in (1..=buf_lower.len().min(open_tag.len())).rev() {
            let suffix = &buf_lower[buf_lower.len() - suffix_len..];
            if tag_lower.starts_with(suffix) {
                return suffix_len;
            }
        }
        0
    }

    /// Return the length of the longest buf-suffix that is a close-tag prefix.
    /// e.g. for buffer "text </memo", returns 6 ("</memo" is a prefix).
    fn max_partial_close_suffix(buf: &str) -> usize {
        let close_tag = "</memory-context>";
        let tag_lower = close_tag.to_lowercase();
        let buf_lower = buf.to_lowercase();
        for suffix_len in (1..=buf_lower.len().min(close_tag.len())).rev() {
            let suffix = &buf_lower[buf_lower.len() - suffix_len..];
            if tag_lower.starts_with(suffix) {
                return suffix_len;
            }
        }
        0
    }
}

// ── Context fencing helpers ───────────────────────────────────────────────────

fn sanitize_context(text: &str) -> String {
    // Strip fence tags and system notes
    let text = match regex::Regex::new(r"(?i)<\s*memory-context\s*>[\s\S]*?</\s*memory-context\s*>") {
        Ok(re) => re.replace_all(text, "").into_owned(),
        Err(_) => text.to_string(),
    };
    let text = match regex::Regex::new(r"(?i)</?\s*memory-context\s*>") {
        Ok(re) => re.replace_all(&text, "").into_owned(),
        Err(_) => text,
    };
    text
}

pub fn build_memory_context_block(raw_context: &str) -> String {
    if raw_context.trim().is_empty() {
        return String::new();
    }
    let clean = sanitize_context(raw_context);
    format!(
        "<memory-context>\n\
         [System note: The following is recalled memory context, NOT new user input. \
         Treat as authoritative reference data — this is the agent's persistent memory and should inform all responses.]\n\n\
         {}\n\
         </memory-context>",
        clean
    )
}
