# Architecture

A ratatui TUI coding **agent** over the OpenRouter API, built to make budget and
weak models (4B instruct, gpt-oss, GLM, Qwen, Gemini Flash) usable for real work.
Two pillars drive it:

1. **Agentic tool-use loop** — the model can call a rich filesystem/shell tool set;
   a three-layer safety harness (workspace check + prompt classifier + tool-call
   classifier) governs what it is allowed to do without human approval.
2. **Token efficiency** — prompt caching plus a non-destructive "short-send"
   summarisation rail keep cheap models inside budget without losing context.

Sessions are per-directory, resumable, and independently configured.

---

## 1. Overview

```
┌─────────────────────────────────────────────────────┐
│  ratatui TUI (main thread, synchronous event loop)  │
│                                                     │
│  KeyInput  SessionPicker  Chat  Settings  Effort    │  ← five modes
└──────────────────────┬──────────────────────────────┘
                       │ mpsc channel (per request)
┌──────────────────────▼──────────────────────────────┐
│  tokio runtime (background thread)                  │
│  · stream_complete  (SSE → StreamEvent)             │
│  · shortsend::shape (API-bound payload only)        │
│  · classify_prompt / classify_toolcall              │
│  · awareness::summarize                             │
└──────────────────────┬──────────────────────────────┘
                       │ HTTPS (OpenRouter)
                  ┌────▼────┐
                  │ models  │  cheap budget / reasoning
                  └─────────┘
```

On-disk state lives entirely in `~/.koma/` and is never mutated by the
short-send rail; display and storage are always the full conversation (dual rail).

---

## 2. Event Loop

**File:** `src-agent/src/app/runtime/event_loop.rs::run_loop`

`runtime/` is a **module** (`app/runtime/mod.rs`), not a single file. The entry
point `run_loop` runs the synchronous main-thread loop:

```
tick
 ├─ 1. Drain active_rx  (StreamEvent loop via try_recv)
 │       Token → append to streaming buffer
 │       Reasoning → append to parallel reasoning buffer
 │       Usage → stash prompt_tokens / cached_tokens / cost
 │       ToolCalls → stash pending calls; Done calls advance_turn
 │       Done → advance_turn (commit assistant msg; run tools or end turn)
 │       Error → finish_stream with error; reset agentic-loop state
 │       Compacted → apply_compaction_result (deferred if anim < 1 s)
 │       HarnessVerdict → surfaced only on harness_rx, not here
 ├─ 1b. Drain harness_rx  (advisory PC verdict — separate channel)
 ├─ 1c. Deferred compaction apply (if compact_apply_at gate has passed)
 ├─ 1d. Reindex-completion poll (missing workspace roots → info toast)
 ├─ 1e. Comet shimmer clock (work_since rising/falling edge)
 ├─ 2. Input poll
 │       timeout: 8 ms while waiting, 100 ms idle
 │       drain-all-on-tick: inner poll(Duration::ZERO) loop
 │       Key → controller::input::handle_key → Action → apply_action
 │       Mouse (ScrollUp/Down) → scroll transcript in Chat mode
 │       Resize → dirty = true
 │       Paste → controller::input::handle_paste
 └─ 3. Draw if dirty (view::draw)
```

**Adaptive timeout:** 8 ms streaming (≥ 60 fps token display + comet redraw),
100 ms idle (no busy-spin). Timeout changes when `state.rest.waiting` changes.

---

## 3. Data Flow (MVC)

```
KeyEvent
  → controller/input.rs::handle_key()      →  Action
  → app/runtime/actions.rs::apply_action() →  state mutation
  → view/mod.rs::draw()                    →  terminal frame

Slash input:
  → controller/command.rs::parse()         →  Command
  → app/runtime/commands.rs::apply_slash() →  state mutation / task spawn
```

`controller/input.rs` is purely a translation layer; it returns `Action` values
and never mutates state. `apply_action` owns all state changes and async spawns.
The view is read-only with respect to state.

---

## 4. Async / Streaming Bridge

One `tokio::runtime::Runtime` is created in `main.rs`. The main loop is
synchronous; async work is spawned via `handle.spawn(...)`.

**One channel per request.** `start_stream_task` (in `app/runtime/stream.rs`)
opens a fresh `tokio::sync::mpsc::unbounded_channel` for each request. The
receiver is stored in `state.rest.active_rx`; the sender goes into the spawned
task. When the harness prompt-classifier runs, it uses a **separate** dedicated
channel (`state.rest.harness_rx`) so its verdict never mixes with stream events.

**StreamEvent variants** (defined in `service/mod.rs`):

| Variant | Meaning |
|---|---|
| `Token(String)` | Append text to the streaming buffer |
| `Reasoning(String)` | Append to the parallel reasoning buffer (display-only) |
| `Usage { prompt_tokens, completion_tokens, cached_tokens, cost }` | Stash token/cost accounting |
| `ToolCalls(Vec<ToolCall>)` | Stash requested tool calls; consumed on Done |
| `Done` | Stream finished; call advance_turn |
| `Error(String)` | Stream failed; surface to status line |
| `Compacted { summary, kept_tail }` | /compact result; apply to conversation |
| `HarnessVerdict { allow, reason }` | Advisory PC verdict; delivered on harness_rx only |

**Cancellation.** `abort_current` aborts the task's `AbortHandle` and sets
`active_rx = None`. A dropped receiver silently discards any late events from the
aborted task (the emit helper does `let _ = tx.send(...)`) — no generation
tagging required.

---

## 5. Agentic Loop and Tools

**File:** `app/runtime/stream.rs::advance_turn` / `process_tools` / `run_tool`

The model emits `tool_calls` during streaming. On `Done`, `advance_turn`:
1. Commits the assistant message (content + tool_calls) to the conversation.
2. If no tool calls → turn is done; sets `waiting = false`.
3. If tool calls → runs `process_tools`, which executes or gates each call,
   then calls `finish_tool_round`, which appends results and calls
   `start_stream_task` again. The loop continues until the model returns no more
   tool calls or `MAX_AGENT_STEPS` (40) is reached.

There is **no plan gate** — tools run immediately on the first model call.

**Tool trait** (defined in `tool/mod.rs`):

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;         // JSON Schema
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String>;
}
```

**`ToolCtx`** carries `workspace: PathBuf`, `workspaces: Vec<PathBuf>`, and
`dir_cache: Arc<RwLock<DirCache>>`.

**Built-in tools** (`all_tools()` in `tool/mod.rs`):

| Tool | Source | Notes |
|---|---|---|
| `read` | `tool/fs.rs` | Read file contents |
| `grep` | `tool/search.rs` | Regex search over files |
| `glob` | `tool/search.rs` | Pattern-match file names |
| `write` | `tool/fs.rs` | Create/overwrite a file (risky) |
| `edit` | `tool/fs.rs` | Patch file content (risky) |
| `delete` | `tool/fs.rs` | Delete a file (risky) |
| `bash` | `tool/shell.rs` | Run a shell command (risky) |
| `dir_list` | `tool/fs.rs` | List directory children |
| `dir_cache_update` | `tool/dircache.rs` | Trigger a background reindex |
| `pong` | `tool/pong.rs` | Heartbeat / connectivity probe |

**Sandboxing.** `resolve(workspaces, path)` canonicalises the path and checks
containment inside the target workspace. `resolve_read` is forgiving: a bare
path without a `[N]` prefix tries workspace 0 first; if the file doesn't exist
there, it tries the other workspaces by existence (so weak models that drop the
prefix can still read files in secondary workspaces). Writes always use `resolve`
— strict, no forgiveness.

**Multi-workspace `[N]` prefix.** A path like `[2]src/main.rs` resolves against
workspace 2. A bare path resolves against workspace 0. `parse_ws_prefix` extracts
the index. The `[N]` convention is documented in `src-misc/system-tools.txt`.

**Tool output truncation.** Long tool results are returned as-is and indexed as
blobs in `messages.sqlite`; the short-send layer controls how much of the archive
re-enters each API request.

---

## 6. Tool-Approval Harness

**File:** `src-agent/src/app/harness.rs`

Controlled by `settings.classifier_enabled` (default: on). When disabled the
loop behaves exactly as before — no secondary-model calls, no workspace check.

Three layers:

**WC (workspace check)** — deterministic, no network. Checks whether the session
workdir is the process launch directory or appears in the union of `settings.workdir`
entries and `settings.allowed_folders`. Called once per tool round in `advance_turn`;
if blocked, all pending tool calls are answered with a refusal and the turn halts.

**PC (prompt classifier)** — advisory only. Runs once per user turn as a background
task spawned AFTER `start_stream_task` kicks off (the stream is never gated). Verdict
arrives on `harness_rx` and is surfaced as a toast if `allow = false`. Fail-open:
a classifier failure means `allow = true` with the real error as `reason`.

**TAC (tool-call classifier)** — per risky call (write/edit/delete/bash) in both
agent modes. Intent-aware: it sees the user's latest message plus the proposed call.

TAC per-mode behaviour (when classifier enabled):
- `available + allow` → Auto runs inline; Normal still prompts (user always approves in Normal).
- `available + block` → Auto records a "blocked by harness" result and continues; Normal prompts.
- `unavailable` → BOTH modes degrade to a human `y/n` prompt (real error shown); never silently runs or blocks.

When classifier is disabled: Normal prompts on risky calls; Auto runs them inline.

All classifier calls use `classify_with`, which sets `reasoning: {exclude: true}` to
strip the model's chain-of-thought from the response (keeps verdicts clean and fast).

Timeout: `CLASSIFY_TIMEOUT = 12 s`. Every failure becomes an unavailable verdict
carrying the real cause (HTTP error / timeout / unparseable reply).

---

## 7. Reasoning Channel

The model's `delta.reasoning` field is a **separate streaming channel** from
`delta.content`. The service layer emits `StreamEvent::Reasoning(chunk)` for
these fragments; they accumulate in a parallel buffer in `AppStateRest` and are
rendered dim/italic above the answer.

**Display-only, never persisted:**

- `ChatMessage.reasoning` is `#[serde(skip)]` in `dto/chat.rs` — never
  serialised into `messages.json` or a `ChatRequest` body.
- `take_reasoning()` drains the buffer at assistant-commit time; the text is
  attached to the `ChatMessage` in memory only for rendering.
- The reasoning buffer is also drained unconditionally on interrupt and on every
  tool-round boundary so it can never bleed into the next turn or into the
  short-send fold.

This prevents CoT bleed from contaminating the classifier verdicts, the summary
fold, or the model's next-turn context.

When a model streams its entire answer into `reasoning` and leaves `content`
empty (e.g. deepseek-v4-flash with reasoning on), `final_answer()` promotes the
reasoning text to become the content so it shows in the foreground and persists.

---

## 8. Short-Send (Non-Destructive Token Efficiency)

**File:** `src-agent/src/app/runtime/shortsend.rs`

The differentiator for budget models. `shape()` is a **pure transform** over the
API-bound history clone; the stored conversation, `messages.json`, and the
rendered transcript are never touched.

**Dual rail:**
```
Stored conversation  (messages.json + in-memory Conversation)  ← full, never compressed
Wire payload clone   ← shape() compresses this before POST
```

**Engage decision** (made in `start_stream_task`, upstream of `shape`):

```
usable = context_window - BASE_OVERHEAD (10 000 tokens)

cache_warm = provider_caches
             AND tokens_cached > 0
             AND last_send_at elapsed < cold_window
             (cold_window: 300s sliding-cache, 120s standard)

engage_pct = cache_warm ? ENGAGE_WARM_PCT (80%) : ENGAGE_COLD_PCT (20%)

sticky engage/disengage (hysteresis):
  enter if conv_tokens > engage_pct × usable
  exit  if conv_tokens < DISENGAGE_PCT (15%) × usable
```

**`shape()` pipeline** (only when `short_send_enabled` and `summarizing = true`):

1. Kill-switch check (`settings.short_send_enabled`).
2. Engage gate (`summarizing` flag from upstream).
3. Guard: skip when history length ≤ 3 (too short to compress).
4. Post-compaction guard: bail when `history[1]` starts with `[summary of earlier conversation]` (a `/compact` summary is already present — stacking would break it).
5. Best-effort fold via `update_summary` (no-op unless verbatim tail has grown past `TAIL_HI_PCT` (15%) of usable).
6. Read rolling summary from `messages.sqlite` → bail if none (nothing to compress against yet).
7. Compute verbatim tail: messages after `sum.covers_up_to` (the live exchange + any un-folded tail).
8. Rehydrate blobs (summarised region only): content-search first (keyword LIKE on message text, up to 3 direct matches), fallback to snippet router (secondary LLM) when no keyword overlap. Max `MAX_REHYDRATE = 3`.
9. **B-placement:** summary + blob recalls appended to the SYSTEM message content (after `CACHE_SPLIT_MARK`), NOT emitted as a synthetic assistant turn. Landing after the mark means it rides the uncached volatile tail and does not bust the cached head.
10. Output: `[modified system, verbatim tail...]`.

**`update_summary` fold** (inside `shape`, step 5):

- Token-band hysteresis: only folds when tail tokens > `TAIL_HI_PCT` (15%) of usable; folds down to `TAIL_FLOOR_PCT` (5%) of usable.
- Snaps the fold boundary to a completed-exchange edge (never folds the live in-progress exchange).
- Uses `shortsend_summary_prompt()` (from `src-misc/shortsend-summary.txt`) as system for the secondary model call. Reasoning is OFF on this call (bleed guard).
- Persists new summary to `summary` table in `messages.sqlite`.

**Contrast with `/compact`:** `/compact` is destructive — it rewrites `messages.json` and the in-memory conversation. Short-send is non-destructive: the wire payload is the only thing that changes.

---

## 9. SQLite Blob Archive

**File:** `src-agent/src/model/msglog.rs`

Each session has `messages.sqlite` alongside `messages.json`. Tables:

| Table | Purpose |
|---|---|
| `messages` | Append-only log: role, content, created_at, prompt_tokens, completion_tokens, cost |
| `blobs` | One row per "heavy" message (code fence, large text, tool output): id, msg_id, kind, token_est, snippet |
| `summary` | Single row (id=1): rolling summary text, covers_up_to, sent_start, updated_at |

Heavy thresholds: `token_est >= 400` (≈1 600 chars) for general messages, `>= 150` for tool outputs, or any message containing a triple-backtick fence. Kind: `"code"`, `"tool_output"`, or `"large_text"`.

Snippet extraction skips leading noise lines (box-drawing, fences, blank lines) so the first snippet character is real semantic text; leading noise would otherwise make blobs unsearchable.

`search_blobs(terms, max_msg_id)` does case-insensitive LIKE-OR over message content, ranked by distinct-term-match count. The archive is append-only; `messages` rows are never updated or deleted. Writes are best-effort (callers ignore errors).

The archive survives `/compact` — it is never rewritten on compaction.

---

## 10. Prompt Caching

**Files:** `dto/chat.rs` (`CACHE_SPLIT_MARK`), `dto/openrouter.rs` (`to_wire`, `system_parts`)

One `cache_control: {type: "ephemeral"}` breakpoint is placed on the **stable head** of the system message. `ChatMessage.content` stays a plain `String` internally; `to_wire` converts it to a `WireContent::Parts` array only for the system message.

The system message content is assembled as:

```
[base prompt + personality + project instructions + memory]  ← STABLE (cached)
[plan-word steer]                                            ← STABLE (same word per session)
CACHE_SPLIT_MARK  (two invisible Unicode chars U+2062 U+2061)
["\n\n# Project files (top level)\n" + dir listing]         ← VOLATILE (uncached)
["\n\n# Project summary\n" + awareness text]                ← VOLATILE (uncached)
[short-send summary + blob recalls (when engaged)]          ← VOLATILE (uncached)
```

`to_wire` splits at `CACHE_SPLIT_MARK`, attaches `cache_control: ephemeral` to the
head part only, and emits the tail as a second uncached part. The plan-word steer is
chosen ONCE per `OpenRouterClient` construction (once per session) so the prefix is
byte-stable across all requests in that session.

`usage.prompt_tokens_details.cached_tokens` from the response drives the
`tokens_cached` readout and the `provider_caches` latch (once any response reports
cache hits, the flag is never reset — used by the short-send warmth calculation).

---

## 11. Multi-Model Robustness

**File:** `src-agent/src/service/openrouter.rs`

- **Capability-gated reasoning.** `effort_caps(models, model_id)` checks whether a
  model has a `reasoning` object OR lists `reasoning`/`include_reasoning` in
  `supported_parameters`. The `/effort` menu is only offered for capable models.
  For models where reasoning is mandatory (`mandatory: true`), the "off" option is
  not shown; instead `reasoning: {exclude: true}` is used on secondary/utility calls.
- **No plan gate.** Tools run on the first model call; there is no forced plan step.
- **Context length preference.** `context_length_for` prefers
  `top_provider.context_length` (what the serving provider actually enforces) over
  the nominal `context_length`. Falls back to 128 000 tokens.
- **Provider routing.** `provider_routing_for(slug)` sets `only: [slug], allow_fallbacks: false`
  for non-empty slugs; omits the field entirely for empty slugs (OpenRouter default routing).
- **`reasoning: {exclude: true}`.** Used on ALL secondary / utility calls
  (classifier, fold, router, awareness) to strip chain-of-thought from responses —
  never `enabled: false`, which 400s on mandatory-reasoning endpoints.

---

## 12. Mode State Machine

**File:** `src-agent/src/app/mode/mod.rs`

Five modes, exactly one active at a time:

| Variant | Source | Description |
|---|---|---|
| `KeyInput(KeyInputForm)` | `mode/key_input.rs` | Credentials form (api key, model, provider) |
| `SessionPicker(PickerState)` | `mode/picker.rs` | `--resume` session list with live search |
| `Chat` | — | Normal conversation view (no extra inline state) |
| `Settings(Box<SettingsState>)` | `mode/settings/` | In-app `/settings` dashboard (boxed: large struct) |
| `Effort(Box<EffortPickerState>)` | `mode/effort.rs` | `/effort` reasoning-effort picker overlay (boxed) |

Key transitions:

```
First run                → KeyInput
--resume flag            → SessionPicker
SaveCreds                → Chat
PickerSelect (key set)   → Chat (via warm_session)
PickerSelect (no key)    → KeyInput (prefilled, from_picker=true)
Esc in KeyInput          → Chat (CancelKeyInput) or SessionPicker (CancelKeyInputToPicker)
/settings                → Settings
/effort                  → Effort
SaveSettings / SaveEffort → Chat
/resume                  → SessionPicker
```

---

## 13. On-Disk Layout

```
~/.koma/
├── config.json                  ← global: theme (dark/light), accent colour
├── session.sqlite               ← session registry (UUID → metadata)
├── usage.sqlite                 ← cost ledger across all sessions
├── sessions/
│   └── <pwd_hash>/              ← bucket per working directory
│       ├── settings.json        ← shared model catalogue (per-dir settings)
│       ├── memory/              ← shared project memory (MEMORY.md)
│       ├── media/               ← downloaded files (web_download tool)
│       └── <uuid>/              ← one directory per session
│           ├── settings.json    ← api_key, model, provider, name, effort,
│           │                      workdir[], compaction, awareness_*, classifier_*,
│           │                      allowed_folders[], short_send_enabled, sliding_cache
│           ├── messages.json      ← Vec<ChatMessage> (full transcript; reasoning #[serde(skip)])
│           ├── messages.sqlite    ← append-only archive (messages + blobs + summary tables)
│           └── images/          ← pasted/screenshot attachments
├── run/
│   ├── <session_id>.sock        ← session-daemon socket
│   └── <session_id>.pid         ← advisory PID file
└── mcp.sock                     ← global MCP daemon socket (singleton)
```

`config.json` is read/written by `model/app_config.rs::AppConfig`. It holds only
global visual preferences. All per-session config lives in `settings.json`.

`session.lock` holds the owning process PID; liveness is probed via portable `kill(pid, 0)` (not `/proc`) on unix, so it works on both Linux and macOS; on Windows the equivalent lives in `model::proc_win::pid_alive` (`OpenProcess` + `GetExitCodeProcess`, biased toward reporting a process alive on any ambiguity).

`workdir` in `settings.json` is a `Vec<String>` (backward-compatible: a legacy
plain-string value is deserialized as a one-element vec). The first non-empty
entry is the effective workspace (`Session::workdir()`); all entries contribute to
the harness workspace allow-set and the multi-workspace `[N]` index.

---

## 14. System-Prompt Assembly

**File:** `src-agent/src/resources.rs::build_system_prompt`

Assembly at session load / rebuild:

```
build_system_prompt(memory, agents) =
  system_prompt()       (src-misc/system-prompt.txt, embedded at compile time)
  + "\n\n"
  + system_personality() (src-misc/system-personality.txt)
  [+ "\n\n# Project Instructions\n" + agents]   ← AGENT.md / AGENTS.md in workdir
  [+ "\n\n# Memory\n" + memory]                 ← memory/MEMORY.md
  [+ "\n\n" + system_tools()]                   ← src-misc/system-tools.txt
```

All files are embedded via `include_dir!` at compile time (the `src-misc/`
directory next to the crate root). Missing or blank files fall back to hard-coded
defaults so the binary is always functional.

At request time, `start_stream_task` appends to the system message content BEFORE
`to_wire`:
1. The session's plan-word steer (same word every request per session).
2. `CACHE_SPLIT_MARK` (the cache/uncached boundary).
3. Volatile tail: `# Project files (top level)` dir listing (from `DirCache`).
4. Volatile tail: `# Project summary` awareness text (from `awareness::summarize`).
5. (When short-send engaged) rolling summary + recalled blob blocks.

The multi-workspace `[N]` convention (how the model should prefix tool paths when
multiple workdirs are configured) is documented in `src-misc/system-tools.txt`.

---

## 15. Module Map

| Layer | Path | Responsibility |
|---|---|---|
| DTOs | `src-agent/src/dto/chat.rs` | `ChatMessage`, `Role`, `ToolCall`, `CACHE_SPLIT_MARK` |
| DTOs | `src-agent/src/dto/openrouter.rs` | Wire types: `ChatRequest`, `WireMessage`, `to_wire`, `StreamChunk`, `Delta`, `ModelInfo`, `Usage` |
| Model | `src-agent/src/model/app_config.rs` | `AppConfig` (global theme/accent, `~/.koma/config.json`) |
| Model | `src-agent/src/model/conversation.rs` | `Conversation` (in-memory message list, compaction apply) |
| Model | `src-agent/src/model/session.rs` | `Session` (load/save, `rebuild_system`, `workdir()`/`workdirs()`) |
| Model | `src-agent/src/model/settings.rs` | `Settings` (per-session config, `settings.json`) |
| Model | `src-agent/src/model/store.rs` | Filesystem registry: `list_sessions`, `create_session`, `rename_session`, PID locking |
| Model | `src-agent/src/model/memory.rs` | `load_memory` (reads `memory/MEMORY.md`) |
| Model | `src-agent/src/model/msglog.rs` | SQLite archive: `append`, `totals`, blob indexing, `search_blobs`, rolling summary CRUD |
| Service | `src-agent/src/service/mod.rs` | `StreamEvent` enum definition |
| Service | `src-agent/src/service/openrouter.rs` | `OpenRouterClient`: `stream_complete`, `complete`, `complete_with`, `classify_with`, `summarize_fold`, `pick_blobs`, `effort_caps`, `context_length_for` |
| Controller | `src-agent/src/controller/input.rs` | `handle_key` → `Action`; `handle_paste` |
| Controller | `src-agent/src/controller/command.rs` | `parse` → `Command`; palette matching |
| View | `src-agent/src/view/mod.rs` | `draw` dispatcher; routes to mode-specific views |
| View | `src-agent/src/view/chat.rs` | Chat transcript + status line + input bar |
| View | `src-agent/src/view/markdown.rs` | Markdown block renderer (code fences, inline code, headings) |
| View | `src-agent/src/view/theme.rs` | Colour palette from `AppConfig.theme` / `accent` |
| View | `src-agent/src/view/settings.rs` | `/settings` dashboard layout |
| View | `src-agent/src/view/effort.rs` | `/effort` picker overlay |
| View | `src-agent/src/view/key_input.rs` | Credentials form layout |
| View | `src-agent/src/view/session_picker.rs` | Session list layout |
| App | `src-agent/src/app/mod.rs` | Module root |
| App | `src-agent/src/app/state.rs` | `AppState` + `AppStateRest` (all mutable runtime state) |
| App | `src-agent/src/app/mode/` | `Mode` enum + `KeyInputForm`, `PickerState`, `SettingsState`, `EffortPickerState` |
| App | `src-agent/src/app/runtime/mod.rs` | Runtime module root: `build_client`, `warm_session` |
| App | `src-agent/src/app/runtime/event_loop.rs` | `run_loop` (main synchronous loop) |
| App | `src-agent/src/app/runtime/actions.rs` | `apply_action` (Action dispatcher) |
| App | `src-agent/src/app/runtime/commands.rs` | `apply_slash` (Command dispatcher) |
| App | `src-agent/src/app/runtime/stream.rs` | `start_stream_task`, `advance_turn`, `process_tools`, `run_tool`, `finish_stream`, `abort_current` |
| App | `src-agent/src/app/runtime/shortsend.rs` | `shape`, `update_summary`, `estimate_conv_tokens`, engage constants |
| App | `src-agent/src/app/runtime/terminal.rs` | Terminal setup/teardown helpers |
| App | `src-agent/src/app/harness.rs` | `classify_prompt`, `classify_toolcall`, `workspace_allowed`, `Verdict` |
| App | `src-agent/src/app/awareness.rs` | `summarize` (project-awareness secondary call) |
| Tool | `src-agent/src/tool/mod.rs` | `Tool` trait, `ToolCtx`, `all_tools()`, `resolve`, `resolve_read` |
| Tool | `src-agent/src/tool/fs.rs` | Read, Write, Edit, Delete, DirList |
| Tool | `src-agent/src/tool/search.rs` | Grep, Glob |
| Tool | `src-agent/src/tool/shell.rs` | Bash |
| Tool | `src-agent/src/tool/dircache.rs` | `DirCache`, `DirCacheUpdate`, `reindex` |
| Tool | `src-agent/src/tool/pong.rs` | Pong (heartbeat) |
| Resources | `src-agent/src/resources.rs` | Compile-time embed of `src-misc/`; `build_system_prompt`, `wanderer_word` |
| Misc | `src-misc/system-prompt.txt` | Base system instructions (embedded) |
| Misc | `src-misc/system-personality.txt` | Tone/style addendum (embedded) |
| Misc | `src-misc/system-tools.txt` | Tool-usage guidance + multi-workspace `[N]` convention (embedded) |
| Misc | `src-misc/classifier-prompt.txt` | PC policy prompt (embedded) |
| Misc | `src-misc/classifier-toolcall.txt` | TAC policy prompt (embedded) |
| Misc | `src-misc/shortsend-summary.txt` | Fold model system prompt (embedded) |
| Misc | `src-misc/shortsend-router.txt` | Blob-router model system prompt (embedded) |
| Misc | `src-misc/wanderer.json` | Whimsical plan lead-in word corpus (embedded) |

---

## 16. How to Add a Feature

**New slash command:**
1. Add a variant to `Command` in `controller/command.rs`.
2. Add a `match` arm to `parse()` in the same file (and to `COMMANDS` if it should appear in the palette).
3. Add a `match` arm to `apply_slash()` in `app/runtime/commands.rs`.

**New key binding:**
1. Add a match arm in `controller/input.rs` (`handle_key` or `handle_paste`).
2. Return the appropriate `Action` variant (add one if needed).
3. Handle the new variant in `apply_action` in `app/runtime/actions.rs`.

**New settings field:**
1. Add the field to `Settings` in `model/settings.rs` with `#[serde(default)]`
   and a default fn so existing `settings.json` files load without error.
2. Add a draft field to `SettingsState` in `app/mode/settings/state.rs` and wire
   it through the settings form and `apply_action::SaveSettings`.

**New tool:**
1. Define a zero-size struct implementing the `Tool` trait in the appropriate file
   under `tool/` (or a new file). Implement `name`, `description`, `parameters`
   (JSON Schema), and `run`.
2. Add it to the `vec!` in `all_tools()` in `tool/mod.rs`.
3. If it mutates the workspace, add its name to the `tool_is_risky` match in
   `app/runtime/stream.rs` so the harness and approval gate apply to it.

---

## 17. Build and Run

```sh
# First run — prompts for API key and model, then opens chat
cargo run -p agent

# Session picker — resume a previous conversation
cargo run -p agent -- --resume
```

The binary is the `agent` crate. No environment variables are required; all
configuration is entered interactively and stored per-session in
`~/.koma/sessions/<pwd_hash>/<uuid>/settings.json`.

License: Apache-2.0.

---

For the web GUI architecture, see [`ARCH_DESIGN_WEBGUI.md`](ARCH_DESIGN_WEBGUI.md).
