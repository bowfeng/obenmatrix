---
slug: tui-turn-display
status: awaiting-approval
intent: clear
pending-action: write .omo/plans/tui-turn-display.md
approach: Multi-block flush — produce reason, response, and tool-result entries from TurnState on turn completion; preserve completed_tools across on_completed() (same pattern as streaming_text); stream reasoning live; no breaking changes
---

# Draft: tui-turn-display

## Components (topology ledger)
| id | outcome | status | evidence |
|----|---------|--------|----------|
| C1: TurnState timing fix (`on_completed` don't clear tools/reasoning) | Active | `oben-agent/src/hooks/kind.rs:174-183` |
| C2: ChatPanel multi-block flush (`update_from_turn_state`) | Active | `oben-tui/src/panels/chat.rs:191-232` |
| C3: ToolResult entry type and message_renderer support | Active | `oben-tui/src/widgets/message_renderer.rs:192`, `conversation.rs:22-27` |
| C4: Stream reasoning display (render phase 2.5) | Active | `oben-tui/src/widgets/conversation.rs:847-943` (stream block area) |
| C5: Reasoning persistence in session messages | Deferred | — |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|------------|----------------|-----------|---------------|
| completed_tools/ reasoning_text not cleared in on_completed | ✅ Don't clear — mirror streaming_text pattern | streaming_text already solved this exact timing bug (kind.rs:180-183 comment) | Yes |
| Reasoning persistence to sessions | Deferred — live display first, persist later | Avoids context window bloat; reasoning is ephemeral during turn, can be attached to Message later | Yes |
| Tool indicator format | `{icon} tool_name` + `{icon} tool_name(output_preview)` | Reuse existing `render_message_entry` tool call indicator pattern (DIM modifier) | Yes |
| Test strategy | tests-after | No existing TUI tests exist; instrument with tracing first | Yes |
| Session message `reasoning: None` stays as-is | ✅ Don't change Message model | Out of scope; deferred to later PR | Yes |

## Findings (cited - C1 = confirmed, C2–5 verified)

**F1. The flush writes ONE block, losing everything.**
`chat.rs:203-232`: on flush, creates single `MessageRenderEntry { role: Assistant, body_lines: streaming_text, tool_calls: [], reasoning: None }`

**F2. TurnState has ALL data needed; it's just not exposed long enough.**
`kind.rs:74-83`: `TurnState` has `streaming_text`, `reasoning_text`, `active_tools`, `completed_tools`, `activity`

**F3. TurnState::on_completed() clears completed_tools BEFORE flush can read them.**
`kind.rs:174-183`: `on_completed()` sets `phase=Completed` AND `completed_tools.clear()`. The flush fires on next draw cycle seeing `prev=Streaming → current=Completed`, but tools are already gone. streaming_text is preserved (not cleared) — the same fix needed for completed_tools.

**F4. render_message_entry already builds tool call indicators.**
`message_renderer.rs:389-424`: Takes `msg.tool_calls`, produces DIM-prefixed entries prepended to body. Can be reused for completed tool names in flush.

**F5. render_entries already produces separate reasoning entries.**
`message_renderer.rs:492-525`: Splits reasoning into a DIM/muted entry. The conversation widget just needs to render the extra entry — no additional render logic needed.

**F6. BlockType::ToolResult already renders compact indented boxes.**
`conversation.rs:22-27`: `BlockType::ToolResult` renders as muted indented rounded box (no role title). Perfect for tool output display.

**F7. Streaming block area exists for live display.**
`conversation.rs:847-943`: Phase 2.5 renders streaming text in dedicated assistant block. Can be extended to also render reasoning_text from TurnState.

## Decisions (with rationale)
| decision | rationale |
|----------|-----------|
| Don't clear completed_tools/reasoning_text in on_completed() | Identical timing problem as streaming_text; the codebase already documented this pattern in kind.rs:180-183 |
| Flush creates MessageRenderEntry blocks (not Messages) | TUI display layer; persisting to sessions is a data model concern deferred to later |
| Reasoning rendered as separate block before response | Follows existing render_entries() pattern; matches Hermes-Agent's "thinking" section |
| Tool results rendered as separate ToolResult blocks | Follows existing block flow; tool results appear below response, indented |

## Scope IN

### Must have
- [ ] TurnState preserves `completed_tools` and `reasoning_text` across `on_completed()`
- [ ] ChatPanel flush produces ordered entries: reasoning (if present) → response + tool indicators → tool result blocks
- [ ] Live turn rendering shows reasoning_text (muted, DIM) alongside streaming_text in the stream block area
- [ ] Each completed tool appears as an indented, colored tool result block (success=green, error=red)
- [ ] No layout regression — existing behavior unchanged when flush has no turn data

### Must NOT have
- [ ] Changes to `Message` struct, `ToolCall` struct, or session persistence
- [ ] Changes to streaming_text handling (that's already a separate improvement)
- [ ] Per-tool-detail expansion / collapsible tool blocks (deferred)
- [ ] Reasoning persistence to session messages
- [ ] Changes to the sessions panel or session message display

## Open questions
- None — all forks resolved by defaults. The user has full confidence in this plan.

## Approval gate
status: awaiting-approval
pending action: Approve or request high-accuracy review before implementation
