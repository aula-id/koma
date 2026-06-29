//! Session-level shadow reconstruction: `SessionRuntime`, `Session`, and `SubAgent`.

use std::time::Duration;

use crate::app::mode::editor::TextEditorState;
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState,
    PickerState, RewindEntry, RewindState, SessionHub, SessionKind, UsageMetric, UsageNavState,
    UsageRange, UsageView, WarmStatus,
};
use crate::app::state::{SessionRuntime, ToastKind};
use crate::app::subagent::{PendingSubagent, SubAgent, SubAgentStatus};
use crate::ipc::proto::{
    AgentEntry, AgentModelPickerSnapshot, AgentsSnapshot, EffortSnapshot, KeyInputSnapshot,
    LoadingSnapshot, ModelModalSnapshot, PathPickerSnapshot, PickerSnapshot, RewindSnapshot,
    SessionHubSnapshot, SessionSnapshot, SettingsSnapshot, SubAgentSnapshot, TextEditorSnapshot,
    ToolPickerSnapshot, WarmStatusWire,
};
use crate::model::conversation::Conversation;
use crate::model::session::Session;
use crate::model::settings::Settings;
use crate::model::store::SessionMeta;

/// Map a wire toast `kind` string ("error" / "info") to the local [`ToastKind`].
/// Anything unexpected degrades to `Info` (a neutral box, never a false error).
pub(crate) fn toast_kind(kind: &str) -> ToastKind {
    match kind {
        "error" => ToastKind::Error,
        _ => ToastKind::Info,
    }
}

/// Build a shadow [`SessionRuntime`] from one [`SessionSnapshot`].
///
/// Carries the stable id + the render-relevant fields (streaming buffers, token /
/// cost counters, approval flags, working/finished-unseen). The committed messages +
/// name + model are reconstructed into a minimal [`Session`] so the unmodified chat
/// transcript/header/input renderers consume it exactly as they do a live session.
/// Every NON-render field stays at `Default` — the client never advances a turn, so
/// the tool/sub-agent state machines and channels are never read.
///
/// Both the QUEUED [`PendingSubagent`]s and the RUNNING/finished `subagents` are
/// reconstructed from their plain-data projections, so the `$` panel rows AND the
/// full-screen sub-agent viewer (both rendered FROM Chat mode) draw off real shadow
/// data. A live [`crate::app::subagent::SubAgent`] needs a tokio `AbortHandle` +
/// receiver, which require a runtime in scope — `client_run` enters the runtime
/// context for the render loop so [`shadow_subagent`] can mint inert ones (the
/// client never drives a sub-agent; the handle/rx exist only to satisfy the type).
pub(crate) fn shadow_session_runtime(s: &SessionSnapshot) -> SessionRuntime {
    let mut rt = SessionRuntime::new();
    rt.id = s.id.clone();
    rt.session = Some(shadow_session(s));
    // Mirror the daemon's effective cwd onto the shadow as the live override, so
    // the reconstructed runtime's `effective_cwd()` matches (the shadow session's
    // own `settings.workdir` isn't projected, so this is the only cwd source).
    // Empty only when the daemon had no session; leave the default `None` then.
    if !s.cwd.is_empty() {
        rt.active_cwd = Some(std::path::PathBuf::from(&s.cwd));
    }
    rt.streaming = s.streaming.clone();
    rt.stream_reasoning = s.stream_reasoning.clone();
    rt.tokens_in = s.tokens_in;
    rt.tokens_out = s.tokens_out;
    rt.cost = s.cost;
    rt.tokens_cached = s.tokens_cached;
    // `waiting` drives the local input-poll cadence + the comet; mirror the snapshot's
    // composite `working` so a parked/streaming background session keeps the shadow
    // ticking fast and shimmering, matching the daemon.
    rt.waiting = s.working;
    rt.awaiting_approval = s.awaiting_approval;
    rt.approval_reason = s.approval_reason.clone();
    // The pending tool-call round + cursor so the Chat-mode approval overlay (gated
    // on `awaiting_approval`) renders the real paused call — name, args, payload
    // preview — exactly as the local TUI does. `ToolCall` is plain data; the client
    // never executes it (the y/n is forwarded as `ApproveTool`).
    rt.pending_tool_calls = s.pending_tool_calls.clone();
    rt.tool_idx = s.tool_idx;
    rt.finished_unseen = s.finished_unseen;
    // Reconstruct the queued delegations (plain data) so the remote `$` panel can
    // list the same "pending" rows the local TUI shows. FIFO order is preserved.
    rt.pending_subagents = s
        .pending_subagents
        .iter()
        .map(|p| PendingSubagent {
            id: p.id,
            agent_name: p.agent_name.clone(),
            prompt: p.prompt.clone(),
            // The turn-bookkeeping call id is daemon-internal + never rendered, and the
            // client never advances a turn, so a shadow pending entry carries `None`.
            tool_call_id: None,
        })
        .collect();
    // Reconstruct the running/finished sub-agents (plain data + an inert handle/rx)
    // so the `$` panel list AND the full-screen viewer render off real shadow data.
    rt.subagents = s.subagents.iter().map(shadow_subagent).collect();
    rt
}

/// Rebuild a render-only [`SubAgent`] from its projection.
///
/// Carries every field the `$` panel + the full-screen viewer read (id, agent name,
/// label, status, transcript, structured messages). The two runtime-bound fields a
/// live `SubAgent` requires — the `abort` [`tokio::task::AbortHandle`] and the `rx`
/// event receiver — are minted INERT here: the client never drains `rx` and never
/// aborts (it forwards every key to the daemon, which owns the real sub-agent), so a
/// no-op aborted task's handle + a fresh unused channel satisfy the type without ever
/// being driven. `tokio::spawn` needs a runtime in scope, which `client_run` enters
/// for the render loop. The non-rendered bookkeeping (`model_id`, `tool_call_id`,
/// usage counters) is left empty/zero — the viewer and panel never read it.
pub(crate) fn shadow_subagent(sa: &SubAgentSnapshot) -> SubAgent {
    // Inert abort handle: a task that completes immediately; its handle is never used
    // to abort anything (the daemon owns the real task). Cheap + completes at once.
    let abort = tokio::spawn(std::future::ready(())).abort_handle();
    // Fresh receiver the client never drains (the daemon folds real events; a shadow
    // sub-agent's content arrives wholesale via the next snapshot's `messages`).
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    SubAgent {
        id: sa.id,
        agent_name: sa.name.clone(),
        label: sa.label.clone(),
        // Not rendered by the panel/viewer; left blank (the wire omits it).
        model_id: String::new(),
        status: shadow_subagent_status(&sa.status),
        abort,
        rx,
        transcript: sa.transcript.clone(),
        messages: sa.messages.clone(),
        tool_call_id: None,
        usage_tokens_in: 0,
        usage_tokens_out: 0,
        usage_cost: 0.0,
    }
}

/// Map a wire sub-agent status string back to a [`SubAgentStatus`].
///
/// The daemon flattens the lifecycle enum to a short string (see
/// `ipc::snapshot::subagent_snapshot`): `"running"` / `"done"` / `"killed"` /
/// `"error: <detail>"`. The `Done` final-answer payload is NOT carried (the viewer —
/// the projection's target — renders the transcript + the short status tag, neither of
/// which uses it), so a reconstructed `Done` carries an empty answer; an `Error`
/// keeps its detail (the panel's status line shows it). Unknown → `Running` (the
/// safe "still going" default, never lost).
fn shadow_subagent_status(status: &str) -> SubAgentStatus {
    match status {
        "done" => SubAgentStatus::Done(String::new()),
        "killed" => SubAgentStatus::Killed,
        s if s.starts_with("error:") => {
            SubAgentStatus::Error(s.trim_start_matches("error:").trim().to_string())
        }
        _ => SubAgentStatus::Running,
    }
}

/// Reconstruct a minimal [`Session`] from a [`SessionSnapshot`] for rendering.
///
/// Only the fields the chat view reads are meaningful: `name` (the input-tab label),
/// `conversation` (the transcript), and `settings.model` (the model-name row). The
/// path / pwd_hash / api_key are render-irrelevant on the client and left empty —
/// the client never saves, sends, or locks anything.
pub(crate) fn shadow_session(s: &SessionSnapshot) -> Session {
    // Seed `settings.model` with the daemon-side resolved model id projected in the
    // snapshot. The client's shadow config is keyless + catalogue-cleared, so
    // resolve_role on the client would return empty; using the projected id means
    // the chat header always shows the same model name the daemon resolved.
    let settings = Settings {
        name: s.name.clone(),
        model: s.resolved_model_id.clone(),
        ..Default::default()
    };

    // Re-attach the display-only reasoning the wire carried out-of-band. The
    // `ChatMessage::reasoning` field is `#[serde(skip)]`, so every deserialised
    // message arrives with `reasoning: None`; without this fold-back a committed
    // turn's thinking block would never render on the client (it would only show
    // while the live `stream_reasoning` buffer streamed, then vanish on finalize).
    // The side-channel is index-aligned with `messages`; a missing/short entry
    // (the common no-reasoning case ships an empty vec) leaves `reasoning` at None.
    let mut messages = s.messages.clone();
    for (i, msg) in messages.iter_mut().enumerate() {
        if let Some(Some(reasoning)) = s.committed_reasoning.get(i) {
            msg.reasoning = Some(reasoning.clone());
        }
    }

    Session::new(
        s.id.clone(),
        std::path::PathBuf::new(),
        String::new(),
        settings,
        Conversation::from_messages(messages),
    )
}
