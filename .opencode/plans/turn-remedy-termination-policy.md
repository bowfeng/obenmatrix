# Plan: Split TerminationPolicy into TurnTerminationPolicy + TurnRemedyPolicy

## Goals
1. Split current `TerminationPolicy` into two separate policy types:
   - **TurnRemedyPolicy** — budget/time check. On budget exhaustion, inject reminder prompt + give 1 turn opportunity. Also handles empty response scenarios (return empty text fallback).
   - **TurnTerminationPolicy** — process_response logic. Decides Continue/Return based on response content (tool_calls vs pure text).
2. Integration: Only if TurnTerminationPolicy returns Continue → then check TurnRemedyPolicy.
3. Remove `IterationBudget` hard-check logic from `turn_executor.rs` — replaced by `BudgetRemedyPolicy`.

## Design

### TurnTerminationPolicy
```rust
pub trait TurnTerminationPolicy: Send + Sync {
    fn evaluate(&self, ctx: &TurnTerminationContext<'_>) -> Result<Option<TurnDecision>>;
}

pub struct TurnTerminationContext<'a> {
    pub response: &'a TransportResponse,
    pub messages: &'a [Message],
}

pub enum TurnDecision {
    Continue,         // tool_calls exist → check TurnRemedyPolicy next
    Return(String),   // pure text response → end turn
    ReturnEmpty,      // empty text + no tool_calls → TurnRemedyPolicy decides next
}
```

### TurnRemedyPolicy + RemedyPolicyGroup
```rust
pub trait TurnRemedyPolicy: Send + Sync {
    fn evaluate(&mut self, call_count: usize, max_calls: usize) 
        -> Result<Option<RemedyAction>>;
}

pub enum RemedyAction {
    None,           // budget OK, normal continue
    Remedy,         // budget exhausted → inject reminder prompt, force 1 more turn
    RemedyExhausted, // already remedied → return last tool result text (soft end)
}
```

**Remedyed once** flag: once the model gets a remedy injection, subsequent budget exhaustion → `RemedyExhausted` (return accumulated text, not another reminder).

### Empty Response Handling (via TurnRemedyPolicy)
- When response text is AND tool_calls are empty → TurnTerminationPolicy returns `ReturnEmpty`
- TurnRemedyPolicy checks `consecutive_empty_responses` count:
  - 1st empty → inject hint, ReturnLastToolResult or empty text
  - 2nd empty → ReturnLastToolResult

### Flow in `execute_turn_with_config`
```
loop:
  1. check_interrupt → early return if interrupted
  2. check_budget (pre-API) → hard error (keep as safety net)
  3. sanitize + steer
  4. API call → response
  5. TurnTerminationPolicy::evaluate(ctx) →
     ├─ Return(text) → return Ok(TurnResult { … })
     ├─ ReturnEmpty → TurnRemedyPolicy handles
     └─ Continue → check budget (TurnRemedyPolicy)
  6. TurnRemedyPolicy::evaluate(call_count, max_calls) →
     ├─ None → dispatch tool results, continue loop
     ├─ Remedy → inject reminder prompt, continue loop (1 more chance)
     └─ RemedyExhausted → return Ok(TurnResult { last text, ... })
```

## Implementation Steps

### Step 1: `coordinator/termination.rs` — New types and traits

1. Define `TurnTerminationPolicy` trait + `TurnTerminationContext`
2. Implement `TurnTerminationPolicy` for `TurnTerminationPolicyGroup` (aggregates policies)
3. Implement `TurnTerminationPolicy` for `DefaultTurnTerminationPolicy` — the default process_response logic
4. Define `TurnRemedyPolicy` trait + `RemedyAction`
5. Implement `BudgetRemedyPolicy` — tracks call_count, max_calls, remedyed_once flag
6. Implement `TimeRemedyPolicy` — tracks elapsed time, max_duration
7. Define `RemedyPolicyGroup`

### Step 2: `turn_executor.rs` — Replace check_budget + process_response

1. Add `max_iterations: usize` field to `TurnConfig`
2. Remove `BudgetCheckResult` enum
3. Remove `ProcessResponseAction` enum
4. In `execute_turn_with_config`:
   - Replace `Self::check_budget()` → keep pre-API budget check for safety only
   - Replace `Self::process_response()` → call new `self.termination_policy.evaluate()`
   - Replace `ProcessResponseAction` match → use TurnDecision match + next check TurnRemedyPolicy
5. Remove `consecutive_empty_responses` tracking → replaced by TurnRemedyPolicy
6. Replace `Self::last_tool_result_text()` → use `TurnDecision::ReturnLastToolResult` directly

### Step 3: `coordinator/mod.rs` — Update call sites

1. Remove `IterationBudget` parameter from `execute_turn` and `execute_turn_full`
2. Add `max_iterations` to `TurnConfig` construction (from `conversation_config.max_iterations`)
3. Pass `max_iterations` via TurnConfig to `TurnExecutor::execute_turn_with_config`

### Step 4: Tests

1. Update `termination.rs` tests → test new policy types
2. Run `cargo test --package oben-agent --lib`
3. Run `cargo check --package oben-agent`
4. Fix any compile errors in `oben-cli` / `oben-tui` if they use the old types

## Files to Modify

| File | Type |
|---|---|
| `oben-agent/src/coordinator/termination.rs` | Add new structs, traits, implementations |
| `oben-agent/src/turn_executor.rs` | Replace old budget/response logic with new policy-based decision |
| `oben-agent/src/coordinator/mod.rs` | Update `execute_turn`/`execute_turn_full` call signatures |
| `oben-agent/src/lib.rs` | Update re-exports if needed (unlikely) |
| `docs/PRD-conversation-parity.md` | Update if parity table exists |

## Trade-offs

- **Pre-API budget check remains**: Keep current hard-error check in `turn_executor.rs` as a safety net before the first API call. New RemedyPolicy is checked after each API call and provides soft-termination with reminder injection.
- **Empty response handling**: Currently handled with `consecutive_empty_responses` counter in `process_response`. Could be kept as a local variable in TurnExecutor or fully moved to TurnRemedyPolicy — keeping it simple as a local variable in TurnExecutor and having TurnRemedyPolicy respond based on count.
