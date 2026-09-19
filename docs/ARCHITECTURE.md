# Architecture

A ratatui TUI coding **agent** (optional wry GUI) over **multi-provider** HTTP APIs
(OpenAI-compatible, Anthropic Messages, Codex, Koma Free, CommandCode, … — often
via OpenRouter or a direct endpoint). Built to make budget and weak models usable
for real work. Two pillars drive it:

1. **Agentic tool-use loop** — the model can call a rich filesystem/shell tool set;
   a three-layer safety harness (workspace check + prompt classifier + tool-call
   classifier) governs what it is allowed to do without human approval.
2. **Token efficiency** — prompt caching plus a non-destructive "short-send"
   summarisation rail keep cheap models inside budget without losing context.

Sessions are per-directory, resumable, and independently configured. Default
launch is **daemon + thin client**; `koma alone` / `--local` is the standalone path.


---

## 1. Overview

```
┌─────────────────────────────────────────────────────┐
│  ratatui TUI (main thread, sync event loop)         │
│  Mode: Chat, SessionHub, Settings, Agents, Bash, …  │  ← many mutually exclusive modes
└──────────────────────┬──────────────────────────────┘
                       │ mpsc / IPC (local or daemon)
┌──────────────────────▼──────────────────────────────┐
│  tokio (stream, classifiers, sub-agents, …)         │
│  · stream_complete  (provider-specific wire)        │
│  · shortsend::shape (API-bound payload only)        │
│  · classify_prompt / classify_toolcall              │
│  · awareness::summarize                             │
└──────────────────────┬──────────────────────────────┘
                       │ HTTPS
                  ┌────▼────┐
                  │ models  │  multi-provider
                  └─────────┘
```

On-disk state lives entirely in `~/.koma/` and is never mutated by the
short-send rail; display and storage are always the full conversation (dual rail).

---

## 2. Event Loop

**File:** `src-agent/src/app/runtime/event_loop/` (`run_loop` and session drains)

`runtime/` is a **module tree** (`app/runtime/mod.rs`), not a single file. The entry
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
 │       Mouse: native terminal selection (capture off by default;
│              PageUp/PageDown scroll; QuitConfirm re-enables capture)
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
  → controller/input/                 →  Action
  → app/runtime/actions/              →  state mutation
  → view/mod.rs::draw()               →  terminal frame

Slash input:
  → controller/command.rs::parse()    →  Command
  → app/runtime/commands/             →  state mutation / task spawn
```

`controller/input` is purely a translation layer; it returns `Action` values
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
| `read` / `write` / `edit` / `delete` / `dir_list` | `tool/fs/` | Filesystem (risky write path) |
| `grep` / `glob` | `tool/search.rs` | Search |
| `bash` / `bash_output` / `bash_kill` | `tool/shell.rs` + `app/bgbash` | Shell as job; FG parks turn; BG / Ctrl+B detach |
| `git_operator` / `git_cred` / `git_worktree` | `tool/git_*.rs` | Git (prefer over bash) |
| `task` / `task_output` / `task_kill` / `task_send` | `tool/task.rs` + subagent | Sub-agents |
| `remember` / `forget` / `recall` | memory tools | Project memory |
| `web_*` / `browser_*` | internet tools | Fetch / full browser (feature-gated) |
| `plan_enter` / checklist / SDLC tools | plan + `tool/sdlc*` | Plan / mission modes |
| `graph_query` | `tool/graph.rs` | Import graph (when enabled) |
| … | `tool/mod.rs` `all_tools()` | Canonical registry |

Paths: there is **no** `tool/fs.rs` — use `tool/fs/{read,write,edit,delete,dirlist,mod}.rs`.


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

**Persisted for display; stripped on the wire for most model roles:**

- `ChatMessage.reasoning` is stored with the message (see `dto/chat/message.rs`) so
  the transcript can show thinking after reload. Provider-specific
  `reasoning_details` stay `#[serde(skip)]` / non-persisted wire junk.
- `take_reasoning()` drains the live stream buffer at assistant-commit time.
- The buffer is also drained on interrupt and tool-round boundaries so it cannot
  bleed into the next turn or the short-send fold.
- Secondary calls (classifier, fold, …) still use `reasoning: {exclude: true}` so
  CoT does not contaminate utility replies.

When a model streams its entire answer into `reasoning` and leaves `content`
empty (e.g. deepseek-v4-flash with reasoning on), `final_answer()` promotes the
reasoning text to become the content so it shows in the foreground and persists.

---

## 8. Dual-Rail Short-Send (DRSS)

**Files:** `src-agent/src/app/runtime/shortsend/`, `src-agent/src/model/msglog/drss.rs`,
`src-agent/src/service/context_limits/`

DRSS shapes a clone of the outgoing request. The displayed conversation,
`messages.json`, and original SQLite message bodies remain unchanged. SQLite
stores derived search terms, content fingerprints, and an archive boundary.

```text
History rail: full original conversation -> display + messages.json
                                |
                                +-> outgoing clone
Request rail: [A system] [B deterministic archive index] [C live context] -> [D reply]
```

A stays unchanged by DRSS. B is a separate ordinary user-role context message
before C, never appended to the system prompt. It contains the current objective,
kickoff charter, historical user constraint quotes, matching archive excerpts,
and indexed term occurrence/message counts with exact message IDs. B is bounded
to 2,000 estimated tokens or 2% of the operating window, whichever is smaller.
It does not invoke an inference model, read stored reasoning, or reuse the legacy
rolling summary. Excerpts are historical evidence; live user messages take
precedence. Assistant excerpts require a history/plan request in the raw user
message; assistant/tool text supplies search relevance only. Per-message indexing
keeps at most 512 distinct terms, with exact repetition counts for those terms.

**Context discovery:** a dedicated, unauthenticated client reads OpenRouter's
public `/api/v1/models` catalog. It sends no provider/OAuth credentials. Matching
uses an optional explicit alias, exact IDs/canonical slugs, normalized punctuation
and word order, then a constrained one-character spelling tolerance. Versions,
snapshot dates, and tiers remain distinct. Ambiguous/fuzzy matches cannot raise
the 128k fallback and may lower it. For unresolved provider-prefixed names, the
public `/api/v1/model/{author}/{slug}` endpoint can resolve OpenRouter aliases.
The actual dispatched model ID is never rewritten.

Catalog results have a 24-hour memory/disk cache, a five-minute failure retry,
and stale-cache fallback. Requests have a three-second timeout and bounded
response size. A smaller known native-endpoint limit or user override wins.
OAuth metadata is an estimate of the named model, not a guarantee about a hidden
backend. Match provenance and effective limits are written to the session's
`drss-context.json` for diagnosis.

```text
W = min(detected context window, optional smaller override, 300,000)
C target = 60% W
C ceiling = 75% W
A + B + C + tool schemas + framing + reserved D + margin <= W
```

For a detected 1M model, W is 300k: C targets 180k and can grow to 225k.
System/tools/output requirements can lower those C budgets. Once C exceeds its
ceiling, oldest indexed completed units leave the outgoing clone until it reaches
the target. The persisted boundary prevents old units returning on the next turn,
so C grows through the 60–75% band before another cut. Cache warmth and message
counts do not override these token bands. Token counts are conservative estimates;
provider tokenization and image accounting can differ.

The latest user request, unfinished/live tool round, attachments, and unindexed
messages are protected. Assistant tool calls and all their results stay together.
When necessary, indexed large assistant/tool bodies become preview/read-pointer
stubs while their call IDs, arguments, and replay metadata remain intact. Full
text is available through `message_find({message_id, offset, limit})`, which returns
bounded Unicode-character pages and `next_offset`. Archive-read results are never
restubbed. An oversized protected request or unavailable archive produces an
explicit context error when safe shaping is impossible; the history rail remains
intact. `/clear` resets the active index range; resend truncation invalidates stale
boundary/index entries.

**Objective precedence:** explicit user goal > approved mission's single active
open leaf > immutable kickoff charter. Assistant drafts never set the objective.
Repeated goal/clear phrases during tool continuations do not rewrite provenance.

**Settings:** `short_send_enabled` is the master switch. `context_window_limit`
(0 = automatic) can lower the operating ceiling; `context_model_alias` supplies
an explicit OpenRouter capability ID. Both are exposed in GUI session settings
and persisted in session `settings.json`. `max_output_tokens` requests a reply
limit (auto = 32k, or 256k on direct xAI), clamped by available room and provider metadata. Generic
chat completions, Anthropic, and Command Code receive this calculated limit.
Codex OAuth rejects output-limit fields, so its output is provider-controlled;
input shaping still reserves reply room. Sub-agents retain their separate endpoint budget policy.
Legacy `short_send_engage_n`, `short_send_tail_n`, and `sliding_cache` fields remain
readable for compatibility but no longer control DRSS.

`/compact` is a separate, explicitly requested operation that rewrites the visible
conversation. DRSS never invokes it.

---

## 9. SQLite Blob Archive

**File:** `src-agent/src/model/msglog.rs`

Each session has `messages.sqlite` alongside `messages.json`. Tables:

| Table | Purpose |
|---|---|
| `messages` | Append-only log: role, content, created_at, prompt_tokens, completion_tokens, cost |
| `blobs` | One row per "heavy" message (code fence, large text, tool output): id, msg_id, kind, token_est, snippet |
| `summary` | Legacy rolling-summary record (unused by deterministic DRSS) |
| `drss_index` / `drss_terms` | Derived content fingerprints and bounded term counts |
| `drss_state` | Active archive range and persisted outgoing boundary |

Heavy thresholds: `token_est >= 400` (≈1 600 chars) for general messages, `>= 150` for tool outputs, or any message containing a triple-backtick fence. Kind: `"code"`, `"tool_output"`, or `"large_text"`.

Snippet extraction skips leading noise lines (box-drawing, fences, blank lines) so the first snippet character is real semantic text; leading noise would otherwise make blobs unsearchable.

The DRSS `drss_index` / `drss_terms` tables provide deterministic search and counts. Original message content is preserved by DRSS. Explicit resend can truncate abandoned history; `/compact` preserves the archive. Archive appends are best-effort, so DRSS protects any live message it cannot match to an archived record.

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
```

`to_wire` splits at `CACHE_SPLIT_MARK`, attaches `cache_control: ephemeral` to the
head part only, and emits the tail as a second uncached part. The plan-word steer is
chosen ONCE per `OpenRouterClient` construction (once per session) so the prefix is
byte-stable across all requests in that session.

`usage.prompt_tokens_details.cached_tokens` from the response drives the
`tokens_cached` readout and the `provider_caches` latch (once any response reports
cache hits, the flag is never reset). DRSS uses its fixed token bands independently.

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

Exactly one `Mode` is active. Variants include (not exhaustive — see the enum):

| Variant | Role |
|---|---|
| `Onboard` / `OnboardProvider` | First-run chooser / provider OAuth wizard |
| `KeyInput` | Credentials form |
| `SessionPicker` | Legacy `--resume` list (startup flag) |
| `SessionHub` | `/resume` live + history two-pane hub |
| `Chat` | Normal conversation |
| `Loading` | Warming splash |
| `Settings` | `/settings` |
| `Agents` / `Mcp` / `Extensions` / `Store` / `Security` | Full-screen managers |
| `Bash` / `Todo` / `Remote` / `Help` | Overlays / panels |
| `Effort` / model cmd / skill / rewind / quit confirm / … | Other overlays |
| `ExtScreen` | Extension-driven TUI screen |

Key transitions (simplified):

```
First run                     → Onboard / KeyInput
--resume flag                 → SessionPicker (or hub path)
/resume                       → SessionHub
SaveCreds / warm complete     → Chat
/settings /agents /mcp /bash… → matching Mode
Esc from overlay              → Chat (or prior detail)
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
│           │                      allowed_folders[], short_send_enabled,
│           │                      context_window_limit, context_model_alias, max_output_tokens
│           ├── messages.json      ← Vec<ChatMessage> (full transcript; reasoning #[serde(skip)])
│           ├── messages.sqlite    ← append-only archive (messages + blobs + legacy summary + derived DRSS tables)
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
DRSS then inserts B as a separate context message after the system message; it does not modify the system text.

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
| Model | `src-agent/src/model/msglog.rs` | SQLite archive: `append`, `totals`, blob indexing, deterministic DRSS index, legacy summary CRUD |
| Service | `src-agent/src/service/mod.rs` | `StreamEvent` enum definition |
| Service | `src-agent/src/service/openrouter.rs` | `OpenRouterClient`: `stream_complete`, `complete`, `complete_with`, `classify_with`, `summarize_fold`, `pick_blobs`, `effort_caps`, `context_length_for` |
| Controller | `src-agent/src/controller/input.rs` | `handle_key` → `Action`; `handle_paste` |
| Controller | `src-agent/src/controller/command.rs` | `parse` → `Command`; palette matching |
| View | `src-agent/src/view/mod.rs` | `draw` dispatcher; routes to mode-specific views |
| View | `src-agent/src/view/chat.rs` | Chat transcript + status line + input bar |
| View | `src-agent/src/view/markdown.rs` | Markdown block renderer (code fences, inline code, headings) |
| View | `src-agent/src/view/theme.rs` | Colour palette from `AppConfig.theme` / `accent` |
| View | `src-agent/src/view/settings.rs` | `/settings` overlay layout |
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
| Tool | `src-agent/src/tool/fs/` | Read, Write, Edit, Delete, DirList |
| Tool | `src-agent/src/tool/shell.rs` | Bash tool defs |
| App | `src-agent/src/app/bgbash/` | Bash job registry (FG + BG) |
| App | `src-agent/src/lsp/` | Host LSP for GUI coding panel |
| Tool | `src-agent/src/tool/search.rs` | Grep, Glob |
| Tool | `src-agent/src/tool/shell.rs` | Bash tool defs + `capture_raw` (non-intercept callers) |
| App | `src-agent/src/app/bgbash/` | Bash job registry: every model `bash` is a `BashJob`; Ctrl+B promote |
| Tool | `src-agent/src/tool/dircache.rs` | `DirCache`, `DirCacheUpdate`, `reindex` |
| Tool | `src-agent/src/tool/pong.rs` | Pong (heartbeat) |
| Resources | `src-agent/src/resources.rs` | Compile-time embed of `src-misc/`; `build_system_prompt`, `wanderer_word` |
| Misc | `src-misc/system-prompt.txt` | Base system instructions (embedded) |
| Misc | `src-misc/system-personality.txt` | Tone/style addendum (embedded) |
| Misc | `src-misc/system-tools.txt` | Tool-usage guidance + multi-workspace `[N]` convention (embedded) |
| Misc | `src-misc/classifier-prompt.txt` | PC policy prompt (embedded) |
| Misc | `src-misc/classifier-toolcall.txt` | TAC policy prompt (embedded) |
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
