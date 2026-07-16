//! Self-contained sub-agent runtime: run a defined agent as an autonomous
//! LLM-tool loop in a background tokio task.
//!
//! A sub-agent is an [`AgentDef`](crate::model::agent_def::AgentDef) (persona +
//! model + tool allow-list + step budget) driven to completion WITHOUT a human
//! in the loop. It runs against its own isolated [`Conversation`](crate::model::conversation::Conversation),
//! reports progress as a stream of [`AgentEvent`](event::AgentEvent)s, and is
//! killable via its [`AbortHandle`](tokio::task::AbortHandle).
//!
//! Module map:
//! - [`event`] — [`AgentEvent`](event::AgentEvent), the task -> orchestrator
//!   progress stream.
//! - [`context`] — [`build_seed`](context::build_seed): the isolated seed
//!   conversation (persona + memory + awareness + task).
//! - [`engine`] — [`run_agent_loop`](engine::run_agent_loop): the autonomous,
//!   non-interactive stream/tool loop.
//! - [`spawn`] — [`spawn_subagent`](spawn::spawn_subagent): resolve + seed +
//!   launch, returning a [`SubAgent`] handle.
//!
//! This module is ADDITIVE and currently UNUSED — nothing in the main chat loop
//! references it yet; it is wired into the UI / `task` tool in a later stage.

// Inert in Stage 1: the whole sub-agent surface is defined but not yet wired into
// the binary, so its items are legitimately unreferenced until a later stage.
#![allow(dead_code)]

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::AbortHandle;

pub mod context;
pub mod engine;
pub mod event;
pub mod spawn;

// Re-exports form the intended public surface for the later wiring stage. The
// loop + spawn entry points are not referenced yet, so silence the unused-import
// lint on them specifically (they become live when the orchestrator calls them).
#[allow(unused_imports)]
pub use engine::run_agent_loop;
pub use event::AgentEvent;
#[allow(unused_imports)]
pub use spawn::spawn_subagent;

// `PendingSubagent` is referenced from `AppStateRest` and both spawn paths the
// moment the queue is wired in, so it is a live part of the public surface.

/// Hard cap on the number of sub-agents that may run CONCURRENTLY. Both spawn
/// paths (the model-callable `task` tool and the `/task` slash command) refuse
/// to launch a new sub-agent while this many are already in [`SubAgentStatus::Running`],
/// so a misbehaving main agent can't fan out an unbounded swarm. Terminated
/// sub-agents are NOT pruned — they stay in the list as session history — but
/// they no longer count toward the cap once they leave `Running`, so a finished
/// agent frees its slot.
pub const MAX_SUBAGENTS: usize = 5;

/// Lifecycle state of a [`SubAgent`], folded from its [`AgentEvent`] stream by
/// the orchestrator (wired up later).
///
/// - `Running`: the loop is in flight (the initial state).
/// - `Done`: the loop finished cleanly; `String` is the final answer.
/// - `Killed`: the loop was aborted via its [`AbortHandle`].
/// - `Error`: the loop hit a fatal stream error; `String` is the cause.
#[derive(Debug, Clone)]
pub enum SubAgentStatus {
    Running,
    Done(String),
    Killed,
    Error(String),
}

/// A handle to one running sub-agent: its identity, lifecycle state, abort
/// handle, event receiver, and the accumulated transcript.
///
/// The orchestrator owns the [`SubAgent`] (in a list, wired up later), drains
/// `rx` each tick to advance `status` / append to `transcript`, and calls
/// `abort.abort()` to kill it.
pub struct SubAgent {
    /// Stable per-session id, assigned by the orchestrator at spawn.
    pub id: usize,
    /// The agent definition's name this sub-agent runs (lowercased).
    pub agent_name: String,
    /// Compact one-line label (the truncated task) for display in a list.
    pub label: String,
    /// Resolved model id the loop runs against. Set at spawn time from the
    /// resolved route; used by the usage ledger row.
    pub model_id: String,
    /// Lifecycle state, advanced as [`AgentEvent`]s are drained from `rx`.
    pub status: SubAgentStatus,
    /// Abort handle for the spawned loop task; `abort()` kills the sub-agent.
    pub abort: AbortHandle,
    /// Receiver end of the sub-agent's [`AgentEvent`] channel. Drained by the
    /// orchestrator; dropping it makes the task's emits no-ops.
    pub rx: UnboundedReceiver<AgentEvent>,
    /// Sender end of the INJECTION channel: a follow-up user message pushed here
    /// (via the broker `agents.send` verb or the main-agent `task_send` tool) is
    /// drained by [`engine::run_agent_loop`] at the TOP of its next iteration and
    /// folded into the sub-agent's isolated history as a fresh `user` turn — so a
    /// human/extension/main-agent can STEER a running sub-agent, delivered at a
    /// turn boundary (never mid-stream). Sending is best-effort: once the loop
    /// ends its receiver closes and further sends are dropped (the next drain then
    /// settles the agent). Restored/shadow records carry an inert sender nothing
    /// drains. Not persisted (runtime-only, like `rx`/`abort`).
    pub inject_tx: UnboundedSender<String>,
    /// Human-readable transcript lines accumulated from the event stream.
    pub transcript: Vec<String>,
    /// The sub-agent's structured conversation, replaced wholesale on each
    /// [`AgentEvent::Snapshot`]. Drives the full-screen history viewer; empty
    /// until the first turn is committed.
    pub messages: Vec<crate::dto::chat::ChatMessage>,
    /// Live in-progress assistant report text for the CURRENT (not-yet-committed)
    /// turn, accumulated from [`AgentEvent::Token`] and cleared on the next
    /// [`AgentEvent::Snapshot`] (which commits that turn into `messages`). Lets the
    /// full-screen viewer render the streaming report as it arrives instead of
    /// waiting for turn-end — mirrors how `messages`/`committed_reasoning` are
    /// projected end-to-end. Empty when nothing is streaming.
    pub live_text: String,
    /// The tool-call id from the model's `task` tool invocation that spawned
    /// this sub-agent, if any. `Some(call_id)` means the sub-agent was spawned
    /// by the model via the `task` tool; `None` means it was spawned by the
    /// user's `/task` slash command.
    pub tool_call_id: Option<String>,
    /// True when this sub-agent runs DETACHED (`task` with `run_in_background:
    /// true`): the spawning `task` call was answered IMMEDIATELY with the id (it
    /// carries `tool_call_id == None`, so the main turn never parks on it) and
    /// the model polls it with `task_output` / stops it with `task_kill`. On
    /// terminal a detached agent fires a ONE-shot completion nudge instead of
    /// the `/task` chat-fold — see `nudged`. A blocking model delegation or a
    /// `/task` slash-command agent is NOT detached.
    pub detached: bool,
    /// De-dupe latch for the detached-completion nudge: set the first tick a
    /// `detached` sub-agent is observed in a terminal state, so the nudge is
    /// injected EXACTLY ONCE even though the terminal-fold block runs every
    /// tick. Ignored for non-detached agents. Starts `false`.
    pub nudged: bool,
    /// True when this sub-agent was spawned by an EXTENSION via the broker's
    /// `agents.spawn` (see `app::ext::broker::broker_spawn`), as opposed to a human
    /// `/task` command or a model `task` delegation. An ext-owned agent is
    /// EXTENSION-INTERNAL: on terminal it stays COMPLETELY SILENT in the human chat
    /// (no fold note, no nudge) — its spawner already receives the result via the
    /// owned `agents.done` event (see [`crate::app::ext::events::emit_subagent_terminal`]).
    /// `drain_subagents` skips the compact completion note when this is set, while
    /// STILL recording usage + the persisted sub-agent record + firing `agents.done`.
    /// `false` for every non-extension spawn path.
    pub ext_owned: bool,
    /// Last-seen prompt tokens from [`AgentEvent::UsageReport`] (context size,
    /// not a cumulative sum). Zero until the report arrives.
    pub usage_tokens_in: u64,
    /// Cumulative completion tokens across all steps (sum).
    pub usage_tokens_out: u64,
    /// Cumulative USD cost across all steps (sum).
    pub usage_cost: f64,
}

/// Per-call spawn overrides a caller may supply to steer a SINGLE delegation
/// onto a different model/effort than its [`crate::model::agent_def::AgentDef`]
/// declares, without mutating the registry entry itself.
///
/// `None` fields mean "use the agent definition's own value" — every
/// non-extension spawn path (the model-callable `task` tool, `/task`, and the
/// queued→running promotion via `try_start_pending`) passes `None` for the
/// WHOLE struct (no overrides at all), so those paths see zero behavior
/// change. Only `agents.spawn`'s optional `model`/`effort` params (see
/// `app::ext::broker::broker_spawn`) ever construct a `Some`.
///
/// Applied in [`spawn::spawn_subagent`]: a `Some(model)` REPLACES the agent's
/// `model` slug (and clears `model_uuid`/`provider_uuid`, so the slug re-resolves
/// fresh through [`crate::app::resolve::resolve_agent`]'s step 1c rather than
/// falling back to a now-stale registered entry); a `Some(effort)` REPLACES the
/// agent's `effort`. Neither mutates the registry's [`crate::model::agent_def::AgentDef`]
/// — the override is applied to a throwaway clone used only for THIS spawn's
/// route resolution.
#[derive(Debug, Clone, Default)]
pub struct SpawnOverrides {
    /// Overrides the agent's `model` slug for this spawn only.
    pub model: Option<String>,
    /// Overrides the agent's `effort` for this spawn only.
    pub effort: Option<String>,
    /// Confines this spawn's [`crate::tool::ToolCtx`] to a single caller-supplied
    /// workspace root instead of inheriting the whole session. `None` (every
    /// non-extension spawn path, and the common `agents.spawn`/`sessions.spawn_into`
    /// call) leaves `ctx.workspace`/`ctx.workspaces` exactly as the session's own
    /// `build_tool_ctx` produced them.
    ///
    /// `Some(path)` is applied in
    /// [`crate::app::runtime::stream::spawn::spawn_task_with_id`] (never here —
    /// [`spawn::spawn_subagent`] receives an already-narrowed `ctx`): the path is
    /// canonicalized and checked for CONTAINMENT within one of the session's
    /// existing `ctx.workspaces` roots (also canonicalized, compared by path
    /// COMPONENTS via `Path::starts_with`, never a raw string prefix — so `/a/bc`
    /// never matches a root `/a/b`). On success `ctx.workspace` and
    /// `ctx.workspaces` are BOTH replaced with the single canonicalized path — the
    /// sub-agent then sees ONLY that root, never the wider session tree. On
    /// failure (can't canonicalize, or resolves outside every root) the spawn is
    /// REJECTED outright with an explicit error naming the rejected path — this is
    /// a sandbox trust boundary, so it never silently falls back to the wide
    /// workspace.
    pub workspace: Option<std::path::PathBuf>,
}

/// A delegation that has been ACCEPTED but not yet started because all
/// [`MAX_SUBAGENTS`] slots are occupied. It waits at the back of
/// [`AppStateRest::pending_subagents`](crate::app::state::AppStateRest::pending_subagents)
/// and is started (popped from the FRONT) by `try_start_pending` the moment a
/// running sub-agent terminates and frees a slot.
///
/// Its `id` is allocated from `next_subagent_id` at ENQUEUE time so the `$`
/// panel can show a stable id for the queued row, and the spawned [`SubAgent`]
/// inherits that exact id when it finally starts. For a model-callable `task`
/// delegation (`tool_call_id == Some`) the call id is ALSO recorded in
/// `pending_subagent_calls` at enqueue time, so a parked main turn waits for the
/// queued delegation just as it waits for a running one — its result fills when
/// the queued agent eventually runs and finishes.
///
/// `PendingSubagent` is NEVER persisted to disk (unlike the running [`SubAgent`]
/// list — see `bg_persist::persist_subagents`, which only serializes `subagents`)
/// — it carries no `Serialize`/`Deserialize` derive at all, so `overrides` needs
/// no `#[serde(default)]` back-compat guard.
#[derive(Debug, Clone)]
pub struct PendingSubagent {
    /// Stable id pre-allocated at enqueue time; the spawned [`SubAgent`] takes it.
    pub id: usize,
    /// The agent definition's name to run (resolved at spawn time, not now).
    pub agent_name: String,
    /// The task prompt to seed the sub-agent with.
    pub prompt: String,
    /// The `task`-tool call id this delegation answers, if any (`None` for a
    /// `/task` slash-command enqueue, which never parks the main turn).
    pub tool_call_id: Option<String>,
    /// Carries the detached flag across the queued→running promotion, so a
    /// background `task` delegation enqueued while all slots were busy stays
    /// detached (fires the completion nudge, never parks) once `try_start_pending`
    /// promotes it. `false` for a blocking delegation or a `/task` enqueue.
    pub detached: bool,
    /// Carries the `ext_owned` origin flag across the queued→running promotion, so
    /// an `agents.spawn` delegation enqueued while all slots were busy stays
    /// extension-owned (silent on completion) once `try_start_pending` promotes it.
    /// `false` for a `/task` enqueue or a model `task` delegation.
    pub ext_owned: bool,
    /// Carries any per-call spawn overrides (model/effort) across the
    /// queued→running promotion, so a queued `agents.spawn` override survives
    /// the wait for a free slot exactly like `detached` does.
    pub overrides: Option<SpawnOverrides>,
    /// Follow-up user messages injected (via `agents.send` / `task_send`) while
    /// this delegation was still QUEUED — it has no running loop / channel yet, so
    /// they are stashed here in submit order and handed to the sub-agent's
    /// injection channel at promotion time (`try_start_pending` →
    /// `spawn_task_with_id`), delivered as its FIRST follow-ups right after the
    /// task prompt. Empty for the common case (nothing steered a queued agent).
    pub pending_injects: Vec<String>,
}

/// Outcome of injecting a follow-up user message into a sub-agent — the shared
/// result of the broker `agents.send` verb and the main-agent `task_send` tool,
/// which funnel through the SAME
/// [`SessionRuntime::inject_into_subagent`](crate::app::state::SessionRuntime::inject_into_subagent)
/// helper so the two surfaces steer identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectOutcome {
    /// Handed to a RUNNING sub-agent's injection channel; the engine folds it into
    /// history (and the viewer transcript) at its next turn boundary.
    Sent,
    /// Stashed on a QUEUED (not-yet-started) sub-agent; delivered when it is
    /// promoted to running.
    Queued,
    /// The sub-agent is in a TERMINAL state (done/killed/error) — nothing delivered.
    Terminal,
    /// No sub-agent (running or queued) with that local id in this session.
    Unknown,
}
