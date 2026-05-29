# PRD-tui-parity.md — TUI / Terminal UI Parity Tracker

Reference: `hermes-agent/ui-tui/` (~60 TypeScript source files across `packages/hermes-ink` + app layer)
Local: `oben-tui/` (~8 Rust source files, 1191 total lines)

The reference TUI is built on Ink (React-like component model, Yoga layout engine, full parse-keypress, cursor/select/search/highlight, OSC52 clipboard, tmux/SSH detection, slash commands, virtual scrolling, input history, completion, overlay system, delegation dashboard, model picker, skills hub, live progress/charms, voice, precision wheel scrolling).

Our `oben-tui` crate is a minimal ratatui-based TUI with 4 panels (chat, sessions, config, setup) and a basic style system.

## Feature Matrix

| Feature Matrix / Gap | Severity | Status | GitHub Issue | Reference File & Lines | Notes / Context |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Ink component model / Yoga layout** | priority-critical | ❌ | — | `packages/hermes-ink/` (120+ files) | Full React-like component tree with Yoga layout engine (flexbox), reconciliation, render-to-screen. We use ratatui (flat layout). |
| **Bracket paste detection** | priority-medium | ⚠️ Partial | PR#61 | `oben-tui/src/panels/chat.rs:240-249` | `ChatPanel::handle_bracket_paste()` method exists to buffer raw paste text. Detection of `\x1b[?2004h` escape not wired into event reader yet (TuiEvent::BracketPaste removed). |
| **Parse-keypress (full keymap)** | priority-high | ⚠️ Partial | PR#61 | `oben-tui/src/panels/chat.rs:175-228` | Handles Up/Down (history), Left/Right (cursor), Backspace/Delete, Enter (submit), Ctrl+U (clear line), Ctrl+V (paste). Missing: Alt+Enter (newline), Ctrl+W (delete word), Ctrl+A/E (home/end), Alt+arrows (word nav). |
| **Cursor + selection + hit-test** | priority-high | ❌ | — | `cursor.ts`, `selection.ts`, `hit-test.ts` | Click-to-move cursor, drag-select, multi-select regions, keyboard shift. We have no mouse input handling beyond capture. |
| **Search highlight** | priority-medium | ❌ | — | `searchHighlight.ts` | Highlights matches in transcript during scroll (overlay). We have no search-in-transcript feature. |
| **Hyperlink hover** | priority-low | ❌ | — | `hyperlinkHover.ts` | Cursor-over hyperlink detection, OSC 8 escape support. Not implemented. |
| **BIDI text rendering** | priority-low | ❌ | — | `bidi.ts` | Arabic/Hebrew right-to-left handling with Unicode combining. Not needed for current scope. |
| **Wrap-ANSI / wrap-text** | priority-medium | ❌ | — | `wrapAnsi.ts`, `packages/hermes-ink/src/ink/wrap-text.ts` | ANSI-aware word wrapping, string width measurement (`stringWidth.ts`), widest-line cache. We use ratatui's built-in wrapping. |
| **OSC52 clipboard** | priority-high | ❌ | — | `osc52.ts` | Full duplex: read (`readOsc52Clipboard`) and write (`writeOsc52Clipboard`) clipboard via terminal escape sequences. Also has `forceTruecolor.ts` for 24-bit color support. |
| **System clipboard (native)** | priority-high | ✅ | PR#61 | `oben-tui/src/clipboard.rs` | Platform-agnostic: pbpaste (macOS), wl-paste (Wayland), wl-copy, xclip (X11), powershell (WSL). Isolation check for suspicious binary content. Max 4MB read buffer. |
| **Terminal setup detection** | priority-medium | ❌ | — | `lib/terminalSetup.ts` | Detects remote SSH, tmux, Apple Terminal, VSCode/Cursor/Windsurf terminals. Pads terminal size for missing escape support, prompts for `/terminal-setup`. |
| **Terminal parity hints UI** | priority-medium | ❌ | — | `lib/terminalParity.ts` | Warns about Apple Terminal (Ctrl+arrow rewrite), tmux (OSC52 passthrough), SSH (image clip limit), IDE terminals (Cmd+Enter missing). |
| **Input history (load/append/persist)** | priority-high | ✅ | PR#61 | `oben-tui/src/history.rs` | Persistent readline-style history file at `~/.config/obenalien/.oben_history`. Up/Down arrow cycling through past inputs. 1000-entry limit with deduplication. |
| **Tab completion (slash + path)** | priority-high | ✅ | PR#61 | `oben-tui/src/panels/chat.rs:484-530` | `/` prefix triggers slash command autosuggestions. Tab/Up/Down cycles completions. Escape clears overlay. 8-item visible max. Path completion not yet implemented. |
| **Slash commands (full catalog)** | priority-high | ❌ | — | `app/slash/commands/` | ~30+ slash commands across 5 categories: core (`/help`, `/details`, `/theme`, `/termkeys`, `/voice`, `/clear`, `/todo`, `/reasoning`, `/copy`, `/chat`, `/undo`), session (`/session`, `/compact`, `/new`, `/switch`, `/rename`), ops (`/delegate`, `/terminal-setup`, `/shell`), setup (`/setup`, `/config`, `/model`), debug (`/debug`, `/inspect`, `/memory`). We have 0 slash commands. |
| **Overlay system** | priority-high | ❌ | — | `overlayStore.ts` | Store-backed overlays: approval, clarify, confirm, pager, picker, secret, sudo, agents, modelPicker, skillsHub. Soft reset preserves user toggles. We have no overlay concept. |
| **Turn controller** | priority-high | ❌ | — | `turnController.ts` | Manages turn lifecycle: start, streaming, idle, interrupted. Tracks tool activity trails, todo status, reasoning tags, diff commits. Interrupt cooldown (1500ms). Pushes session snapshots. |
| **Virtual scrolling (virtual history)** | priority-critical | ✅ | PR#61 | `oben-tui/src/panels/chat.rs:656-717` | Viewport-based rendering: only ~50 lines drawn at once. Mouse wheel and Up/Down scroll message history. Scrollbar reflects actual scroll position. |
| **Precise scroll / wheel acceleration** | priority-medium | ❌ | — | `lib/wheelAccel.ts`, `lib/precisionWheel.ts` | Inter-event timing drives wheel step multiplier. Mod-held wheel = precision mode (1 line/row). Shift+arrows for paragraph scroll. Reference: Claude Code port. |
| **Composer state (paste, editor, snippets)** | priority-high | ❌ | — | `useComposerState.ts` | Manages paste detection (clipboard vs OSC52), external editor spawn (`EDITOR`/`VISUAL`), paste snippet labels, large paste (LARGE_PASTE limit). |
| **Submit pipeline** | priority-high | ✅ | PR#61 | `oben-tui/src/panels/chat.rs:59-104` | Double-Enter debounce (150ms), slash command detection (`/help`/`/clear`/`/quit`), large paste blocking (>64KB). |
| **Session lifecycle (create/open/close/switch)** | priority-high | ❌ | — | `useSessionLifecycle.ts` | Full session CRUD via gateway RPC: create, open, resume (from STARTUP_RESUME_ID), close, switch, compact, rename, save, load usage. Active session file write (`HERMES_TUI_ACTIVE_SESSION_FILE`). |
| **Delegation dashboard (/agents)** | priority-medium | ❌ | — | `delegationStore.ts` | Real-time subagent spawn status: max concurrency, max depth, paused/unpaused. Overlay accordion state for subagent sections. Section expand/collapse persistence. |
| **Long-run tool charms** | priority-low | ❌ | — | `useLongRunToolCharms.ts` | Visual indicators for long-running shell/tool executions. Progress bars, spinners, ETA. |
| **Voice recording integration** | priority-low | ❌ | — | `useInputHandlers.ts` (voice toggle) | Ctrl+V toggle recording. Handles `busy`/`recording` states. Uses platform-specific voice key detection (`isVoiceToggleKey`). |
| **Live progress / tool trail** | priority-medium | ❌ | — | `lib/liveProgress.ts`, `turnController.ts` | Shows running tool execution trails, compact tool output in "tool shelf" lines. Dedupes against final assistant narration. |
| **Reasoning tag detection & rendering** | priority-medium | ❌ | — | `lib/reasoning.ts`, `turnController.ts` | Detects `<think>`/`<reasoning>` tags, renders collapsible reasoning blocks with pulse indicator (PULSE_MS). Renders `thought` sections in transcript details. |
| **Todo management (from tool output)** | priority-medium | ❌ | — | `lib/todo.ts` | Parses `TODO:` / `DONE:` markers from messages. Tracks pending/in-progress/completed/cancelled. Renders in `/details expanded`. Archives todo on turn completion. |
| **Git branch display** | priority-low | ❌ | — | `hooks/useGitBranch.ts` | `git branch --show-current` fallback. Shows in status bar. |
| **Usage display in status bar** | priority-medium | ❌ | — | `domain/usage.ts` | Token counts, cost estimation (input/output/tokens), provider tracking. Displayed in status bar. |
| **Model picker overlay** | priority-medium | ❌ | — | `coreCommands` (/model picker) | Two-step model selection: curated curated IDs via picker overlay, not free-text. |
| **Skills hub overlay** | priority-low | ❌ | — | `coreCommands` (/skills) | Browse available skills, toggle on/off, with descriptions and hotkeys. |
| **Debug features (/inspect, /debug)** | priority-low | ❌ | — | `app/slash/commands/debug.ts` | Memory usage, session state inspection, message dump, component state. |
| **Shell subcommands (/shell)** | priority-medium | ❌ | — | `app/slash/commands/ops.ts` | Execute shell commands from TUI without losing the session. Output shown in pager overlay. |
| **Status bar dynamic modes** | priority-medium | ❌ | — | `useMainApp.ts` | Status bar modes: ready, error, interrupted, streaming, compiling, voice processing, session busy, tool running. Color-coded with icons. |
| **Session steering (`/steer`)** | priority-low | ❌ | — | `coreCommands` (/steer) | Runtime system prompt adjustment without full session restart. |
| **Session undo (`/undo`)** | priority-low | ❌ | — | `coreCommands` (/undo) | Removes last user message from history + reissues turn. |
| **Terminal title update** | priority-low | ❌ | — | `hooks/useTerminalTitle.ts` | Updates `$TERM_TITLE` environment-based terminal tab title with session name + status. |
| **Session save (manual checkpoint)** | priority-low | ❌ | — | `coreCommands` (/save) | Explicit save trigger via slash command. |
| **Intro message rendering** | priority-medium | ❌ | — | `domain/messages.ts` | Session intro message on first load (setup required, welcome, etc.). |
| **External URL open** | priority-low | ❌ | — | `lib/openExternalUrl.ts` | `xdg-open` / `open` / `start` platform dispatch for URLs in transcript. |
| **Memory monitoring** | priority-low | ❌ | — | `lib/memoryMonitor.ts` | Tracks RSS/memory usage, warns when approaching limit. |
| **FPS counter / performance** | priority-low | ❌ | — | `lib/fpsStore.ts` | Real-time FPS tracking for render performance. |
| **Platform detection helpers** | priority-medium | ❌ | — | `lib/platform.ts` | macOS detection, isMac, key parsing (`ParsedVoiceRecordKey`), platform-specific defaults. |
| **Termux device detection** | priority-low | ❌ | — | `lib/termux.ts` | Detects Android Termux, adjusts terminal behavior. |
| **Precision wheel step config** | priority-medium | ❌ | — | `lib/precisionWheel.ts` | Configurable steps (WHEEL_SCROLL_STEP), precision mode multiplier via mod+wheel. |
| **Config sync (`/config` slash)** | priority-low | ❌ | — | `app/slash/commands/setup.ts`, `useConfigSync.ts` | Runtime config get/set via slash commands. |
| **External CLI bridge (`/exec`)** | priority-low | ❌ | — | `lib/externalCli.ts` | Bridge to external CLI tools. |
| **Math Unicode rendering** | priority-low | ❌ | — | `lib/mathUnicode.ts` | Renders mathematical Unicode characters (superscripts, fractions, Greek letters). |
| **Emoji rendering** | priority-low | ❌ | — | `lib/emoji.ts` | Emoji detection and width handling (wide emojis take 2 columns). |
| **Cursor advance context** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/components/CursorAdvanceContext.ts` | Automatic cursor position tracking in Ink component tree. |
| **LRU cache for line widths** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/line-width-cache.ts` | Cache line width computations to avoid recalculation. |
| **Node cache / text measurement** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/node-cache.ts`, `measure-text.ts` | Cached measurement of rendered text nodes. |
| **Devtools** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/devtools.ts` | Internal debug rendering hooks. |
| **Screen capture / OSC52 write optimization** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/screen.ts` | Optimizes screen writes for large outputs. |
| **Clear terminal / reset** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/clearTerminal.ts` | Full terminal reset escape sequence. |
| **Colorize (ANSI parsing)** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/colorize.ts` | Parses ANSI color codes into structured styles. |
| **Termio (ANSI escape handling)** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/termio/` (7 files) | Full ANSI escape parser: CSI, OSC, ESC, SGR sequences. |
| **Tabstops** | priority-low | ❌ | — | `packages/hermes-ink/src/ink/tabstops.ts` | Tab character positioning. |
| **Early input detection** | priority-low | ❌ | — | `packages/hermes-ink/src/utils/earlyInput.ts` | Reads pending input before Ink main loop to avoid blocking. |

## Architecture Notes

### Reference TUI Stack (TypeScript)
```
ui-tui/
├── packages/hermes-ink/    # Ink forking: Yoga layout, render, components, hooks
│   ├── src/ink/            # Core layout engine, parsing, rendering
│   ├── src/ink/layout/     # Yoga flexbox engine bindings
│   ├── src/ink/events/     # Keyboard, mouse, resize, paste, click, focus events
│   ├── src/ink/termio/     # ANSI escape sequence parser
│   └── src/hooks/          # React hooks: useInput, useStdout, useStdin, etc.
├── src/app/                # Main app: useMainApp, turnController, session lifecycle
│   ├── slash/commands/     # Slash command implementations (core/session/ops/setup/debug)
│   └── *.ts                # State stores (overlay, turn, submission, composer, etc.)
├── src/hooks/              # Shared hooks: history, completion, virtual history, git, queue
├── src/lib/                # Utilities: clipboard, OSC52, terminal setup, parity, editor, etc.
├── src/content/            # Static content: hotkeys, fortunes, faces, verbs, charms
├── src/domain/             # Domain models: usage, roles, messages, paths, slash config
└── src/protocol/           # Protocol: paste handling, string interpolation (@file)
```

### Our Stack (Rust)
```
oben-tui/
├── src/lib.rs              # Main loop, event reader, App struct, draw_ui, handle_key
├── src/panels/mod.rs       # Panel trait, PanelId enum
├── src/panels/chat.rs      # Chat panel: messages, input bar, streaming indicator
├── src/panels/sessions.rs  # Sessions panel: list, search, select, new, delete, compact
├── src/panels/config.rs    # Config panel: YAML display, basic navigation
├── src/panels/setup.rs     # Setup wizard: provider → model → API key → options
└── src/widgets/
    ├── mod.rs
    └── style.rs            # Color constants, Theme struct (minimal, no dynamic theming)
```

### Gap Summary
| Area | Reference | Ours | Delta |
|---|---|---|---|
| Layout engine | Yoga (flexbox) | ratatui linear layout | Major rewrite |
| Component model | React (Ink), virtual DOM reconciliation | Direct draw | N/A (different paradigm) |
| Panel system | Hooks + stores | Trait + panels | Moderate |
| Keyboard handling | Full Ink parse-keypress + custom handlers | Basic match on KeyCode | Major gap |
| Clipboard | OSC52 + system clipboard (5 platforms) | None | Major gap |
| Input history | Persistent readline-style | None | Major gap |
| Tab completion | Slash + path completion | None | Major gap |
| Slash commands | 30+ across 5 categories | 0 | Major gap |
| Overlays | 10 overlay types with soft reset | None | Major gap |
| Virtual scrolling | 120-item mounted cap + height cache | No scrolling optimization | Major gap |
| Scroll precision | Wheel accel + precision mode + shift-arrows | None | Major gap |
| Terminal detection | SSH, tmux, IDE, Apple Terminal | None | Major gap |
| Status bar | Dynamic modes (8+), color-coded | Static F1-F4 labels | Moderate gap |
| Tool output display | Tool trail + live progress | Basic text | Moderate gap |
| Voice recording | Ctrl+V toggle, bus/busy states | None | Low priority |
| Delegation dashboard | Real-time subagent status | None | Low priority |
