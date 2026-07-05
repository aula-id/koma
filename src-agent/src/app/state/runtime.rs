//! [`SessionRuntime`]: the per-session EXECUTION state carved out of
//! [`super::AppStateRest`].
//!
//! This holds everything tied to ONE session's in-flight turn: its [`Session`],
//! the streaming buffers, the tool-approval / deferred-task / sub-agent state
//! machines, the shared dir cache, and the cache-warmth bookkeeping. Splitting
//! it out is the structural groundwork for running several concurrent sessions
//! later — for now there is always exactly ONE `SessionRuntime` (the foreground
//! one) and behaviour is identical to before the split.
//!
//! Streaming-lifecycle methods (`begin_stream`, `append_token`,
//! `append_reasoning`, `take_stream`, `take_reasoning`) live here because they
//! operate purely on the moved `streaming` / `stream_reasoning` buffers.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::AbortHandle;

use crate::app::mode::Mode;
use crate::app::subagent::{PendingSubagent, SubAgent};
use crate::dto::chat::ToolCall;
use crate::model::session::Session;
use crate::service::StreamEvent;
use crate::tool::DirCache;

use super::types::ToastKind;

/// Per-session execution state. Always non-empty in [`super::AppStateRest::sessions`];
/// the foreground one is reached through `fg()` / `fg_mut()`.
pub struct SessionRuntime {
    /// Stable, process-unique identity (UUID v4), assigned once at creation and
    /// never reused or reordered. This is how the daemon's IPC clients address a
    /// session — NEVER by its `Vec` index, which later session-lifecycle
    /// (tombstoning) would shift and silently cross-wire (see `ipc::proto`
    /// critique #2). Purely additive; the single-process TUI ignores it for now.
    #[allow(dead_code)] // read by the daemon IPC layer in stage 2+
    pub id: String,
    /// This session's CURRENT UI mode (C3): the screen it shows — `Chat` or one of the
    /// slash overlays / pickers (`Settings`, `Help`, `SessionHub`, `Loading`, …) with its
    /// form/picker data. Moved OUT of [`super::AppState`] and onto the session so each
    /// session carries its own overlay state; reached through [`super::AppState::mode`] /
    /// [`mode_mut`](super::AppState::mode_mut), which index the foreground. In the daemon
    /// the per-client foreground is swapped in before each request/projection (C2), so a
    /// client in `/help` over session A no longer forces a client in Chat over session B
    /// into `/help`. A fresh session defaults to `Chat` (see [`Self::new`]); the
    /// spawn/startup flows set the right initial mode on the right session.
    pub mode: Mode,
    /// THIS session's status-line text (C6). Moved off the GLOBAL [`super::AppStateRest`]
    /// so that in the daemon a status flash fired by one session's processing
    /// (`streaming`, `thinking`, `$ cmd — exit`, an error line, …) is projected ONLY into
    /// the client(s) viewing that session — the snapshot reads `fg().status` after the
    /// per-client foreground swap, so each window shows its own. In the single-window TUI
    /// `fg()` is the only session, so behaviour is byte-identical to the old global field.
    /// Defaults to `"ready"`.
    pub status: String,
    /// THIS session's transient toast: `(message, expiry instant, kind)`. Moved off the
    /// GLOBAL [`super::AppStateRest`] for the same reason as [`status`](Self::status) — a
    /// toast a session raises (bash-N finished, session ready, harness flagged, …) is
    /// projected only into the client(s) viewing that session (snapshot reads `fg().toast`).
    /// Shown at the top of the transcript and auto-dismissed once the instant passes;
    /// `kind` selects the box style (red "error" vs neutral "info"). Expiry is swept
    /// PER-SESSION (each session ticks its own toast). `None` when no toast is showing.
    pub toast: Option<(String, std::time::Instant, ToastKind)>,
    pub input: String,
    /// Caret position within `input`, as a CHAR index (0..=char_count). Edits
    /// (insert / backspace) and the Left/Right/Home/End keys move it; the view
    /// paints the block cursor here instead of always at the end. Kept in char
    /// units so multibyte input never splits a code point; converted to a byte
    /// offset only at the `String::insert`/`remove` call site. Reset to the end
    /// on any bulk replace (submit/clear, history recall, completion).
    pub cursor: usize,
    /// Image attachments staged by the composer (path-paste / `@`-picker) that
    /// have NOT yet been submitted. Each was produced by the ingest core (its
    /// bytes are already on disk under `<session>/images/`) and matches an
    /// `[Image #N]` marker inserted into `input`. On submit, these are MOVED onto
    /// the user `ChatMessage` and this is cleared; a `/clear` or take_input that
    /// drops the text also clears them so a stray marker can't outlive its image.
    pub pending_attachments: Vec<crate::dto::chat::Attachment>,
    /// Bash-style input history: index into the sent-user-message list while
    /// recalling (None = editing live input).
    pub hist_idx: Option<usize>,
    /// Live input stashed when history recall starts; restored on recall past
    /// the newest entry.
    pub input_stash: String,
    /// Transcript scroll offset (top visual line) used only while NOT following.
    pub scroll: u16,
    /// When true, the transcript stays pinned to the bottom (auto-follows new
    /// content). Cleared when the user scrolls up; re-set on reaching bottom.
    pub follow: bool,
    pub session: Option<Session>,
    pub waiting: bool,
    pub streaming: Option<String>,
    /// Parallel to `streaming`: the in-progress assistant's reasoning/thinking
    /// text, accumulated from `StreamEvent::Reasoning` deltas during a turn. Set
    /// up alongside the content buffer in `begin_stream`, drained at commit, and
    /// folded onto the committed `ChatMessage` as a display-only block (never
    /// serialised). Empty when the model emits no reasoning.
    pub stream_reasoning: String,
    /// Parallel to `stream_reasoning`: the in-progress assistant's OpenRouter
    /// `reasoning_details`, merged (by index) from `StreamEvent::ReasoningDetails`
    /// deltas during a turn. Armed fresh in `begin_stream`, drained on the tool-call
    /// commit, and echoed back on tool-continuation requests (OpenRouter only) so
    /// the model keeps its signed chain-of-thought across tool calls. Never
    /// serialised; empty when the model emits no structured reasoning.
    pub stream_reasoning_details: Vec<crate::dto::chat::ReasoningDetail>,
    pub current_task: Option<AbortHandle>,
    /// Receiver for the in-flight request's events, or `None` when idle. Each
    /// request owns a fresh channel; dropping this receiver silently discards
    /// any further events from a task that was aborted or superseded.
    pub active_rx: Option<UnboundedReceiver<StreamEvent>>,
    /// Receiver for the advisory prompt-classifier (PC) verdict. Each new turn
    /// (when the classifier is enabled) opens a fresh channel here and spawns a
    /// background task that sends one [`StreamEvent::HarnessVerdict`]. Drained in
    /// `run_loop` independently of the streaming channel, so PC never blocks or
    /// interferes with streaming. `None` when no PC task is in flight.
    pub harness_rx: Option<UnboundedReceiver<StreamEvent>>,
    /// Usage for the in-flight response, captured from the StreamEvent::Usage
    /// chunk and consumed when the assistant message is committed.
    pub pending_usage: Option<(u64, u64, f64)>,
    /// The model id actually DISPATCHED for the in-flight request, captured in
    /// `stream::run::start_stream_task` at the moment `resolve_turn_model` picks
    /// the route (Main, or Planner while `AgentMode::Plan` is active) — BEFORE
    /// the request is sent. The usage-ledger write in `finish_stream` /
    /// `advance_turn` reads this instead of re-resolving the role, because a
    /// stream can run for seconds and `agent_mode` (or the model/route
    /// assignments) may change before that response finishes — re-resolving at
    /// ledger-write time would then misattribute cost to whatever model happens
    /// to be configured NOW, not the one that actually served the request. Reset
    /// on every dispatch; `None` only before the very first send of a session.
    pub pending_dispatch_model_id: Option<String>,
    /// THIS session's cumulative token/cost totals (summed from its own
    /// messages.sqlite on open via `load_token_totals`, incremented per response).
    /// Per-session so each tab tracks only its own usage — switching foreground
    /// just renders the active session's counters, never the sum. Survive /compact.
    /// `tokens_in` is the CURRENT context size (latest prompt), not a running sum;
    /// `tokens_out` and `cost` accumulate.
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    /// Prompt tokens served from the prompt cache on THIS session's LATEST
    /// response (a cache hit at the discounted rate). Like `tokens_in`, tracks the
    /// current prompt, not a cumulative sum; set from `StreamEvent::Usage` each
    /// response, 0 on a cold prefix or a provider that doesn't report cache stats.
    pub tokens_cached: u64,
    /// Tool calls emitted by the in-flight stream, stashed on
    /// `StreamEvent::ToolCalls` and consumed by `advance_turn` once the stream
    /// finalises. Empty when the model returned a plain (final) answer.
    pub pending_tool_calls: Vec<ToolCall>,
    /// Number of tool-call rounds taken in the current turn. Reset to 0 when a
    /// new user turn starts / the turn ends; bounded so a runaway model can't
    /// loop forever.
    pub agent_steps: usize,
    // --- tool-approval state machine (within a single agentic turn) ---
    /// Index of the next call in `pending_tool_calls` to process this round.
    pub tool_idx: usize,
    /// `(tool_call_id, result)` pairs collected so far this round, flushed into
    /// the conversation once every call in the round resolves.
    pub tool_results: Vec<(String, String)>,
    /// True while a risky call is paused waiting for the user's `y/n`. The event
    /// loop routes keys to the approval modal while this is set.
    pub awaiting_approval: bool,
    /// Reason the tool-call classifier (TAC) flagged the currently-paused call,
    /// shown in the approval overlay so the user sees WHY approval is asked.
    /// `None` for an approval that wasn't classifier-driven. Cleared when the
    /// approval resolves.
    pub approval_reason: Option<String>,
    /// One-shot marker: the `call.id` of a `git_worktree` call the user just
    /// approved at the y/n prompt. `git_worktree` is intercepted before the
    /// generic risky gate and needs its special post-processing, so the approval
    /// resume can't run it via the normal `run_tool` path. The resume instead
    /// sets this id and re-enters `process_tools`, whose git_worktree arm sees the
    /// match, skips re-gating, runs the interception, and clears it. `None`
    /// normally.
    pub approved_worktree_call: Option<String>,
    // --- deferred tool-task lane (parallel to the sub-agent lane below) ---
    /// Tool-call ids of DEFERRED tools (see [`crate::tool::DEFERRED_TOOLS`] — the
    /// heavy/blocking ones: read / write / edit / delete / bash / grep / glob /
    /// remember / web_fetch / web_search) currently running OFF the UI thread.
    /// These tools do blocking I/O (fs reads/writes, a subprocess, a tree walk, or
    /// blocking HTTP), so running them inline on the event-loop thread would freeze
    /// the TUI for the whole call. Instead `process_tools` spawns the work on a
    /// plain `std::thread` and records the call id here; the round PARKS (mirroring
    /// `pending_subagent_calls`) until the background thread sends its result back
    /// over `tool_task_rx`, which the event-loop drain folds into `tool_results`
    /// (removing the id). The round's deferred tools run ONE AT A TIME, so this vec
    /// holds AT MOST ONE id at a time. Empty when no deferred tool is in flight.
    pub pending_tool_tasks: Vec<String>,
    /// True while a tool round is PARKED waiting on a deferred tool task (see
    /// `pending_tool_tasks`). Set by `dispatch_deferred` (or alongside
    /// `awaiting_subagents` for a task-tool park) when `process_tools` returns
    /// without `finish_tool_round`; cleared by the event-loop drain once the
    /// deferred tool has delivered its result, which then resumes the round.
    /// Keeps the busy/shimmer indicator on while parked.
    pub awaiting_tool_tasks: bool,
    /// Receiver for deferred tool-task results: `(tool_call_id, result_string)`.
    /// Lazily created (with `tool_task_tx`) the first time a deferred tool is
    /// dispatched in a session, then reused. Drained each event-loop tick into
    /// `tool_results`. `None` until the first deferred tool runs.
    pub tool_task_rx: Option<UnboundedReceiver<(String, String)>>,
    /// Sender half of the deferred tool-task channel. Cloned into each spawned
    /// tool thread (the sender is `Send`, so it can fire from a non-tokio thread).
    /// Kept here so later deferred tools in the same session reuse the one channel.
    /// `None` until the first deferred tool runs.
    pub tool_task_tx: Option<UnboundedSender<(String, String)>>,
    // --- TAC-classify park lane (parallel to the deferred tool-task lane) ---
    /// True while a risky tool call is PARKED waiting on an off-thread TAC
    /// (tool-call classifier) verdict. Set by `process_tools` when it spawns the
    /// classify task and returns without a verdict rather than freezing the event
    /// loop on `block_on`; cleared by the event-loop drain once the verdict lands
    /// (which stages it in `pending_classify_verdict` and re-enters `process_tools`).
    /// `waiting` stays true across the park so the comet keeps shimmering — this
    /// flag only records WHY the round is parked. Distinct from `awaiting_approval`
    /// (the human y/n prompt), which is a later, separate state.
    pub awaiting_classify: bool,
    /// Sender half of the TAC-classify verdict channel: `(tool_call_id, verdict)`.
    /// Cloned into each spawned classify task (the sender is `Send`, so it can fire
    /// from the tokio task). Lazily created (with `classify_rx`) the first time a
    /// risky tool is classified in a session, then reused. `None` until then.
    pub classify_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, crate::app::harness::Verdict)>>,
    /// Receiver half of the TAC-classify verdict channel. Drained each event-loop
    /// tick: a verdict whose id matches the parked call at `tool_idx` is staged in
    /// `pending_classify_verdict` and the round resumes; any other (stale) delivery
    /// is dropped. `None` until the first risky tool is classified.
    pub classify_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(String, crate::app::harness::Verdict)>>,
    /// The verdict the classifier produced for the parked call, staged by the
    /// event-loop drain for `process_tools` to consume on re-entry: `(tool_call_id,
    /// verdict)`. `process_tools` takes it when the id matches the current call and
    /// acts on it via the SAME three-way branch the old inline `block_on` drove; a
    /// staged verdict for a DIFFERENT id (an interrupted/superseded turn) is dropped
    /// and the call is re-classified. `None` when no verdict is staged.
    pub pending_classify_verdict: Option<(String, crate::app::harness::Verdict)>,
    // --- `!` user-shell lane (off-thread, parallel to the deferred tool lane) ---
    /// True while a `!`-shortcut command is running OFF the UI/event-loop thread
    /// (see `actions::chat::handle_shell`). The `!` shell uses the SAME blocking
    /// `run_shell_capture` primitive the `bash` tool does, so running it inline on
    /// the event-loop thread would freeze the local TUI render loop — or, in the
    /// daemon, the whole event loop for EVERY session — for the command's duration
    /// (the 120s timeout). Instead the work is spawned on a plain `std::thread` and
    /// this latches `true`; the event-loop drain folds the captured output into a
    /// `SHELL_MARK` conversation entry and clears it. Counts as "working"
    /// (`is_working`) so the busy indicator stays on and the self-exit grace timer
    /// treats the session as live; also gates a second `!`/Submit/Resend so a shell
    /// result can never be interleaved into an in-flight or queued turn.
    pub awaiting_shell: bool,
    /// Receiver for `!`-shell results: `(command, captured_output)`. Lazily created
    /// (with `shell_task_tx`) the first time a `!` command runs in a session, then
    /// reused. Drained each event-loop tick. `None` until the first `!` runs.
    pub shell_task_rx: Option<UnboundedReceiver<(String, String)>>,
    /// Sender half of the `!`-shell result channel. Cloned into the spawned shell
    /// thread (the sender is `Send`, so it can fire from a non-tokio thread). Kept
    /// here so later `!` commands in the same session reuse the one channel.
    /// `None` until the first `!` runs.
    pub shell_task_tx: Option<UnboundedSender<(String, String)>>,
    // --- background-bash lane (model `bash` with run_in_background=true) ---
    /// All background bash jobs registered this session (running + finished).
    /// A `bash` call with `run_in_background: true` is intercepted in
    /// `process_tools`, spawned via [`crate::app::bgbash::spawn_bash_job`], and
    /// pushed here; finished jobs STAY in the list so a later `bash_output` poll
    /// can still read their final status + captured output. Addressed by the model
    /// as `bash-<id>` (the id below), never by Vec position.
    pub bash_jobs: Vec<crate::app::bgbash::BashJob>,
    /// Monotonic counter: the id assigned to the NEXT background bash job (starts
    /// at 1, so job ids read as `bash-1`, `bash-2`, …). Never reused.
    pub next_bash_job_id: usize,
    /// Receiver for background-bash COMPLETION signals: the job id of a finished
    /// job. The worker thread fires the id over `bash_done_tx` when its child
    /// exits; the event-loop deferred drain reads it to pop a completion toast.
    /// Lazily created (with `bash_done_tx`) the first time a bg job is spawned in a
    /// session, then reused. `None` until the first bg job runs.
    pub bash_done_rx: Option<UnboundedReceiver<usize>>,
    /// Sender half of the background-bash completion channel. Cloned into each
    /// spawned bg-bash worker thread (the sender is `Send`, so it can fire from a
    /// non-tokio thread). `None` until the first bg job runs.
    pub bash_done_tx: Option<UnboundedSender<usize>>,
    /// Background bash jobs that have finished but whose completion has not yet
    /// been delivered to the model as a nudge. Buffered here while the agent is
    /// busy; drained into ONE injected user turn when the session next goes idle.
    /// Each entry is `(job_id, status_label)`.
    pub pending_bash_nudges: Vec<(usize, String)>,
    /// Detached (`task` `run_in_background`) sub-agents that reached a terminal
    /// state but whose completion has not yet been delivered to the model as a
    /// nudge. Buffered here (mirrors [`pending_bash_nudges`](Self::pending_bash_nudges))
    /// while the agent is busy; drained into ONE injected user turn when the
    /// session next goes idle. Each entry is `(subagent_id, agent_name, full_report)`:
    /// the third element is the FULL outcome/report text (not a short label) and
    /// is injected verbatim into the wake-nudge body so the model receives the
    /// complete result without needing to call task_output.
    pub pending_subagent_nudges: Vec<(usize, String, String)>,
    /// All sub-agents spawned this session (running + finished). Drained each tick
    /// by the event loop; finished ones stay in the list for the UI to show their
    /// final state.
    pub subagents: Vec<SubAgent>,
    /// FIFO queue of delegations accepted while all [`crate::app::subagent::MAX_SUBAGENTS`]
    /// slots were busy. Unlimited length: over-cap delegations ENQUEUE here instead
    /// of being refused. `try_start_pending` (in the event-loop sub-agent drain)
    /// pops the FRONT and spawns it whenever a running sub-agent terminates and a
    /// slot frees, so at most `MAX_SUBAGENTS` ever run at once. Each entry's id is
    /// pre-allocated from `next_subagent_id` at enqueue time (stable `$`-panel row);
    /// a `task`-tool entry's call id is also held in `pending_subagent_calls` so the
    /// parked main turn waits for the queued delegation too.
    pub pending_subagents: VecDeque<PendingSubagent>,
    /// Queued steer messages: user submits made WHILE a turn is cooking. Drained +
    /// coalesced into one user message at the next tool-hop boundary (or auto-sent as
    /// a fresh turn if the turn ends first). Full text kept here; the projection sends
    /// truncated previews to the client. Hard cap 5 (a 6th submit toasts a warning).
    pub pending_steer: Vec<String>,
    /// Tool-call ids of in-flight `task`-tool delegations whose result the main
    /// agent is still waiting for. The model-callable `task` tool DEFERS its tool
    /// result (mirroring the `awaiting_approval` park): `process_tools` pushes the
    /// call id here instead of an immediate "started" result, the round parks, and
    /// the event-loop sub-agent drain delivers the FULL report into `tool_results`
    /// (removing the id) once that sub-agent reaches a terminal state. Empty when
    /// no task delegation is pending. The `/task` slash command path never touches
    /// this (its sub-agents carry `tool_call_id == None`).
    pub pending_subagent_calls: Vec<String>,
    /// True while a tool round is PARKED waiting on one or more deferred
    /// `task`-tool delegations (see `pending_subagent_calls`). Set when
    /// `process_tools` returns without calling `finish_tool_round`; cleared by the
    /// event-loop drain once every pending delegation has filled its result, which
    /// then resumes the round (`finish_tool_round`) so the main agent reacts to the
    /// delegated reports. Keeps the busy/shimmer indicator on while parked.
    pub awaiting_subagents: bool,
    /// Monotonic counter: the id assigned to the NEXT spawned sub-agent.
    #[allow(dead_code)]
    pub next_subagent_id: usize,
    /// LIVE working-directory override for this session, set by the `cd` tool /
    /// the user `/cd` command (Phase 8). `None` means "use the session's
    /// configured workdir" (`Session::workdir()` — the first `settings.workdir`
    /// entry); `Some(dir)` REPOINTS the session's effective cwd to `dir` without
    /// touching the persisted `settings.workdir` list. Like `awareness_summary`
    /// it is purely in-memory and NEVER serialised — a cd is ephemeral per
    /// session run. The effective cwd (this override, else the configured
    /// workdir) feeds `build_tool_ctx`'s `ToolCtx::workspace` (so `bash` runs
    /// there and the dir cache indexes it) and the harness workspace check (so a
    /// `/cd` outside every allowed root makes the next MODEL tool turn WC-denied).
    /// The configured roots in `Session::workdirs()` stay the allow-list / the
    /// `[N]` multi-root set; cd never widens them (use `/adddir` for that).
    pub active_cwd: Option<PathBuf>,
    /// Background-refreshed index of the active session's workspace files
    /// (gitignore-respecting). Re-indexed off-thread; shared with the tool layer.
    pub dir_cache: Arc<RwLock<DirCache>>,
    /// Project-awareness summary (Phase 2): a few-sentence digest of the
    /// project's depth-1 docs, produced by a secondary model at startup and
    /// after `/compact`. Appended to the first System message on every request
    /// (see `runtime::stream::start_stream_task`) so it survives compaction.
    /// `None` when awareness is disabled, no docs exist, or the call failed —
    /// it is recomputed per session, never persisted.
    pub awareness_summary: Option<String>,
    /// Start instant of THIS session's `/compact` animation. `Some` only while a
    /// compaction is in flight for this session (set in `Command::Compact`, cleared
    /// once the result is applied). The renderer reads the FOREGROUND session's value
    /// to draw the spinner + elapsed + indeterminate bar; the event loop reads it both
    /// to keep redrawing each tick (so the animation actually animates) and to enforce
    /// the cosmetic minimum duration. Per-session (C4) so two clients compacting
    /// different sessions can't cross-corrupt each other's apply.
    pub compact_anim_start: Option<Instant>,
    /// Earliest instant THIS session's stashed compaction result may be applied. Set
    /// when a fast `StreamEvent::Compacted` arrives before the minimum animation
    /// duration has elapsed; the event loop applies `compact_pending` once `now >= this`.
    pub compact_apply_at: Option<Instant>,
    /// Stashed `(summary, kept_tail)` for THIS session awaiting the minimum-duration
    /// gate. Held only when a compaction finished faster than the minimum so the apply
    /// is deferred (non-blocking) rather than slept on. Applied by the event loop to
    /// this session by index.
    pub compact_pending: Option<(String, Vec<crate::dto::chat::ChatMessage>)>,
    /// Path of the session whose on-disk `session.lock` THIS instance currently
    /// holds (its active session's directory). `reconcile_session_lock` keeps it
    /// in lock-step with the active session: it releases this lock when switching
    /// away and acquires the new one. The clean-exit teardown in `runtime::run`
    /// removes it; a crash leaves a stale lock that PID-liveness later sweeps.
    pub held_lock: Option<PathBuf>,
    /// Latched true the first time a response reports `cached_tokens > 0`, meaning
    /// the active provider supports and is using a prompt cache. Never reset.
    pub provider_caches: bool,
    /// Sticky engage-state for the cache-warmth-adaptive summarization hysteresis.
    /// Set true when the summarizer engages; a later wave reads and writes it.
    #[allow(dead_code)]
    pub summarizing: bool,
    /// Wall-clock instant of the most-recent send (user turn start). Stamped by
    /// the submit handler in a later wave; used to estimate prompt-cache warmth.
    #[allow(dead_code)]
    pub last_send_at: Option<Instant>,
    /// Working-state from the PREVIOUS event-loop tick, for the background-finish
    /// nudge. The per-session servicer (`service_all_sessions`) records `is_working()`
    /// here at the end of each tick; on the next tick a `was_working && !is_working`
    /// transition for a NON-foreground session fires a "session ready" toast. Starts
    /// `false` so a freshly-created idle session never spuriously nudges.
    pub was_working: bool,
    /// STICKY "this background session finished a turn and nobody has looked at it
    /// since" flag (daemon critique #3). Distinct from the background-finish TOAST,
    /// which is TTL-based and expires on its own — useless when the only client is
    /// DETACHED, since it would lapse before anyone reattaches. This flag instead
    /// LATCHES on the same NON-foreground `working -> ready` edge that raises the
    /// toast (set in `service_all_sessions`) and is carried in `SessionSnapshot` so a
    /// reattaching client still sees the unseen marker. Cleared when a client
    /// foregrounds / views the session (the switch handler in a later stage, or here
    /// the moment this session IS the foreground). Starts `false`.
    pub finished_unseen: bool,
    /// TOMBSTONE marker (daemon stage 10). When a session is closed
    /// (`ClientRequest::QuitSession` or a daemon-side kill-all), it is NOT removed
    /// from [`super::AppStateRest::sessions`] — `service_all_sessions` indexes that
    /// Vec by POSITION ~40x per session per tick, so a `Vec::remove` would shift every
    /// later index and silently cross-wire in-flight async (see `ipc::proto`
    /// critique #2). Instead the slot stays put and this flag latches `true`; the
    /// per-session servicer SKIPS a closed session (no drain, no turn advance, no
    /// nudge) and the self-exit grace timer treats it as quiesced. Never un-set — a
    /// tombstone is permanent for the daemon's lifetime. Starts `false`; the local
    /// TUI never sets it (it has no per-session close).
    pub closed: bool,
    /// PARK-START instant for the detached-approval timeout (daemon stage 11). Set by
    /// the daemon loop to `Some(Instant::now())` the first tick this session is
    /// `awaiting_approval` while NO client is attached — i.e. a risky tool is parked
    /// with no operator present to answer it. Once the elapsed time crosses
    /// `APPROVAL_PARK_TIMEOUT` the loop AUTO-DENIES the pending call(s) (via the shared
    /// `deny_all_pending` path, so the conversation stays API-valid) and clears this
    /// back to `None`. Cleared the moment the park ends for ANY reason — the operator
    /// approves/denies, or a client (re)attaches (an attached client waits for the
    /// operator indefinitely, so the timer must not run while attached). The local TUI
    /// never sets it (it always has its operator on screen); it is purely the daemon's
    /// safety valve against an immortal parked daemon holding a lock with nobody home.
    /// Starts `None`.
    pub park_started_at: Option<Instant>,
    /// Mtime of `MEMORY.md` the last time this session read or wrote it. Used by
    /// the cross-instance memory-sync poll to detect when another koma instance
    /// updated the shared memory store. `None` until first snapshot.
    pub last_memory_mtime: Option<SystemTime>,
    /// PER-SESSION koma-free toggle, flipped by the `/free` command. When `true`,
    /// this session's Main role and the roles that inherit it (Compactor /
    /// Awareness) resolve to the keyless koma-free route regardless of the
    /// configured Main model — see [`crate::app::resolve::resolve_role_free`].
    /// Ephemeral + daemon-owned: NEVER written to config/settings, and a fresh
    /// session always starts `false`. Projected into `SessionSnapshot` so the thin
    /// client can render the `free` status tag off the shadow.
    pub free_mode: bool,
}

impl Default for SessionRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRuntime {
    pub fn new() -> Self {
        Self {
            // Fresh stable id per session. Every construction path (the initial
            // session in `AppStateRest::new` and each `/new` spawn) routes
            // through here, so every session is uniquely keyed automatically.
            id: uuid::Uuid::new_v4().to_string(),
            // Fresh session default (C3): a brand-new live session lands in Chat. The
            // spawn/startup flows (KeyInput on a creds-less spawn, Loading on a warming
            // startup session, SessionPicker on --resume) overwrite this on the RIGHT
            // session after construction.
            mode: Mode::Chat,
            // Per-session status line (C6); same default the old global field carried.
            status: "ready".into(),
            // Per-session toast (C6): none on a fresh session.
            toast: None,
            input: String::new(),
            cursor: 0,
            pending_attachments: Vec::new(),
            hist_idx: None,
            input_stash: String::new(),
            scroll: 0,
            follow: true,
            session: None,
            waiting: false,
            streaming: None,
            stream_reasoning: String::new(),
            stream_reasoning_details: Vec::new(),
            current_task: None,
            active_rx: None,
            harness_rx: None,
            pending_usage: None,
            pending_dispatch_model_id: None,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            tokens_cached: 0,
            pending_tool_calls: Vec::new(),
            agent_steps: 0,
            tool_idx: 0,
            tool_results: Vec::new(),
            awaiting_approval: false,
            approval_reason: None,
            approved_worktree_call: None,
            pending_tool_tasks: Vec::new(),
            awaiting_tool_tasks: false,
            tool_task_rx: None,
            tool_task_tx: None,
            awaiting_classify: false,
            classify_tx: None,
            classify_rx: None,
            pending_classify_verdict: None,
            awaiting_shell: false,
            shell_task_rx: None,
            shell_task_tx: None,
            bash_jobs: Vec::new(),
            next_bash_job_id: 1,
            bash_done_rx: None,
            bash_done_tx: None,
            pending_bash_nudges: Vec::new(),
            pending_subagent_nudges: Vec::new(),
            subagents: Vec::new(),
            pending_subagents: VecDeque::new(),
            pending_steer: Vec::new(),
            pending_subagent_calls: Vec::new(),
            awaiting_subagents: false,
            next_subagent_id: 0,
            active_cwd: None,
            dir_cache: Arc::new(RwLock::new(DirCache::default())),
            awareness_summary: None,
            compact_anim_start: None,
            compact_apply_at: None,
            compact_pending: None,
            held_lock: None,
            provider_caches: false,
            summarizing: false,
            last_send_at: None,
            was_working: false,
            finished_unseen: false,
            closed: false,
            park_started_at: None,
            last_memory_mtime: None,
            // Fresh sessions never start on the koma-free tier; `/free` toggles it.
            free_mode: false,
        }
    }

    /// Streaming lifecycle methods.
    pub fn begin_stream(&mut self) {
        self.streaming = Some(String::new());
        // Arm the parallel reasoning buffer fresh so the previous round's
        // thinking can never bleed into this one.
        self.stream_reasoning.clear();
        // Same for the structured reasoning_details accumulator (replay buffer).
        self.stream_reasoning_details.clear();
    }

    pub fn append_token(&mut self, t: &str) {
        if let Some(buf) = self.streaming.as_mut() {
            buf.push_str(t);
        }
    }

    /// Append a reasoning fragment to the parallel thinking buffer (driven by
    /// `StreamEvent::Reasoning`, mirroring `append_token` for content).
    pub fn append_reasoning(&mut self, t: &str) {
        self.stream_reasoning.push_str(t);
    }

    pub fn take_stream(&mut self) -> Option<String> {
        self.streaming.take()
    }

    /// Take the accumulated reasoning buffer, clearing it. Returns `Some` only
    /// when non-empty so an empty thinking block never attaches to a message.
    /// Always clears (alongside `take_stream`) so reasoning can't leak forward.
    pub fn take_reasoning(&mut self) -> Option<String> {
        if self.stream_reasoning.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.stream_reasoning))
        }
    }

    /// Take the accumulated OpenRouter `reasoning_details`, clearing the buffer.
    /// Returns `Some` only when non-empty. Drained alongside `take_reasoning` at
    /// the tool-call commit so the structured chain-of-thought can be echoed back
    /// on the next request and can never leak into a later round/turn.
    pub fn take_reasoning_details(&mut self) -> Option<Vec<crate::dto::chat::ReasoningDetail>> {
        if self.stream_reasoning_details.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.stream_reasoning_details))
        }
    }

    // ----- toast management (per-session in C6; was `impl AppStateRest`) -----

    /// Show an error toast (red box) for ~6 seconds on THIS session.
    pub fn set_toast(&mut self, msg: String) {
        self.toast = Some((
            msg,
            std::time::Instant::now() + std::time::Duration::from_secs(6),
            ToastKind::Error,
        ));
    }

    /// Show an informational toast (neutral box) for ~8 seconds on THIS session.
    /// Used for non-failure notices like the post-compaction summary, which is
    /// multi-line and shouldn't read as an error.
    pub fn set_toast_info(&mut self, msg: String) {
        self.toast = Some((
            msg,
            std::time::Instant::now() + std::time::Duration::from_secs(8),
            ToastKind::Info,
        ));
    }

    /// Clear THIS session's toast if it has expired. Returns true if it was just
    /// cleared (so the caller can mark the frame dirty). Swept per-session each tick.
    pub fn tick_toast(&mut self) -> bool {
        if let Some((_, until, _)) = &self.toast {
            if std::time::Instant::now() >= *until {
                self.toast = None;
                return true;
            }
        }
        false
    }

    /// Allocate the next background-bash job id, advancing the counter. Ids are
    /// monotonic and never reused, so a finished job's id stays a stable handle
    /// for later `bash_output` polls (the job is kept in `bash_jobs`).
    pub fn next_bash_id(&mut self) -> usize {
        let id = self.next_bash_job_id;
        self.next_bash_job_id += 1;
        id
    }

    // ----- composer editing (the caret `cursor` is a CHAR index into `input`;
    // `byte_at` maps it to the byte offset `String::insert`/`remove` need, so
    // non-ASCII input can never panic on a non-char-boundary). -----

    /// Char count of the current input (the caret's upper bound).
    fn char_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset of char index `idx` (== input length when `idx >= char_len`).
    fn byte_at(&self, idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    /// Insert `c` at the caret and advance it (mid-text editing supported).
    ///
    /// The `palette_sel = 0` / `hist_idx = None` reset for the `/` palette is the
    /// caller's job ([`super::AppStateRest::push_char`] resets the GLOBAL
    /// `palette_sel` after delegating here); this clears only the per-session
    /// `hist_idx`.
    pub fn push_char(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.hist_idx = None;
    }

    /// Delete the char BEFORE the caret and retreat it; no-op at the start.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_at(self.cursor);
        self.input.remove(at);
        self.hist_idx = None;
        // A backspace may have deleted (part of) an `[Image #N]` marker; drop any
        // staged attachment whose marker is now gone so the card can't outlive it.
        self.reconcile_attachments();
    }

    /// Delete the char AT the caret (forward delete, the Delete key); no-op at the
    /// end of the input. Mirrors [`Self::backspace`] but does not move the caret.
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.char_len() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.input.remove(at);
        self.hist_idx = None;
        // Mirror `backspace`: a forward-delete may have removed (part of) an
        // `[Image #N]` marker, so re-sync the staged attachments to the text.
        self.reconcile_attachments();
    }

    /// Move the caret one char left (no-op at the start).
    pub fn cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the caret one char right (capped at the input length).
    pub fn cursor_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_len());
    }

    /// Jump the caret to the start of the input.
    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Jump the caret to the end of the input. Also called after any bulk replace
    /// (history recall, command/file completion) so the caret never dangles past
    /// the new (possibly shorter) text.
    pub fn cursor_end(&mut self) {
        self.cursor = self.char_len();
    }

    /// Move the caret up one visual line within a multi-line input.
    ///
    /// Returns `true` when the caret moved (so the caller can suppress history
    /// recall), or `false` when the caret is already on the first line (single-
    /// line input always returns `false`, preserving the existing history-recall
    /// behaviour).
    pub fn cursor_up(&mut self) -> bool {
        // Walk chars up to cursor to compute (line, col) in char units.
        let mut line: usize = 0;
        let mut col: usize = 0;
        for ch in self.input.chars().take(self.cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if line == 0 {
            return false; // already on the first line → let caller do history
        }
        // Collect char lengths per line (split on '\n').
        let line_lens: Vec<usize> = self.input.split('\n').map(|l| l.chars().count()).collect();
        let target_line = line - 1;
        let target_col = col.min(line_lens[target_line]);
        // Convert (target_line, target_col) back to a flat char index.
        self.cursor = line_lens[..target_line].iter().sum::<usize>()
            + target_line  // one '\n' per consumed line break
            + target_col;
        true
    }

    /// Move the caret down one visual line within a multi-line input.
    ///
    /// Returns `true` when the caret moved, `false` when already on the last
    /// line (single-line input always returns `false`).
    pub fn cursor_down(&mut self) -> bool {
        let line_lens: Vec<usize> = self.input.split('\n').map(|l| l.chars().count()).collect();
        let last_line = line_lens.len() - 1;
        // Walk chars up to cursor to compute (line, col).
        let mut line: usize = 0;
        let mut col: usize = 0;
        for ch in self.input.chars().take(self.cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if line == last_line {
            return false; // already on the last line → let caller do history
        }
        let target_line = line + 1;
        let target_col = col.min(line_lens[target_line]);
        self.cursor = line_lens[..target_line].iter().sum::<usize>()
            + target_line  // one '\n' per consumed line break
            + target_col;
        true
    }

    /// Take the input buffer, resetting the caret + per-session history index.
    /// The GLOBAL `palette_sel` reset is the caller's job (see
    /// [`super::AppStateRest::take_input`]).
    pub fn take_input(&mut self) -> String {
        self.hist_idx = None;
        self.cursor = 0;
        std::mem::take(&mut self.input)
    }

    /// Clear the composer in place (idle double-Esc with text present): empty the
    /// input, park the caret at 0, drop any staged attachments so a marker can't
    /// outlive its image, and leave history recall (`hist_idx = None`). The GLOBAL
    /// `palette_sel` reset, if any, is the caller's job.
    pub fn clear_composer(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.pending_attachments.clear();
        self.hist_idx = None;
    }

    /// Insert the literal marker string `s` (e.g. `"[Image #3]"`) at the caret,
    /// advancing it past the inserted run. Mirrors [`Self::push_char`]'s caret /
    /// history discipline so a bulk marker insert behaves like typing; the GLOBAL
    /// `palette_sel` reset is the caller's job (see
    /// [`super::AppStateRest::insert_marker`]).
    pub fn insert_marker(&mut self, s: &str) {
        let at = self.byte_at(self.cursor);
        self.input.insert_str(at, s);
        self.cursor += s.chars().count();
        self.hist_idx = None;
    }

    /// Re-sync `pending_attachments` to the `[Image #N]` markers still present in
    /// `input`: drop any staged attachment whose marker number no longer appears
    /// in the composer text. Called from the char-removal paths (`backspace` /
    /// `delete_forward`) so deleting an `[Image #N]` token live-drops its
    /// attachment card.
    ///
    /// Deliberately NOT called from insert paths (`push_char` / `insert_marker` /
    /// the attach helpers): an image's [`Attachment`] record is pushed a beat
    /// AFTER its marker is inserted, so reconciling on insert could drop a
    /// just-staged image before its record lands.
    pub fn reconcile_attachments(&mut self) {
        if self.pending_attachments.is_empty() {
            return; // nothing staged → nothing to drop (skips the scan on every keystroke)
        }
        let present = Self::marker_numbers(&self.input);
        self.pending_attachments
            .retain(|a| present.contains(&a.marker_n));
    }

    /// Collect the set of `N` values from every literal `[Image #N]` token in
    /// `text` — the exact format produced by `model::attachment`
    /// (`format!("[Image #{n}]")`). A run of ASCII digits sitting between the
    /// `[Image #` prefix and a closing `]` counts; a broken / half-typed marker
    /// matches nothing, so its attachment reconciles away.
    fn marker_numbers(text: &str) -> std::collections::HashSet<usize> {
        const PREFIX: &str = "[Image #";
        let mut out = std::collections::HashSet::new();
        for (i, _) in text.match_indices(PREFIX) {
            let after_prefix = &text[i + PREFIX.len()..];
            let digits: String = after_prefix
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.is_empty() || !after_prefix[digits.len()..].starts_with(']') {
                continue;
            }
            if let Ok(n) = digits.parse::<usize>() {
                out.insert(n);
            }
        }
        out
    }

    /// Move the staged composer attachments out for the message being submitted,
    /// leaving `pending_attachments` empty. Called at submit, paired with
    /// `take_input()`, so the markers and their attachment records travel
    /// together onto the user message.
    ///
    /// `submitted_text` is the text being sent (the composer `input` has already
    /// been emptied by `take_input` at this point, so we reconcile against the
    /// SUBMITTED text, not `self.input`). This is the send-time backstop: an
    /// attachment whose `[Image #N]` marker did not survive into the sent message
    /// — a marker broken by mid-token typing, or dropped by a history-recall
    /// replace — is discarded here so a marker-less image can never ship.
    pub fn take_attachments(&mut self, submitted_text: &str) -> Vec<crate::dto::chat::Attachment> {
        let present = Self::marker_numbers(submitted_text);
        self.pending_attachments
            .retain(|a| present.contains(&a.marker_n));
        std::mem::take(&mut self.pending_attachments)
    }

    /// Recall the previous (older) sent user message into the input. `users` is
    /// the session's user messages oldest-first.
    pub fn history_prev(&mut self, users: &[String]) {
        if users.is_empty() {
            return;
        }
        let next = match self.hist_idx {
            None => {
                self.input_stash = self.input.clone();
                users.len() - 1
            }
            Some(0) => return, // already at the oldest
            Some(i) => i - 1,
        };
        self.hist_idx = Some(next);
        self.input = users[next].clone();
        self.cursor = self.char_len();
    }

    /// Recall the next (newer) sent user message; past the newest, restore the
    /// stashed live input and leave recall mode.
    pub fn history_next(&mut self, users: &[String]) {
        match self.hist_idx {
            Some(i) if i + 1 < users.len() => {
                self.hist_idx = Some(i + 1);
                self.input = users[i + 1].clone();
                self.cursor = self.char_len();
            }
            Some(_) => {
                self.hist_idx = None;
                self.input = std::mem::take(&mut self.input_stash);
                self.cursor = self.char_len();
            }
            None => {}
        }
    }

    /// Kill running sub-agents that belong to THIS session, drop model-delegated
    /// queued sub-agents, but PRESERVE user-initiated /task jobs
    /// (tool_call_id == None).
    ///
    /// `include_detached` controls whether detached background sub-agents
    /// (`sub.detached == true`) are killed along with blocking ones:
    ///
    /// - `false` (turn-halt / Esc / deny): SKIP detached agents — they are
    ///   background jobs the user explicitly asked to run independently, and they
    ///   must survive an Esc/interrupt just as bg-bash jobs do (bg-bash: `bash_jobs`
    ///   are never touched by `interrupt()`, only dropped with the session on
    ///   `close()`). Only blocking (non-detached) Running sub-agents are killed.
    ///   `pending_subagent_nudges` is left INTACT because that buffer is exclusively
    ///   fed by detached agents; clearing it would drop a completion notification
    ///   that arrived just before the interrupt.
    ///
    /// - `true` (session close / tombstone): kill ALL Running sub-agents including
    ///   detached ones — the session is going away entirely, nothing survives.
    ///   `pending_subagent_nudges` is cleared (the session can no longer fire them).
    ///
    /// - Running (non-detached) sub-agents: `abort.abort()` kills the tokio task;
    ///   status is flipped to `Killed` immediately so the `$` panel reflects it
    ///   without waiting for a terminal event that will never arrive.
    /// - Model-delegated queued sub-agents (tool_call_id == Some): dropped to
    ///   halt the interrupted turn's work (always, regardless of `include_detached`).
    /// - User-initiated /task entries (tool_call_id == None): retained so the
    ///   user's independent pending commands survive the turn halt.
    /// - `pending_subagent_calls` / `awaiting_subagents`: cleared here so the
    ///   caller does NOT need to do it separately (keeps the halt paths consistent).
    ///
    /// This method ONLY touches the session it is called on — it is always
    /// invoked via `state.rest.fg_mut()` (or a named session slot), so other
    /// sessions are not affected.
    pub fn abort_running_subagents(&mut self, include_detached: bool) {
        for sub in &mut self.subagents {
            if matches!(sub.status, crate::app::subagent::SubAgentStatus::Running) {
                // When NOT including detached agents, skip detached ones so they
                // keep running independently (mirrors bg-bash surviving Esc).
                if !include_detached && sub.detached {
                    continue;
                }
                sub.abort.abort();
                sub.status = crate::app::subagent::SubAgentStatus::Killed;
                // Suppress the detached-completion nudge for an agent killed here:
                // the user stopped this turn (or the session closed), so the session
                // must NOT auto-wake to announce "your background agent finished".
                // Latching `nudged` stops the next-tick terminal-fold in
                // `drain_subagents` from buffering a nudge for it.
                sub.nudged = true;
            }
        }
        self.pending_subagents.retain(|p| p.tool_call_id.is_none());
        self.pending_subagent_calls.clear();
        self.awaiting_subagents = false;
        // `pending_subagent_nudges` is exclusively fed by DETACHED agents. When
        // `include_detached == false` (turn-halt/Esc), preserved detached agents
        // may have already buffered a nudge that arrived just before the interrupt —
        // keep it so the session auto-wakes to announce completion when next idle.
        // When `include_detached == true` (session close), clear it: the session is
        // going away and can no longer fire the nudge.
        if include_detached {
            self.pending_subagent_nudges.clear();
        }
    }

    /// TOMBSTONE this session (daemon stage 10): tear down ALL of its in-flight work
    /// and latch [`closed`](Self::closed) so the per-session servicer skips it from
    /// now on, WITHOUT removing it from the sessions Vec (a `Vec::remove` would shift
    /// every later index and cross-wire index-routed async — see `ipc::proto`
    /// critique #2). After this returns the slot is inert: `is_working()` is false,
    /// no receiver is live, no lock is held.
    ///
    /// Steps (a superset of `abort_current` + `abort_running_subagents`, applied to
    /// THIS session rather than the foreground):
    /// - abort the in-flight stream task + drop its receiver (late events vanish),
    /// - drop the advisory prompt-classifier channel,
    /// - abort every running sub-agent + drop queued model delegations,
    /// - clear `waiting` and the parked-lane flags so nothing reads as busy,
    /// - RELEASE this session's on-disk `session.lock` (so a closed session frees its
    ///   lock immediately, not only at daemon teardown — another process may reopen
    ///   it); the path is unlinked here and dropped from `held_lock`.
    ///
    /// Idempotent: closing an already-closed session is a harmless no-op (everything
    /// is already torn down). Does NOT touch foreground — the caller repoints
    /// foreground off a tombstone (only it knows the session set).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        // In-flight stream task: abort + drop the receiver so late events vanish.
        if let Some(h) = self.current_task.take() {
            h.abort();
        }
        self.active_rx = None;
        self.harness_rx = None;
        self.waiting = false;
        self.awaiting_approval = false;
        self.approval_reason = None;
        // Drop any detached-park timer (daemon stage 11) — a tombstone is never
        // awaiting, and the loop only inspects non-closed sessions, so this just
        // keeps the inert slot fully clean.
        self.park_started_at = None;
        self.awaiting_tool_tasks = false;
        // Drop any TAC-classify park too (a tombstone is never awaiting); a late
        // verdict to this inert slot is discarded because the servicer skips a
        // closed session's drain.
        self.awaiting_classify = false;
        self.pending_classify_verdict = None;
        // A `!` shell may be draining off-thread; clear the park flag so a late
        // delivery to this tombstone is discarded by the gated drain (the OS child
        // finishes on its own — we never block close() on it).
        self.awaiting_shell = false;
        // Sub-agents: kill running, drop model-delegated queued work, clear the
        // parked-delegation bookkeeping. (Unlike a turn-halt, a CLOSE also drops
        // user-initiated /task entries — the session is going away entirely.)
        // include_detached = true: session is gone, all background agents die with it.
        self.abort_running_subagents(true);
        self.pending_subagents.clear();
        // Release this session's on-disk lock right away (unlink + forget the path).
        if let Some(path) = self.held_lock.take() {
            crate::model::store::remove_lock(&path);
        }
        self.closed = true;
    }

    /// Stop this session's in-flight turn WITHOUT tombstoning it: abort the
    /// stream task + sub-agents, drop all parked agentic state, and commit any
    /// partial assistant buffer with an `[interrupted]` marker. Idempotent and
    /// safe on an idle session (nothing in flight → no-op commit). Used both by
    /// the foreground Esc-interrupt and by the session hub's Ctrl+X "stop".
    ///
    /// This is the per-session half of the old `handle_interrupt`: every step here
    /// operated on `fg_mut()` before, so it works on ANY session now. The partial
    /// buffer is committed to THIS session's own `session` (path/conversation/log),
    /// and only THIS session's counters are touched. The rest-GLOBAL compaction
    /// cleanup + status line stay with the caller (`actions::chat::handle_interrupt`).
    pub fn interrupt(&mut self) {
        // Abort the in-flight stream task + stop listening to it (the per-session
        // part of `abort_current`): abort the handle, drop the active receiver so
        // any late events from the aborted task vanish, and clear `waiting`.
        if let Some(h) = self.current_task.take() {
            h.abort();
        }
        self.active_rx = None;
        self.waiting = false;
        // Halt the agentic loop: drop any stashed tool calls, reset the step
        // counter, and clear the approval machine so a halt mid-approval doesn't
        // leave the turn wedged.
        self.pending_tool_calls.clear();
        self.agent_steps = 0;
        self.awaiting_approval = false;
        self.approval_reason = None;
        self.tool_idx = 0;
        self.tool_results.clear();
        // Kill every BLOCKING running sub-agent spawned by this turn and drop the
        // pending queue. `abort_running_subagents` also clears
        // `pending_subagent_calls` and `awaiting_subagents`, so the halt path is
        // complete. Detached (background) sub-agents are PRESERVED — they are the
        // user's independent background jobs and survive Esc exactly as bg-bash
        // jobs do (bash_jobs is never touched by interrupt()).
        // include_detached = false: Esc/turn-halt, preserve background agents.
        self.abort_running_subagents(false);
        // Abandon any round parked on a deferred tool task. The off-thread worker
        // keeps running but its result lands with no matching pending id, so the
        // next-turn machine reset discards it; it can't resume a turn that was
        // killed. The channel itself is left intact for reuse by later deferred
        // tools. We deliberately do NOT join the worker here.
        self.pending_tool_tasks.clear();
        self.awaiting_tool_tasks = false;
        // Abandon any round parked on the TAC classifier the same way: the
        // off-thread classify task keeps running but its verdict lands with no
        // matching parked id (pending_tool_calls is cleared above), so the drain
        // drops it. Channel ends are left intact for reuse (mirrors the tool-task
        // lane); a stale staged verdict is cleared so it can't be mis-consumed.
        self.awaiting_classify = false;
        self.pending_classify_verdict = None;
        // Take any captured usage unconditionally so a partial turn's usage can't
        // leak into the next response.
        let usage = self.pending_usage.take();
        // Likewise drain the reasoning buffer unconditionally so a half-streamed
        // thinking block can't bleed into the next turn; it's folded onto the
        // interrupted message (display-only).
        let reasoning = self.take_reasoning();
        self.stream_reasoning_details.clear();
        let buf = self.take_stream();
        if let Some(b) = buf {
            if !b.is_empty() {
                let mut committed = false;
                if let Some(sess) = self.session.as_mut() {
                    let content = format!("{b}  [interrupted]");
                    let _ = crate::model::msglog::append(
                        &sess.path,
                        crate::dto::chat::Role::Assistant,
                        &content,
                        usage,
                    );
                    sess.conversation.push_assistant(content, reasoning);
                    let _ = sess.save();
                    committed = true;
                }
                // Update THIS session's own counters once the `sess` borrow above
                // has ended (mirrors the foreground-interrupt accounting).
                if committed {
                    if let Some((pt, ct, cost)) = usage {
                        self.tokens_in = pt; // current context size, not a sum
                        self.tokens_out += ct;
                        self.cost += cost;
                    }
                }
            }
        }
    }

    /// True when this session has work in flight: a turn waiting / streaming, a
    /// paused approval, a parked deferred lane (tool tasks or sub-agent
    /// delegations), or any still-running sub-agent. Used by the session hub's
    /// cooking pane to flag busy sessions, by the foreground status line, and by
    /// the background-finish nudge.
    ///
    /// A CLOSED (tombstoned) session is NEVER working: `close()` already tore down
    /// every lane, but short-circuit here so a stray flag can't keep a tombstone
    /// reading as busy (the self-exit grace timer treats `!is_working()` as quiesced).
    pub fn is_working(&self) -> bool {
        if self.closed {
            return false;
        }
        self.waiting
            || self.streaming.is_some()
            || self.awaiting_approval
            || self.awaiting_tool_tasks
            || self.awaiting_classify
            || self.awaiting_shell
            || self.awaiting_subagents
            || self
                .subagents
                .iter()
                .any(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running))
    }

    /// UI-activity twin of [`is_working`](Self::is_working): identical busy-flag set,
    /// except a DETACHED (backgrounded) sub-agent does NOT count. A detached agent
    /// runs off to the side while the main turn is idle, so the UI must read 'ready'.
    ///
    /// The split exists because `is_working` serves two masters that need different
    /// answers for detached agents:
    /// - LIVENESS (daemon quiescence / self-exit grace / hub-kill abort-vs-close /
    ///   quit warning / completion-nudge gating) — a detached agent MUST count, else
    ///   the daemon could self-exit and kill the running background agent. That is
    ///   `is_working`; leave those callers on it.
    /// - UI ACTIVITY (projected `working` → client `waiting` → comet shimmer + fast
    ///   redraw cadence, the session-hub ●/○ marker, the foreground status line) — a
    ///   detached agent must NOT count. Those callers read this.
    pub fn is_ui_busy(&self) -> bool {
        if self.closed {
            return false;
        }
        self.waiting
            || self.streaming.is_some()
            || self.awaiting_approval
            || self.awaiting_tool_tasks
            || self.awaiting_classify
            || self.awaiting_shell
            || self.awaiting_subagents
            || self.subagents.iter().any(|s| {
                matches!(s.status, crate::app::subagent::SubAgentStatus::Running) && !s.detached
            })
    }

    /// True once this session has been tombstoned via [`close()`](Self::close) —
    /// its slot stays in `sessions` (so no index shifts) but it is inert. Read by
    /// the session-hub cooking builder (a closed session must not reappear) and by
    /// the kill handler's foreground reassignment (never repoint onto a tombstone).
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// This session's EFFECTIVE working directory: the live `cd` override
    /// ([`active_cwd`](Self::active_cwd)) when set, else the session's configured
    /// workdir (`Session::workdir()` — the first `settings.workdir` entry), else
    /// the process cwd when there is no session at all.
    ///
    /// The single source of truth for "where this session is right now". Read by
    /// `build_tool_ctx` (→ `ToolCtx::workspace`, so `bash` + the dir cache follow
    /// `cd`), by the harness workspace check (so a `cd` outside every allowed root
    /// blocks the next MODEL tool turn), and by the IPC snapshot. The configured
    /// allow-list / `[N]` roots in `Session::workdirs()` are deliberately NOT
    /// affected — cd moves only the cwd.
    pub fn effective_cwd(&self) -> PathBuf {
        if let Some(cwd) = self.active_cwd.as_ref() {
            return cwd.clone();
        }
        self.session
            .as_ref()
            .map(|s| s.workdir())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}
