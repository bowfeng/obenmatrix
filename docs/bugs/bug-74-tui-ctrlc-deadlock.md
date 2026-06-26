## 🐛 Bug #74: TUI Ctrl+C causes deadlock / hang

**Reported**: Pressing Ctrl+C in the TUI hangs the program instead of exiting.

**Root Cause Analysis:**
- **Core Vulnerability**: The `running` `Arc<AtomicBool>` shared between the event loop and the reader task was never set to `false` in the Ctrl+C shutdown path.
- **Timing Bug**: When the event loop received `TuiEvent::Quit`:
  1. It dropped `chat_tx` and broke from the loop
  2. It then `await`ed `reader_handle`
  3. But `running` was never set to `false`, so the reader task's `while running_clone.load()` condition stayed `true`
  4. The reader task remained stuck in `crossterm::event::poll(16ms)`, which can block indefinitely on some macOS terminal configurations

**Stack Trace (Conceptual):**
```
event_loop: receives TuiEvent::Quit -> drop(chat_tx) -> break -> await reader_handle  {BLOCKED}
reader_task: in spawn_blocking -> poll() -> blocks on terminal fd  {NEVER WRITES running=false}
coordinator: waiting for chat_tx drop -> exits -> coordinator_handle completes  {OK, but irrelevant}
```

**Coverage Gaps:**
- **Unit Layer Missed Because**: The reader task's loop condition was tested in isolation but not in the full event loop shutdown sequence.
- **Integration Layer Missed Because**: Different terminal environments behave differently with `poll()` — the bug manifests only on certain macOS terminal configs.

**Regression BDD Test Scenario:**
- **Given**: TUI is running with active agent turn
- **When**: User presses Ctrl+C
- **Then**: Program exits cleanly within 2 seconds (not hang)
- **Location**: Manual test — run `cargo run -p oben-cli tui` and press Ctrl+C

**Fix Applied:**
1. **Primary** (`lib.rs:673`): Added `running.store(false, Ordering::SeqCst)` in the `TuiEvent::Quit` handler to ensure the reader task's loop exits on its next `poll()` return.
2. **Defensive** (`lib.rs:786-790`): Added a 2-second timeout on `reader_handle.await` so that even if `poll()` blocks indefinitely on the terminal fd, the process exits gracefully.

**Files Changed:**
- `oben-tui/src/lib.rs`
