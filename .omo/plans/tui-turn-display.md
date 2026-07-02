# tui-turn-display - Work Plan

## TL;DR (For humans)

**What you'll get:** The TUI chat panel will display AI reasoning/thinking, which tools were called and their results as separate visual blocks during each turn — matching how Hermes-Agent structures turn output. Reasoning appears in muted text below the assistant response start, tool calls show as dim indicators, and tool results render as indented styled blocks.

**Why this approach:** The TUI already collects all turn data in `TurnState` (reasoning text, tool starts/completions, streaming text) but only flushes streaming_text to the message display. The fix is to leverage the existing `MessageRenderEntry` infrastructure (which already supports reasoning splits and tool-result blocks) and make the flush produce the proper multi-block output. A critical timing bug in `on_completed()` clears tool data before the flush can read it — fixed by the same pattern the codebase already uses for streaming_text.

**What it will NOT do:** Persist reasoning to session messages, expand/collapse individual tool calls, change the message data model, or modify streaming_text behavior. The sessions panel display is unchanged.

**Effort:** Medium
**Risk:** Low — all changes are in the TUI rendering layer and one TurnState method; the existing MessageRenderEntry infrastructure handles rendering.
**Decisions to sanity-check:** (1) completed_tools preserved across on_completed() (2) reasoning deferred to session persistence (3) tool result blocks use existing BlockType::ToolResult.

Your next move: approve to start, or request a high-accuracy review first. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, Low risk — 4 todos across 3 waves. Fix TurnState timing, multi-block chat flush (reasoning+response+tools), stream reasoning display.

## Scope
### Must have
1. TurnState preserves `completed_tools` and `reasoning_text` across `on_completed()` (same pattern as streaming_text)
2. ChatPanel flush produces ordered entries: reasoning block → response + tool call indicators → tool result blocks
3. Tool result entries use `is_tool_result: true` with colored error/success indicator
4. Live turn render shows reasoning_text (DIM) alongside streaming_text
5. No layout regression — existing behavior unchanged when flush has no turn data

### Must NOT have
- Changes to Message struct, ToolCall struct, or session persistence
- Changes to streaming_text handling (separate improvement)
- Per-tool-detail expansion / collapsible tool blocks
- Reasoning persistence to session messages
- Changes to the sessions panel or session message display

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after (no existing TUI tests; instrument with tracing, verify compilation passes)
- Evidence: `cargo check -p oben-tui && cargo check -p oben-agent` in each wave
- QA: manual TUI run observing turn completion with reasoning + tools

## Execution strategy
### Parallel execution waves
- **Wave 1** (core fix): 1 todo — TurnState timing fix
- **Wave 2** (flush): 1 todo — ChatPanel multi-block flush (reasoning + response + tool entries + styling)
- **Wave 3** (stream reasoning): 1 todo — Live reasoning text display
- **Wave 4** (verify): 1 todo — compile check + test run

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T1. TurnState timing fix | — | T2 | — |
| T2. Multi-block flush | T1 | T5 | — |
| T3. Stream reasoning | T1 | T5 | — |
| T4. Compile + test | T2, T3 | — | — |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

- [ ] 1. Preserve completed_tools in TurnState::on_completed()
  What to do / Must NOT do: In `oben-agent/src/hooks/kind.rs:174-183`, CHANGE the `on_completed` method: replace `self.completed_tools.clear();` with a no-op (remove that line entirely). reasoning_text is already preserved (it's not cleared in `on_completed()` — only in `on_turn_start()` at line 112). This mirrors the `streaming_text` pattern: the data survives until `on_turn_start()` clears it and the TUI flush reads it in the next draw cycle.
  Critical: The flush at `chat.rs:203-232` fires when it sees `prev=Streaming → current=Completed`. It reads `completed_tools` in that flush — if `on_completed()` already cleared them, the flush gets empty data. The `streaming_text` fix (kind.rs:176, no clear) is the documented precedent — the comment at lines 180-183 explicitly says: *"Don't clear streaming_text here — the TUI flushes it to message_entries in the next draw."* Apply the same reasoning to `completed_tools`.
  Parallelization: Wave 1 | Blocked by: none | Blocks: T2
  References: `oben-agent/src/hooks/kind.rs:174-183` (TurnState::on_completed — CHANGE), `kind.rs:106-115` (on_turn_start — clears completed_tools for NEXT turn), `kind.rs:112` (streaming_text NOT cleared — precedent), `oben-tui/src/panels/chat.rs:191-232` (flush path that reads completed_tools)
  Acceptance criteria (agent-executable):
  1. `cargo check -p oben-agent` passes
  2. `on_completed` no longer calls `self.completed_tools.clear()`
  3. `on_turn_start` still clears `self.completed_tools` (verified in kind.rs:109)
  4. Diff shows exactly one line removed from kind.rs
  QA scenarios:
  - Happy: `on_completed` sets `phase=Completed` and `outcome` but does NOT clear `completed_tools`
  - Failure: verify `on_turn_start` still clears completed_tools (line 109), `on_tool_complete` still inserts at index 0 (line 135), `truncate(8)` still works (line 143)
  Evidence: .omo/evidence/task-1-tui-turn-display.diff
  Commit: Y | fix(agent): preserve completed_tools across on_completed for TUI flush

- [ ] 2. Multi-block flush: build reasoning + response + tool result MessageRenderEntries
  What to do / Must NOT do: In `oben-tui/src/panels/chat.rs:203-232`, REPLACE the flush block. Current code (lines 203-232) creates a single `MessageRenderEntry { role: Assistant, body_lines, tool_calls: [], reasoning: None }`. Replace with:

  ```rust
  // 1. Reasoning block (if present)
  let mut new_entries: Vec<MessageRenderEntry> = Vec::new();
  if !ts.reasoning_text.is_empty() {
      let reasoning_color = self.renderer.current_palette().muted;
      let reasoning_lines: Vec<StyledLine> = ts.reasoning_text
          .lines()
          .map(|line| StyledLine {
              content: Line::styled(
                  line.to_string(),
                  Style::default().fg(reasoning_color).add_modifier(Modifier::DIM),
              ),
              role_color: None,
          })
          .collect();
      new_entries.push(MessageRenderEntry {
          role: MessageRole::Assistant,
          is_tool_result: false,
          body_lines: reasoning_lines,
          tool_calls: Vec::new(),
          reasoning: Some(ts.reasoning_text.clone()),
      });
      ts.reasoning_text.clear(); // consumed, like streaming_text at line 208
  }

  // 2. Main response block with tool call indicators
  if !text.is_empty() {
      let mut tool_calls_for_display = Vec::new();
      // Show completed tool names as indicators
      for ct in &ts.completed_tools {
          let indicator = if ct.has_error {
              format!("{} {{error}}", ct.name)
          } else {
              ct.name.clone()
          };
          tool_calls_for_display.push(indicator);
      }
      let body_lines = vec![StyledLine {
          content: Line::from(text.clone()),
          role_color: None,
      }];
      new_entries.push(MessageRenderEntry {
          role: MessageRole::Assistant,
          body_lines,
          is_tool_result: false,
          tool_calls: tool_calls_for_display,
          reasoning: None,
      });
  }

  // 3. Tool result blocks (from completed_tools)
  for ct in &ts.completed_tools {
      let preview = if ct.output_preview.chars().count() > 60 {
          format!("{}...", ct.output_preview.chars().take(60).collect::<String>())
      } else {
          ct.output_preview.clone()
      };
      let status_icon = if ct.has_error { "💥" } else { "✅" };
      let status_color = if ct.has_error {
          self.renderer.current_palette().danger
      } else {
          self.renderer.current_palette().success
      };
      let body_lines = vec![
          StyledLine {
              content: Line::from(Span::styled(
                  format!("   {status_icon} {}({})", ct.name, preview),
                  Style::default().fg(status_color),
              )),
              role_color: Some(status_color),
          },
      ];
      new_entries.push(MessageRenderEntry {
          role: MessageRole::Tool,
          is_tool_result: true,
          body_lines,
          tool_calls: Vec::new(),
          reasoning: None,
      });
  }

  // 4. Flush: persist to message_entries AND clear TurnState fields
  ts.streaming_text.clear();
  ts.completed_tools.clear(); // NOW safe — flush just read them
  // Drop ts lock before acquiring message_entries lock to avoid deadlock
  drop(ts);
  if !new_entries.is_empty() {
      self.message_state
          .message_entries
          .lock()
          .unwrap()
          .extend(new_entries);
  }
  ```

  Key points:
  - Reasoning entry placed FIRST (before response) to show thinking → response flow
  - Tool indicators in `tool_calls` field (uses existing DIM indicator rendering)
  - Tool result entries are `MessageRole::Tool` with `is_tool_result: true` (uses BlockType::ToolResult rendering)
  - Colors: `palette.success` for success, `palette.danger` for error (ratatui_themes ThemePalette field)
  - Clear `reasoning_text`, `completed_tools`, AND `streaming_text` AFTER flush (mirrors line 208 pattern); flush now clears all fields it consumed

  Must NOT do: Do NOT modify the `Message` struct. Do NOT persist `reasoning` to session messages. Do NOT change `streaming_text` behavior (only clear it here).
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: T4
  References: `oben-tui/src/panels/chat.rs:191-232` (current flush — REPLACE), `kind.rs:60-64` (CompletedTool), `kind.rs:74-83` (TurnState fields), `message_renderer.rs:192-203` (MessageRenderEntry), `message_renderer.rs:410-424` (tool indicator DIM pattern), `message_renderer.rs:492-525` (reasoning ENTRY styling), `conversation.rs:22-27` (BlockType::ToolResult for indented rendering), `kind.rs:106-115` (on_turn_start clears all fields)
  Acceptance criteria (agent-executable):
  1. `cargo check -p oben-tui` passes
  2. Flush produces: [reasoning entry?][assistant entry with tool indicators][tool result entries...]
  3. Empty turn (no reasoning, no tools, no streaming text) produces NO new entries (zero-entries case)
  4. All consumed fields cleared: streaming_text, reasoning_text, completed_tools
  QA scenarios:
  - Happy with reasoning: produces reasoning entry (DIM) → assistant entry with indicators → tool entries
  - Happy with tools but no reasoning: produces assistant entry with indicators → tool entries
  - Happy empty turn: no entries added (identical to current behavior)
  - Failure: check flush with completed_tools empty produces no tool result blocks
  Evidence: .omo/evidence/task-2-tui-turn-display.diff + .omo/evidence/task-2-tui-turn-display-compile.txt
  Commit: Y | feat(tui): multi-block flush for reasoning, tools, and results

- [ ] 3. Stream reasoning text: display reasoning_text in live turn view (Phase 2.5)
  What to do / Must NOT do: In `oben-tui/src/widgets/conversation.rs:847+` (Phase 2.5 — STREAMING BLOCK RENDERING), modify the stream block rendering so that when `is_streaming` is true, the stream block also displays `reasoning_text` from `TurnState`.

  Implementation: In the `stream_parsed` section (Phase 1.5, conversation.rs:668-727), after constructing `stream_lines` from `streaming_text`, check `ts_ref.reasoning_text` for non-empty content. If present, PREPEND reasoning lines (DIM, muted color) to `stream_lines` before wrapping. Use the exact same styling as `render_entries` reasoning at `message_renderer.rs:492-525`: `palette.muted` (or `palette.info` for non-assistant) with `Modifier::DIM`.

  ```rust
  // In Phase 1.5 (around line 692 in conversation.rs), after parsing streaming lines:
  if ts_len > 0 {
      let raw = ts_ref.streaming_text.trim_start_matches(|c: char| c.is_whitespace());
      let mut stream_lines: Vec<Line<'static>> = raw.lines()
          .map(|l| Line::from(Span::styled(
              l.to_string(),
              Style::default().fg(palette.info).add_modifier(Modifier::DIM),
          )))
          .collect();
      
      // Prepend reasoning text (muted) if present
      if !ts_ref.reasoning_text.is_empty() {
          let reasoning_color = palette.muted; // consistent with render_entries
          let reasoning_lines: Vec<Line<'static>> = ts_ref.reasoning_text
              .lines()
              .map(|line| Line::from(Span::styled(
                  line.to_string(),
                  Style::default().fg(reasoning_color).add_modifier(Modifier::DIM),
              )))
              .collect();
          stream_lines.splice(0..0, reasoning_lines);
      }
      
      let wrapped = layout::wrap_styled_lines_to_lines(&stream_lines, inner_width.saturating_sub(2));
      // ... rest unchanged
  }
  ```

  Key points:
  - Reasoning lines are PREPENDED to the stream block (thinking → response)
  - Uses `palette.muted` color, `Modifier::DIM` — identical to `render_entries` reasoning
  - Only when `is_streaming=true` (live view), not when displaying persisted messages
  - Thread-safe: uses `try_lock()` like existing streaming_text access at line 671
  - No layout changes: reasoning text wraps the same way as streaming text

  Must NOT do: Do NOT change the stream block area height calculation based on reasoning. Do NOT add collapsibility. Do NOT modify Phase 2 rendering of persisted messages.
  Parallelization: Wave 3 | Blocked by: T1 | Blocks: T4
  References: `conversation.rs:668-727` (Phase 1.5 — parse & wrap streaming text — MODIFY), `conversation.rs:847-943` (Phase 2.5 — render stream block — MODIFY), `message_renderer.rs:492-525` (render_entries reasoning styling — reference), `conversation.rs:496-500` (role_title/role_border_style — for consistency)
  Acceptance criteria (agent-executable):
  1. `cargo check -p oben-tui` passes
  2. During streaming, `reasoning_text` appears as DIM text BEFORE streaming text in the stream block
  3. When `reasoning_text` is empty, stream block behavior is UNCHANGED
  4. No layout breakage: stream block height calculation unchanged
  QA scenarios:
  - Happy: turn starts, reasoning appears in muted color, then streaming text follows
  - Failure: no reasoning from model → stream block shows only streaming text (unchanged)
  Evidence: .omo/evidence/task-3-tui-turn-display.diff
  Commit: Y | feat(tui): display reasoning_text during streaming in turn panel

- [ ] 4. Compile check, cargo test, and scope audit
  What to do / Must NOT do: Run `cargo check -p oben-agent && cargo check -p oben-tui && cargo test -p oben-agent --lib && cargo test -p oben-tui --lib`. Verify all existing tests still pass. Check clippy (`cargo clippy -p oben-tui -p oben-agent -- -D warnings`). Review final diff against Must NOT have list.
  Parallelization: Wave 4 | Blocked by: T2, T3 | Blocks: F1-F4
  References: All prior tasks
  Acceptance criteria (agent-executable): All checks pass with zero errors and zero new warnings. All existing tests pass. No file outside scope touched.
  QA scenarios: Happy — zero errors, zero warnings, all tests green. Failure — any compilation error or new warning is a blocker.
  Evidence: .omo/evidence/task-4-tui-turn-display-check.txt + .omo/evidence/task-4-tui-turn-display-test.txt
  Commit: Y | ci(tui): final compile check and integration test

- [x] 5. Add proper markdown rendering with pulldown-cmark ✅ **COMPLETED**
  What to do / Must NOT do: In `oben-tui/Cargo.toml`, add `pulldown-cmark = "0.12"`. In `oben-tui/src/widgets/message_renderer.rs`, replace the custom `tokenize()` function and `render_body_lines()` implementation with a `pulldown-cmark` parser. The current custom tokenizer only handles: inline code, bold, italic, fenced code blocks, headings, blockquotes. Replace with complete markdown support: tables, ordered/unordered lists, headings, code blocks, links, bold/italic, blockquotes, horizontal rules. Keep `MessageRenderEntry`, `StyledLine`, `render_message_entry()`, and `MessageRenderer` unchanged. Remove old `Token` enum and `tokens_to_spans()` function. Verify `cargo check -p oben-tui` and `cargo test -p oben-tui --lib` pass.
  Must NOT do: Do NOT change `Message`, `MessageContent`, or `TokenCall` structs. Do NOT change `render_message_entry()` or `MessageRenderer`. Keep existing tests passing.
  Parallelization: Wave 5 | Blocked by: none | Blocks: F1-F4
  References: `oben-tui/Cargo.toml` (add dependency), `message_renderer.rs:217-266` (render_body_lines — REPLACE), `message_renderer.rs:23-178` (tokenize + tokens_to_spans — REMOVE)
  Acceptance criteria (agent-executable):
  1. `cargo check -p oben-tui` passes
  2. Tables render with monospace alignment
  3. Lists show bullet/number prefixes
  4. Code blocks render dim + green with language labels
  5. All existing tests still pass
  QA scenarios:
  - Happy: full markdown (tables, lists, code, links) renders properly
  - Failure: cargo check fails or tests break
  Evidence: .omo/evidence/task-5-markdown-rendering.diff + .omo/evidence/task-5-markdown-rendering-compile.txt
  Commit: Y | feat(tui): replace custom markdown tokenizer with pulldown-cmark

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit — verify all must-have items implemented, no must-not-have items violated
- [ ] F2. Code quality review — idiomatic Rust, no any/unwrap/panic, proper lifetimes, no Arc<Mutex<T>> where &mut suffices
- [ ] F3. Real manual QA — run the TUI, observe a turn with reasoning+tools, confirm all blocks render correctly
- [ ] F4. Scope fidelity — no files outside oben-agent/src/hooks/kind.rs, oben-tui/src/panels/chat.rs, oben-tui/src/widgets/conversation.rs modified (except for test updates)

## Commit strategy
Single PR: `#X-tui-turn-display` with title `#X: Display reasoning and tool calls in TUI chat panel`
Commit: Atomic — all 4 changes in one commit. Documentation unchanged (parity tracking deferred to later).

## Success criteria
1. TurnState preserves completed_tools and reasoning_text until flush completes
2. Flush produces reasoning block (if present) → response + tool call indicators → tool result blocks
3. Each completed tool displays as an indented block with name, preview, and error indicator
4. Live turn shows reasoning in muted color alongside streaming text
5. Zero regression: existing turns without reasoning/tools render identically to before
