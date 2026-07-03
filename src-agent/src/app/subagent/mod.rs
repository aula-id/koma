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

use tokio::sync::mpsc::UnboundedReceiver;
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
    /// Human-readable transcript lines accumulated from the event stream.
    pub transcript: Vec<String>,
    /// The sub-agent's structured conversation, replaced wholesale on each
    /// [`AgentEvent::Snapshot`]. Drives the full-screen history viewer; empty
    /// until the first turn is committed.
    pub messages: Vec<crate::dto::chat::ChatMessage>,
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
    /// Last-seen prompt tokens from [`AgentEvent::UsageReport`] (context size,
    /// not a cumulative sum). Zero until the report arrives.
    pub usage_tokens_in: u64,
    /// Cumulative completion tokens across all steps (sum).
    pub usage_tokens_out: u64,
    /// Cumulative USD cost across all steps (sum).
    pub usage_cost: f64,
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
}
