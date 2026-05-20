# Context Compression Analysis

> Analysis of Hermes Agent's context compression system, and comparison with ObenAgent.

## 1. Trigger Condition

```
prompt_tokens >= context_length × threshold_percent (default 50%)
AND last 2 compressions saved >= 10% (anti-thrashing)
```

**Key**: does not wait for full context window — proactively compresses at **50% usage** to leave headroom.

## 2. Four-Stage Compression Algorithm

```
Messages: [System Prompt] [History 1..N] [Recent Messages]
           │              │                 │
           │              ▼                 │
           │     Phase 1: Tool Result Pruning
           │     (no LLM call — replaces large outputs with
           │      1-line summaries like:
           │      [terminal] ran `npm test` -> exit 0, 47 lines)
           │              │                 │
           │              ▼                 │
           │     Phase 2: Determine Boundaries
           │     - Head: System Prompt
           │       + first 3 non-system messages
           │     - Tail: last ~20K tokens
           │       (latest user message forced into tail)
           │              │                 │
           │              ▼                 │
           │     Phase 3: LLM Summarization
           │     Uses auxiliary model to generate
           │     structured summary
           │              │                 │
           │              ▼                 │
           │     Phase 4: Assemble
           │     [System Prompt + compression note]
           │     [Structured summary]
           │     [Recent messages]
           │
           │     Post: orphan tool_call cleanup
           │           strip historical image base64
```

### Detailed Breakdown

**Phase 1 — Tool Result Pruning** (cheap, no LLM call):
- Replace large tool outputs with 1-line summaries (e.g., `[read_file] read config.py from line 1 (3,400 chars)`)
- Deduplicate identical tool results (keep only newest full copy)
- Truncate large `tool_call` arguments in assistant messages outside protected tail
- Strip base64 image content from historical screenshots

**Phase 2 — Boundary Determination**:
- **Head protection**: system prompt + `protect_first_n` (default 3) non-system messages
- **Tail protection**: recent messages within `tail_token_budget` (~20K tokens, scales with model context)
- Always ensure the **last user message** is in the tail (prevents "active task" loss — fix for issue #10896)
- Never cut inside a tool_call → tool_result group

**Phase 3 — LLM Summarization**:
- Uses auxiliary model (cheaper/faster than main model)
- Redacts sensitive text (API keys, tokens, passwords) before sending to summarizer
- If auxiliary model fails, falls back to main model
- Structured template with 12 sections

**Phase 4 — Assembly**:
- Appends compression note to system prompt
- Inserts structured summary as standalone message
- Preserves tail messages verbatim
- Sanitizes orphaned tool_call/tool_result pairs
- Strips historical media (images, screenshots) from pre-tail messages

## 3. Structured Summary Template (12 Sections)

```
## Active Task         ← CRITICAL: user's most recent unfulfilled request
## Goal                ← Overall objective
## Constraints         ← User preferences, coding style, constraints
## Completed Actions   ← Numbered list of concrete actions taken
## Active State        ← Working dir, branch, modified files, test status
## In Progress         ← Work underway when compaction fired
## Blocked             ← Blockers, errors, issues not yet resolved
## Key Decisions       ← Important technical decisions and WHY
## Resolved Questions  ← Questions already answered (with answers)
## Pending User Asks   ← Questions/requests not yet answered
## Relevant Files      ← Files read, modified, or created
## Remaining Work      ← What remains to be done (as context, not instructions)
## Critical Context    ← Specific values, error messages, config details that would be lost
```

### Summary Generation

- **First compression**: summarizes from scratch using the template
- **Subsequent compressions**: iterative update — preserves existing info, adds new progress
- **Focus topic**: user can run `/compress <topic>` to prioritize preserving info about that topic
- **Budget**: proportional to content size (`content_tokens × 0.20`), scaled with model context window (5% cap, max 12K tokens)

## 4. Session Splitting (Core Impact)

The most significant effect: **each compression creates a new child session**.

```python
# In compress_context():
agent._session_db.end_session(old_session_id, "compression")
new_session_id = f"{datetime.now().strftime('%Y%m%d_%H%M%S')}_{uuid.uuid4().hex[:6]}"
agent.session_id = new_session_id
agent._session_db.create_session(
    session_id=new_session_id,
    parent_session_id=old_session_id,  # ← LINEAGE EDGE
)
# Title auto-numbering: "chat-xxx" → "chat-xxx (2)"
```

**Result**: compression forms a lineage tree:

```
Session tree:
chat-20260520-103902 (root, 95 messages)
  └─> chat-20260520-103902 (2)   (after 1st compression)
        └─> chat-20260520-103902 (3)  (after 2nd compression)
              └─> ...
```

This matches ObenAgent's `parent_session_id` schema in the `sessions` table.

## 5. Anti-Thrashing & Error Handling

| Mechanism | Description |
|---|---|
| **Anti-thrashing** | If last 2 compressions saved <10% each, skip compression. Suggests `/new` to start fresh. |
| **Summary cooldown** | After failure, pause summary attempts for 30-60s to avoid repeated API calls |
| **Aux model fallback** | If configured auxiliary model fails (404, timeout, JSON error), retry on main model |
| **Abort mode** | Configurable `compression.abort_on_summary_failure=true` returns messages unchanged (session frozen) vs. legacy fallback (static placeholder, drop middle) |

## 6. Preflight Compression

Before entering the main conversation loop, Hermes checks:

```python
if (len(messages) > protect_first_n + protect_last_n + 1):
    _preflight_tokens = estimate_request_tokens_rough(
        messages, system_prompt=..., tools=agent.tools)
    if _preflight_tokens >= threshold_tokens:
        # Compress before first LLM call
```

This handles cases where:
- User switches to a model with smaller context window
- A large existing session exceeds new model's threshold

## 7. ObenAgent vs Hermes Comparison

| Feature | Hermes (Python) | ObenAgent (Rust) |
|---|---|---|
| **Auto compression** | ✅ At 50% token threshold | ❌ Not implemented |
| **Session splitting** | ✅ Creates child session on compression | ✅ DB schema supports `parent_session_id` |
| **Structured summary** | ✅ 12-section template | ✅ Has `SummaryChunk` structure |
| **Iterative update** | ✅ Updates existing summary | ⚠️ Regenerates from scratch each time |
| **Tool result pruning** | ✅ Phase 1, no LLM call | ✅ Has `compact_session_messages` |
| **Tail token budget** | ✅ Protects ~20K recent tokens | ❌ No tail budget protection |
| **Focus compression** | ✅ `/compress <topic>` | ✅ Has `focus_topic` parameter |
| **Anti-thrashing** | ✅ Tracks compression effectiveness | ❌ Not implemented |
| **Summary failure fallback** | ✅ Falls back to main model | ❌ Not implemented |
| **Orphan tool_call cleanup** | ✅ `_sanitize_tool_pairs` | ❌ Not implemented |
| **Historical media strip** | ✅ `_strip_historical_media` | ❌ Not implemented |
| **Preflight compression** | ✅ On model switch | ❌ Not implemented |

## 8. ObenAgent Gap Analysis

### High Priority (Core Functionality)

1. **Auto-compression in ConversationLoop** — Add `should_compress()` check in the turn loop, call compression, and create child session via `parent_session_id`
2. **Tail token budget** — Protect recent N tokens from being summarized away during compression
3. **Iterative summary update** — Pass previous compaction summary to summarizer LLM as context for incremental updates

### Medium Priority (Quality Improvements)

4. **Summary failure fallback** — If auxiliary model fails, retry on main model instead of failing silently
5. **Anti-thrashing** — Track compression savings percentage; skip if ineffective for 2+ rounds
6. **Orphan tool_call cleanup** — Remove or create stub results for mismatched tool_call/tool_result pairs after compression

### Lower Priority (Edge Cases)

7. **Historical media stripping** — Replace old image content with text placeholders
8. **Preflight compression** — Check token budget on model switch before first API call
9. **Summary redaction** — Redact API keys/secrets from summary output before persisting

## 9. Implementation Notes for ObenAgent

### Integration Points

The `compact_session_messages` function in `oben-conversation/compression.rs` already handles:
- Tool result pruning
- FTS rebuild
- Summary chunk creation

Missing pieces to add:
1. **Boundary calculation** — Implement head/tail protection with token budget
2. **Session splitting** — After compression, create child session in `SessionManager` with `parent_session_id`
3. **Iterative update** — Read `summary_chunks` from previous compaction and pass to summarizer
4. **Auto-trigger** — Check token usage in `ConversationLoop` turn cycle

### Session Manager Changes

```rust
impl SessionManager {
    /// After compression: end old session, create child.
    pub fn split_after_compression(
        &mut self,
        old_id: &str,
        old_messages: &[Message],
    ) -> Result<String> {
        // 1. Mark old session as ended
        // 2. Save old messages to DB
        // 3. Create new session with parent_session_id = old_id
        // 4. Return new session ID
    }
}
```

### Configurable Parameters

Hermes exposes these via `config.yaml`:
- `context.threshold_percent` (default 0.50)
- `context.protect_first_n` (default 3)
- `context.protect_last_n` (default 20)
- `auxiliary.compression.model`
- `auxiliary.compression.context_length`
- `compression.abort_on_summary_failure`

ObenAgent should add similar config options to `AppConfig`.
