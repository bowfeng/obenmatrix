# Tools — Parity vs Hermes-Agent

**Scope:** `oben-tools` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/tools/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| TL.1 | ToolRegistry + auto-registration | ✅ | ✅ | (built-in) | Dynamic registration, dispatch |
| TL.2 | Shell tool (safe command execution) | ✅ | ✅ | (built-in) | Terminal fg/bg, path security |
| TL.3 | read_file / write_file | ✅ | ✅ | (built-in) | File I/O tools |
| TL.4 | http_get | ✅ | ✅ | (built-in) | HTTP fetch tool |
| TL.5 | web_search | 🟡 | ✅ | (built-in) | Stub with configurable provider |
| TL.6 | search_files (ripgrep) | ✅ | ✅ | (built-in) | `rg`-backed file search |
| TL.7 | patch (fuzzy) | ✅ | ✅ | (built-in) | Multi-search-replace patch |
| TL.8 | web_extract (SSRF + HTML) | ✅ | ✅ | (built-in) | HTML content extraction |
| TL.9 | memory (add/replace/remove) | ✅ | ✅ | (built-in) | Memory CRUD with injection scanning |
| TL.10 | clarify | ✅ | ✅ | (built-in) | Clarification tool |
| TL.11 | todo (JSON store) | ✅ | ✅ | (built-in) | Todo list tool |
| TL.12 | code_execution (sandbox) | ✅ | ✅ | (built-in) | Sandboxed code execution |
| TL.13 | osv_check | ✅ | ✅ | (built-in) | OSV vulnerability check |
| TL.14 | skill (list/view) | ✅ | ✅ | (built-in) | Skill management tool |
| TL.15 | **Search provider** (DuckDuckGo, Brave) | 🟡 | ❌ | [TBD] | Configurable search backend |
| TL.16 | **Browser automation** (CUA-driver) | 🟡 | ❌ | [TBD] | `browser_dialog_tool.py` → `cua-driver` |
| TL.17 | **Voice** (STT/TTS) | 🟡 | ✅ (partial) | (this PR) | whisper-rs, msedge-tts (native Rust crate), OpenAI, ElevenLabs, Mistral, xAI, Gemini |

---

## Voice Testing Guide

**TTS (text → audio):**
```bash
# Send text to agent
Agent: text_to_speech(text="Hello world")
  → Output: MEDIA:~/.config/obenalien/audio_cache/tts_20260612_123456.mp3
  → System sends audio file as voice bubble
```

**STT (audio → text):**
```bash
# Agent calls with audio file path (from voice message or TTS output)
Agent: speech_to_text(audio_file="/path/to/audio.mp3")
  → Output: "Hello world"
```

**End-to-end test via agent interaction:**
1. User: "Say hello to me" (text) → Agent generates audio
2. User: (sends the generated audio as voice message) → Agent transcribes it
3. Verify transcript matches original text
| TL.18 | **Image generation** (FLUX, DALL-E, Midjourney) | 🟡 | ❌ | [TBD] | `image_gen_provider.py` |
| TL.19 | **MCP integration** | 🟢 | ❌ | [TBD] | `mcp_oauth.py`, `mcp_tool.py` |
| TL.20 | **Cron scheduler** | 🟢 | ✅ (#63) | [#63](https://github.com/.../63) `oben-cron/` | Schedule parsing (duration/interval/ISO/cron), JSON persistence, daemon |
| TL.21 | **Delegate tool** | 🟡 | ✅ | #25 | Subagent delegation (`delegate_tool.py`): `SubagentSpawner` (shared DB, fresh context), `CallbacksRelay` (parent→child forwarding), `ToolsetFilter` (blocked tools), TUI wiring with `SpawnFn` |
| TL.22 | **Kanban tools** | 🟡 | ❌ | [TBD] | Task management board |
| TL.23 | **Computer use** | 🟡 | ✅ (#TL.23) | (built-in) | `computer_use.rs` — macOS GUI via cua-driver stdio MCP; capture/click/drag/scroll/type/key/set_value/wait/list_apps/focus_app; safety gates for type/key patterns |
| TL.24 | **Video generation** | 🟢 | ❌ | [TBD] | `video_gen_provider.py` |
| TL.25 | **Home Assistant** | 🟢 | ❌ | [TBD] | `homeassistant_tool.py` |
| TL.26 | **Mixture of Agents** | 🟢 | ❌ | [TBD] | `mixture_of_agents_tool.py` |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.

---

## Voice Tools Implementation (TL.17)

**Implemented in this PR:**

| Component | Details |
|-----------|---------|
| **tts.rs** | Text-to-Speech tool with Flat schema, 7 providers (Edge via native msedge-tts Rust crate, OpenAI, ElevenLabs, Gemini, xAI, Mistral), markdown text cleaning, ffmpeg Opus conversion |
| **stt.rs** | Speech-to-Text tool with 6 providers (whisper-rs local, OpenAI, Groq, Mistral, xAI, ElevenLabs Scribe), WAV loading + resampling, base64 audio support |
| **VoiceConfig** | Added to `oben-config`: `SttConfig` + `TtsConfig` with provider selection, voice/speed/format settings |
| **whisper-rs** | Feature-gated local STT using `whisper-rs = "0.16"` with GGML model download on first use |
| **msedge-tts** | Feature-gated Edge TTS using `msedge-tts v0.4` with tokio-runtime feature |
| **Tests** | 135 tests pass (6 new voice-related unit tests, 1 ignored for live testing) |

**Provider parity achieved:**

| Provider Type | TTS | STT | Status |
|---------------|-----|-----|--------|
| Free/Local | ✅ Edge TTS (msedge-tts rust crate) | ✅ whisper-rs (GGML, ~150MB download) | ✅ |
| OpenAI | ✅ API key required | ✅ `whisper-1` model | ✅ |
| Groq | ❌ (TTS not supported by Groq) | ✅ `whisper-large-v3-turbo` | ✅ |
| Mistral | ✅ Voxtral | ✅ Voxtral transcribe | ✅ |
| xAI | ✅ Grok voice | ✅ `grok-2-transcribe` | ✅ |
| ElevenLabs | ✅ v2 models | ✅ Scribe v2 | ✅ |
| Gemini | ✅ `gemini-2.0-flash` | ❌ (no STT API yet) | Partial |

**Not yet implemented:**
- MiniMax, KittenTTS, Piper (local TTS)
- NeuTTS, custom command TTS providers
- Streaming TTS (`stream_tts_to_speaker`)
- TTS/STT provider auto-detection (uses configured provider)
