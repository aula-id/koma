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
//!
//! Split into sibling modules for size: [`composer`] carries the caret/text
//! editing methods, [`attach`] carries the staged-attachment + input-history
//! recall clusters, [`lifecycle`] carries the sub-agent-teardown / tombstone-
//! close / turn-halt-interrupt / busy-predicate / effective-cwd cluster. All
//! are `impl SessionRuntime` blocks on the SAME type defined here — no
//! behaviour or visibility change from the pre-split single-file layout.

mod attach;
mod composer;
mod lifecycle;

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

/// An SDLC edit batch awaiting historian summary.
#[derive(Debug, Clone)]
pub struct HistorianBatch {
    pub batch_id: String,
    pub node_id: Option<String>,
    pub mission_goal: String,
    pub mission_phase: String,
    pub node_title: Option<String>,
    pub paths: Vec<String>,
}

/// Consecutive extension-injected-turn budget (cost-DoS guard, review finding — see
/// [`SessionRuntime::ext_injected_turns`]): the max number of synthetic `chat.prompt`
/// turns the deferred-drain injection gate
/// (`app::runtime::event_loop::sessions::deferred`) will fire back-to-back, and the
/// ceiling `broker_chat_prompt` (`app::ext::broker`) refuses to buffer past, before a
/// REAL user turn resets the counter. LOAD-BEARING — without this ceiling an
/// extension subscribed to `agent.turn_end` that re-prompts with varying text
/// (defeating the broker's consecutive-duplicate dedupe) could keep the session
/// spending API calls indefinitely while the user is away. Enforced in TWO places
/// belt-and-braces: the broker refuses to even BUFFER a new prompt once at/over
/// budget, and the deferred injection gate additionally refuses to INJECT an
/// already-buffered one — so a prompt buffered just before the budget tripped stays
/// parked, never silently dropped. Defined here (next to the counter it bounds) and
/// re-exported through `app::state` so both consumers share the ONE constant.
pub const EXT_TURN_BUDGET: u32 = 10;

/// Currently loaded skill: the body injected into context, plus an optional
/// `skill_dir` for dir-form skills (`bar/SKILL.md` sets `Some(bar/)`; flat
/// `foo.md` sets `None`). `skill_dir` is used at load time to list companion
/// files and later to grant read-only access through `resolve_read`.
#[derive(Debug, Clone)]
pub struct ActiveSkill {
    /// Markdown body injected into the volatile system tail.
    pub body: String,
    /// Dir-form only: absolute path to the skill's parent directory.
    /// `None` for flat skills.
    pub skill_dir: Option<PathBuf>,
}

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
    /// The endpoint actually DISPATCHED for the in-flight request, captured
    /// alongside `pending_dispatch_model_id` at the same dispatch-time
    /// snapshot in `stream::run::start_stream_task`. Used by the usage-ledger
    /// write (W3) to look up curated per-1M-token pricing in the catalogue
    /// overlay when a provider reports cost as 0.0 (Codex/Claude hardcode it;
    /// direct APIs like DeepSeek may omit it entirely). `None` only before the
    /// very first send of a session.
    pub pending_dispatch_endpoint: Option<String>,
    /// THIS session's cumulative token/cost totals (seeded on open via
    /// `load_token_totals` from msglog + the global usage ledger — the ledger
    /// is preferred for cost/tokens_out because it includes every per-step
    /// `sub:*` row, then incremented live per main + sub-agent response).
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
    /// Consecutive main-chat stall nudges in the current turn (budget 2). Bumped
    /// when the model returns no tools but text looks like a cliffhanger
    /// ("Let me…"); reset on real end-of-turn, user submit, or stream error.
    pub main_stall_nudges: u8,
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
    /// The plan text (truncated) the user most recently APPROVED for execution, or
    /// `None` when no plan is executing. Set by the plan-approval handlers
    /// (`handle_approve_plan` / `handle_approve_plan_compact`) and PREPENDED to the
    /// tool-call classifier's (TAC) conversation context in `process_tools`, so the
    /// classifier — which keeps running as the safety net — is TOLD the plan was
    /// approved and ALLOWS the tool calls that carry it out, flagging only genuinely
    /// off-plan / destructive actions. Cleared on the next genuine user submit and on
    /// (re)entering Plan mode, so it never leaks past the plan's execution window.
    pub approved_plan: Option<String>,
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
    pub classify_tx:
        Option<tokio::sync::mpsc::UnboundedSender<(String, crate::app::harness::Verdict)>>,
    /// Receiver half of the TAC-classify verdict channel. Drained each event-loop
    /// tick: a verdict whose id matches the parked call at `tool_idx` is staged in
    /// `pending_classify_verdict` and the round resumes; any other (stale) delivery
    /// is dropped. `None` until the first risky tool is classified.
    pub classify_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<(String, crate::app::harness::Verdict)>>,
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
    /// Extension-buffered `chat.prompt` texts awaiting injection as a synthetic
    /// user turn, each `(ext_id, text)`. Filled by the grant broker's
    /// `chat.prompt` arm (buffer-only — it NEVER injects) and drained into ONE
    /// injected user turn the next time this session goes idle (see the event-loop
    /// `deferred` drain), mirroring [`pending_bash_nudges`](Self::pending_bash_nudges)
    /// / [`pending_subagent_nudges`](Self::pending_subagent_nudges). Hard-capped at
    /// 5 by the broker with consecutive-duplicate dedupe, so a `turn_end`-subscribed
    /// extension that re-prompts can neither amplify into a runaway loop nor flood
    /// the buffer. Purely in-memory / transient — `SessionRuntime` is rebuilt fresh
    /// each launch (it is never serialised), so this is never persisted.
    pub pending_ext_prompts: Vec<(String, String)>,
    /// THIS session's tool-approval / lifecycle mode (per-session, not global).
    /// Shift+Tab and `/mode` mutate the FOREGROUND session via
    /// [`super::AppStateRest::set_agent_mode`]; stream/harness paths that already
    /// know a `sess_idx` must read `sessions[sess_idx].agent_mode` so a
    /// background session never inherits another session's SDLC/Plan/Yolo envelope.
    pub agent_mode: super::types::AgentMode,
    /// Mode to restore when THIS session leaves `Plan`.
    pub plan_return_mode: Option<super::types::AgentMode>,
    /// Mode to restore when THIS session leaves SDLC.
    pub sdlc_return_mode: Option<super::types::AgentMode>,
    /// Prior `short_send_enabled` before THIS session forced it on for SDLC.
    pub sdlc_prev_short_send: Option<bool>,
    /// Active mission phase for THIS session: assess | execute | integrate | done.
    pub sdlc_phase: Option<String>,
    /// Mission branch (intent or bound) for header/projection. Transient — not serialised.
    pub sdlc_branch: Option<String>,
    /// Primary branch captured once on SDLC enter; restored on leave/deny if clean.
    /// Transient — never serialised.
    pub sdlc_assess_entry_branch: Option<String>,
    /// One-shot: after mission-approval compact, seed the mission capsule on THIS session.
    pub pending_mission_seed: bool,
    /// One-shot: after plan-approval compact, seed plan.md on THIS session.
    pub pending_plan_seed: bool,
    /// SDLC keeper due-flag: set after mission approve and after each finished
    /// tool round while in SDLC. Deferred idle rail evaluates once then clears.
    /// Transient — never serialised.
    pub sdlc_keeper_due: bool,
    /// Optional LLM keeper inject staged by async Safeguard oneshot; drained on idle.
    pub pending_sdlc_keeper_llm: Option<String>,
    /// True while an async SDLC LLM-keeper classify is in flight (dedupe spawns).
    pub sdlc_keeper_llm_inflight: bool,
    /// Receiver for the async SDLC LLM-keeper classify result.
    /// Payload is `(epoch_at_spawn, inject)` so drain can drop stale results.
    pub sdlc_keeper_llm_rx: Option<tokio::sync::oneshot::Receiver<(u64, Option<String>)>>,
    /// Monotonic epoch bumped whenever in-flight LLM keeper results must be
    /// cancelled/ignored (SDLC exit, phase change, contract-hash change).
    pub sdlc_keeper_epoch: u64,
    /// Best-effort SDLC historian: audit batch awaiting summary, if any.
    pub pending_sdlc_historian_batch: Option<HistorianBatch>,
    /// True while an async SDLC historian summary is in flight.
    pub sdlc_historian_inflight: bool,
    /// Receiver for async SDLC historian summary result.
    pub sdlc_historian_rx: Option<tokio::sync::oneshot::Receiver<(u64, Option<String>)>>,
    /// Epoch bumped on SDLC exit/phase change to drop stale historian results.
    pub sdlc_historian_epoch: u64,
    /// Session active SDLC card (claimed leaf). Transient — never serialised.
    ///
    /// - Set on successful `claim_leaf` (task.node_id or checklist in_progress).
    /// - Kept after task spawn so main-path ownership can resolve the claim.
    /// - Cleared on mission_verify PASS when sealed node == pending,
    ///   leave SDLC, deny, or mission_ready amend/park.
    /// - Second claim still denied by `claim_leaf` exclusivity.
    pub sdlc_pending_node_id: Option<String>,
    /// Consecutive EXTENSION-injected turn counter (cost-DoS guard, review finding):
    /// the number of synthetic user turns injected back-to-back by the `chat.prompt`
    /// broker path (see [`EXT_TURN_BUDGET`]) SINCE the last REAL user turn.
    /// Incremented AFTER a successful ext-prompt injection kickoff (the deferred-drain
    /// gate in `app::runtime::event_loop::sessions::deferred`); reset to `0` the
    /// moment a genuine user submit is accepted (`actions::chat::handle_submit`, both
    /// the immediate-kickoff and the queued-steer paths) — NEVER by a synthetic
    /// kickoff, since bash/subagent/ext-nudge auto-wakes call `start_stream_task`
    /// directly and bypass `handle_submit` entirely. Bounds an extension subscribed to
    /// `agent.turn_end` that keeps re-prompting with varying text (defeating the
    /// broker's consecutive-duplicate dedupe) from spending unbounded API cost while
    /// the user is away: once this reaches [`EXT_TURN_BUDGET`], both the injection
    /// gate and the `chat.prompt` broker refuse further turns until a real user turn
    /// resets it. Purely transient (never serialised), like `pending_ext_prompts`.
    pub ext_injected_turns: u32,
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
    /// CUMULATIVE file-change log for THIS session (#24): every workspace file the
    /// `write` / `edit` / `delete` tools touched, with its latest status
    /// (added/modified/deleted, dedup by path). The fs tools record each op
    /// event-driven into the per-session `messages.sqlite` (durable + survives
    /// `/compact`); this in-memory mirror is refreshed from that store at
    /// `finish_tool_round` + on session load, and projected into the GUI Explore
    /// "File changed" panel. Read-only for the TUI (it has no such panel). Never
    /// a git-status snapshot — it is what this session itself changed.
    pub file_changes: Vec<crate::model::msglog::FileChange>,
    /// THIS session's Plan-mode todo checklist, mirroring `plan_todos.md` on disk
    /// (empty outside Plan mode / when no plan is in progress). Refreshed
    /// in-memory at every mutation site — `set_agent_mode`'s enter/leave-Plan rail
    /// seed/clear, the `checklist` interception, and `plan_ready`'s rail-completion
    /// write — and on session load (mirrors `file_changes`'s refresh pattern).
    /// INCLUDES the two locked workflow rails ("serve plan to user"/"save plan to
    /// file & prompt approval") — they are filtered out at the snapshot projection
    /// boundary (`ipc::snapshot::projection::core`), not here, since they are
    /// internal bookkeeping rather than user-facing plan content (mirrors the
    /// `plan_ready` digest's own `!it.locked` filter). Projected into the GUI
    /// Explore "PLAN" section; the TUI's `/todo` overlay reads `plan_todos.md`
    /// directly and ignores this mirror.
    pub plan_todos: Vec<crate::app::mode::todo::TodoItem>,
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
    /// Cached graph summary text for L1 injection into the system prompt.
    /// Populated by the linker daemon on scan complete / generation change.
    pub graph_summary: Option<String>,
    /// Generation counter of the last graph summary we fetched.
    pub graph_generation: u64,
    /// Currently loaded skill bodies (name → [`ActiveSkill`]). Injected into the
    /// volatile system tail each request. Ephemeral, never persisted.
    pub active_skills: std::collections::BTreeMap<String, ActiveSkill>,
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
    /// Active image viewer overlay. `Some` when the viewer is open.
    pub image_overlay: Option<crate::app::mode::ImageOverlayState>,
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
            pending_dispatch_endpoint: None,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            tokens_cached: 0,
            pending_tool_calls: Vec::new(),
            agent_steps: 0,
            main_stall_nudges: 0,
            tool_idx: 0,
            tool_results: Vec::new(),
            awaiting_approval: false,
            approval_reason: None,
            approved_worktree_call: None,
            approved_plan: None,
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
            pending_ext_prompts: Vec::new(),
            agent_mode: super::types::AgentMode::default(),
            plan_return_mode: None,
            sdlc_return_mode: None,
            sdlc_prev_short_send: None,
            sdlc_phase: None,
            sdlc_branch: None,
            sdlc_assess_entry_branch: None,
            pending_mission_seed: false,
            pending_plan_seed: false,
            sdlc_keeper_due: false,
            pending_sdlc_keeper_llm: None,
            sdlc_keeper_llm_inflight: false,
            sdlc_keeper_llm_rx: None,
            sdlc_keeper_epoch: 0,
            pending_sdlc_historian_batch: None,
            sdlc_historian_inflight: false,
            sdlc_historian_rx: None,
            sdlc_historian_epoch: 0,
            sdlc_pending_node_id: None,
            ext_injected_turns: 0,
            subagents: Vec::new(),
            pending_subagents: VecDeque::new(),
            pending_steer: Vec::new(),
            pending_subagent_calls: Vec::new(),
            awaiting_subagents: false,
            next_subagent_id: 0,
            file_changes: Vec::new(),
            plan_todos: Vec::new(),
            active_cwd: None,
            dir_cache: Arc::new(RwLock::new(DirCache::default())),
            awareness_summary: None,
            graph_summary: None,
            graph_generation: 0,
            active_skills: std::collections::BTreeMap::new(),
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
            image_overlay: None,
        }
    }

    /// Update the cached graph summary if the generation changed or the text
    /// differs. Called by the warm path (L1) after a linker summary fetch.
    pub fn update_graph_summary(&mut self, summary: String, generation: u64) {
        if generation > self.graph_generation
            || summary != self.graph_summary.as_deref().unwrap_or_default()
        {
            self.graph_summary = Some(summary);
            self.graph_generation = generation;
        }
    }

    /// Cancel/ignore any in-flight or staged SDLC LLM-keeper result so a stale
    /// oneshot cannot inject or start a non-SDLC (or wrong-phase) turn.
    /// Bumps [`Self::sdlc_keeper_epoch`]; drops the receiver (task result is
    /// ignored on drop); clears staged inject + inflight + due flags.
    pub fn invalidate_sdlc_keeper_llm(&mut self) {
        self.sdlc_keeper_epoch = self.sdlc_keeper_epoch.wrapping_add(1);
        self.sdlc_keeper_llm_rx = None;
        self.sdlc_keeper_llm_inflight = false;
        self.pending_sdlc_keeper_llm = None;
        self.sdlc_keeper_due = false;
        // Stale historian results are invalid on SDLC exit / phase change.
        self.sdlc_historian_epoch = self.sdlc_historian_epoch.saturating_add(1);
        self.sdlc_historian_rx = None;
        self.sdlc_historian_inflight = false;
        self.pending_sdlc_historian_batch = None;
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

    /// Inject a follow-up user `message` into the sub-agent with local `id` in
    /// this session — the SINGLE shared core of the broker `agents.send` verb and
    /// the main-agent `task_send` tool, so the two surfaces steer identically.
    ///
    /// - RUNNING → hand it to the agent's injection channel (the engine folds it
    ///   into history + the viewer transcript at its NEXT turn boundary) →
    ///   [`Sent`](crate::app::subagent::InjectOutcome::Sent). Best-effort: a
    ///   Running agent whose loop just ended has a closed receiver, so the send is
    ///   dropped and the next `drain_subagents` tick settles it — but the caller
    ///   still reports success, since status is the source of truth.
    /// - QUEUED (still in `pending_subagents`, not yet started) → stash it on the
    ///   pending record for delivery at promotion →
    ///   [`Queued`](crate::app::subagent::InjectOutcome::Queued).
    /// - TERMINAL (done/killed/error) →
    ///   [`Terminal`](crate::app::subagent::InjectOutcome::Terminal) (nothing sent).
    /// - no such id →
    ///   [`Unknown`](crate::app::subagent::InjectOutcome::Unknown).
    ///
    /// Callers validate `message` non-empty before calling.
    pub fn inject_into_subagent(
        &mut self,
        id: usize,
        message: String,
    ) -> crate::app::subagent::InjectOutcome {
        use crate::app::subagent::{InjectOutcome, SubAgentStatus};
        // A live (running or already-terminal) record takes precedence over a
        // queued one — ids are unique across the two, but check running first so a
        // just-promoted agent is steered live, not re-stashed.
        if let Some(sa) = self.subagents.iter().find(|s| s.id == id) {
            if matches!(sa.status, SubAgentStatus::Running) {
                let _ = sa.inject_tx.send(message);
                return InjectOutcome::Sent;
            }
            return InjectOutcome::Terminal;
        }
        if let Some(p) = self.pending_subagents.iter_mut().find(|p| p.id == id) {
            p.pending_injects.push(message);
            return InjectOutcome::Queued;
        }
        InjectOutcome::Unknown
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
}
