//! The `requires` GRANT BROKER: let an extension DRIVE koma's sub-agent system
//! over its duplex socket, gated by the scopes koma granted it.
//!
//! ## Where this runs (and why)
//!
//! An extension's socket reader task (see [`super::wire::reader_task`]) runs in a
//! background tokio task with NO access to [`AppState`] / the session set — but
//! spawning and reading sub-agents needs that state. So an `agents.*`
//! [`ExtMsg::Call`](koma_extension::protocol::ExtMsg) is not handled inline on the
//! read path; it is packaged into an [`ExtCallRequest`] and pushed onto
//! `AppStateRest::ext_call_tx`, then drained on the event loop (where `AppState`
//! is live) by `drain_ext_calls`, which calls [`handle_ext_call`] here and sends
//! the resulting JSON back over the request's `reply` oneshot. This mirrors the
//! `drain_oauth` background→event-loop hand-off exactly.
//!
//! ## The security boundary
//!
//! [`handle_ext_call`] checks the GRANT GATE **first**, before touching any
//! session state: `agents.spawn`/`agents.kill` require
//! [`Grant::AgentsOrchestrate`]; `agents.list`/`agents.status`/`agents.result`
//! require [`Grant::AgentsRead`] OR `AgentsOrchestrate` (orchestrate implies
//! read). A call whose required grant is not in the extension's granted set is
//! rejected outright — nothing is spawned, read, or killed. The gate decision is
//! factored into the pure [`method_permitted`] so it can be unit-tested
//! exhaustively.
//!
//! ## Where the sub-agents live
//!
//! An extension's spawned sub-agents live in the **ACTIVE chat session** — they
//! share its [`MAX_SUBAGENTS`](crate::app::subagent::MAX_SUBAGENTS) cap and its
//! lifecycle, routed through the SAME `spawn_or_queue` path the model's `task`
//! tool uses. There is no separate fleet/session space. A spawn carries
//! `tool_call_id = None` and `detached = false` (the `/task`-command shape): on
//! completion the session records a display-only note and accounts the usage, but
//! the chat model is never auto-woken — the extension retrieves the real output by
//! polling `agents.result`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use koma_extension::protocol::Grant;

use crate::app::runtime::{spawn_or_queue, SpawnOutcome};
use crate::app::state::{AppState, SessionRuntime};
use crate::app::subagent::SubAgentStatus;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

/// The agent an extension `agents.spawn` runs when it omits `agent` — koma's
/// built-in general-purpose agent.
const DEFAULT_AGENT: &str = "general";

/// One extension's PRIVATE registry of the sub-agents IT spawned, keyed by an
/// ext-facing id unique to THIS extension. This is the containment fix for a
/// review finding: the raw per-session `SubAgent::id` is a per-session
/// MONOTONIC COUNTER that restarts at `0` for every session, so two different
/// (possibly unrelated) sessions can each have a sub-agent numbered e.g. `0` at
/// the same time. Before this registry, `agents.status`/`agents.result`/
/// `agents.kill` resolved `agentId` against whichever session happened to be
/// FOREGROUND at call time (via `active_session_idx`) — so a foreground switch
/// between an extension's `agents.spawn` and its next poll could silently
/// redirect that poll at a DIFFERENT session's (possibly user-spawned)
/// sub-agent that happened to reuse the same numeric id.
///
/// The fix: every verb except `agents.spawn` resolves `agentId` ONLY through
/// this map, keyed by an id this extension itself was handed. Each entry binds
/// that ext-facing id permanently to the STABLE session UUID (never the
/// transient `Vec` index, which shifts as sessions open/close) it was spawned
/// into, plus the per-session local sub-agent id `spawn_or_queue` returned
/// there. An extension can therefore never name a sub-agent it didn't spawn —
/// not another session's, not another extension's, not the user's own — even
/// after any number of foreground switches.
#[derive(Default)]
pub struct ExtAgentRegistry {
    /// Next ext-facing agent id to hand out. Monotonic within this extension,
    /// never reused — so even a killed/forgotten agent's id is never recycled
    /// onto a different sub-agent.
    next_id: u64,
    /// ext-facing agent id -> where it really lives.
    map: HashMap<u64, ExtAgentRef>,
}

impl ExtAgentRegistry {
    /// Allocate a fresh ext-facing id for a just-spawned/queued sub-agent and
    /// remember where it really lives. Returns the new ext-facing id.
    fn insert(&mut self, session_uuid: String, local_subagent_id: usize) -> u64 {
        let ext_agent_id = self.next_id;
        self.next_id += 1;
        self.map.insert(
            ext_agent_id,
            ExtAgentRef {
                session_uuid,
                local_subagent_id,
            },
        );
        ext_agent_id
    }

    /// Resolve an ext-facing id to where it really lives, iff THIS extension
    /// was ever handed that id.
    fn get(&self, ext_agent_id: u64) -> Option<&ExtAgentRef> {
        self.map.get(&ext_agent_id)
    }

    /// Every sub-agent this extension has ever spawned, oldest-id-first (stable
    /// ordering for `agents.list`).
    fn entries_sorted(&self) -> Vec<(u64, &ExtAgentRef)> {
        let mut v: Vec<(u64, &ExtAgentRef)> = self.map.iter().map(|(k, r)| (*k, r)).collect();
        v.sort_by_key(|(id, _)| *id);
        v
    }
}

/// Where one ext-facing agent id really lives: the sub-agent's STABLE session
/// UUID (see [`ExtAgentRegistry`]'s doc for why never the transient `Vec`
/// index) and the per-session local sub-agent id `spawn_or_queue` returned
/// within that session.
#[derive(Clone)]
struct ExtAgentRef {
    session_uuid: String,
    local_subagent_id: usize,
}

/// Resolve `session_uuid` to a LIVE (non-closed) session in `state.rest.sessions`.
/// `None` when the session has been closed (or, in principle, no longer exists) —
/// sessions are append+tombstone (never removed from the `Vec`, only marked
/// `closed`), so in practice this is `None` exactly when the session that
/// `ExtAgentRef` points at has since been closed.
fn resolve_ext_session<'a>(state: &'a AppState, session_uuid: &str) -> Option<&'a SessionRuntime> {
    state
        .rest
        .sessions
        .iter()
        .find(|s| s.id == session_uuid && !s.closed)
}

/// An ext→koma `agents.*` call awaiting dispatch on the event loop.
///
/// Built by the extension's [`reader_task`](super::wire::reader_task) (which has
/// no [`AppState`] access) and pushed onto `AppStateRest::ext_call_tx`; drained
/// each tick by `drain_ext_calls`, which hands it to [`handle_ext_call`] and
/// sends the broker's JSON back on `reply`. Carries the extension's `granted`
/// scopes so the gate is evaluated against exactly what koma extended to THIS
/// extension.
pub struct ExtCallRequest {
    /// The calling extension's id (for logging / diagnostics).
    pub ext_id: String,
    /// The scopes koma granted this extension (its manifest `requires`, echoed at
    /// handshake). The grant gate is evaluated against this set.
    pub granted: Vec<Grant>,
    /// The canonical method: `agents.spawn` | `agents.list` | `agents.status` |
    /// `agents.result` | `agents.kill`.
    pub method: String,
    /// Method params (e.g. `{ "task": ..., "agent": ... }` / `{ "agentId": ... }`).
    pub params: Value,
    /// One-shot the reader task awaits; [`handle_ext_call`]'s `Value` is sent here.
    pub reply: tokio::sync::oneshot::Sender<Value>,
}

/// An ext→koma fire-and-forget `Notify` awaiting dispatch on the event loop.
///
/// Built by the extension's [`reader_task`](super::wire::reader_task) (which has
/// no [`AppState`] access) and pushed onto `AppStateRest::ext_notify_tx`, drained
/// each tick by the event loop. Unlike [`ExtCallRequest`] there is no `reply` —
/// `Notify` never expects one (e.g. `panel.push`, routed to the panel bridge in a
/// later wave).
pub struct ExtNotify {
    /// The calling extension's id.
    pub ext_id: String,
    /// The notify name (e.g. `"panel.push"`).
    pub name: String,
    /// Notify params.
    pub params: Value,
}

/// Outcome of the grant gate for one `agents.*` method against a granted set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateDecision {
    /// The extension holds a grant that permits the method.
    Allow,
    /// The method is known but the extension lacks the required grant (carried).
    Deny(Grant),
    /// The method is not a recognised `agents.*` verb.
    UnknownMethod,
}

/// The grant an `agents.*` method requires, or `None` if the method is unknown.
///
/// `agents.spawn` / `agents.kill` MUTATE the fleet → require
/// [`Grant::AgentsOrchestrate`]. `agents.list` / `agents.status` /
/// `agents.result` only READ → require [`Grant::AgentsRead`] (satisfied by
/// orchestrate too; see [`is_granted`]).
fn required_grant(method: &str) -> Option<Grant> {
    match method {
        "agents.spawn" | "agents.kill" => Some(Grant::AgentsOrchestrate),
        "agents.list" | "agents.status" | "agents.result" => Some(Grant::AgentsRead),
        _ => None,
    }
}

/// Whether `granted` satisfies `required`. Orchestrate IMPLIES read: a
/// read-requiring method is permitted by either `AgentsRead` or
/// `AgentsOrchestrate`; an orchestrate-requiring method needs `AgentsOrchestrate`
/// outright (read alone never grants it).
fn is_granted(granted: &[Grant], required: Grant) -> bool {
    match required {
        Grant::AgentsOrchestrate => granted.contains(&Grant::AgentsOrchestrate),
        Grant::AgentsRead => {
            granted.contains(&Grant::AgentsRead) || granted.contains(&Grant::AgentsOrchestrate)
        }
        // WAVE-1 COMPILE STUB: `required_grant` never returns these yet (no method
        // requires them), so these arms are unreachable today. Exhaustiveness-only
        // placeholder so the crate builds after `Grant` grew wave-1 protocol
        // variants; real gating logic for each lands in the wave that wires its
        // methods (see task board: sessions:manage / chat:prompt / models:invoke /
        // context:publish).
        Grant::SessionsManage => granted.contains(&Grant::SessionsManage),
        Grant::ChatPrompt => granted.contains(&Grant::ChatPrompt),
        Grant::ModelsInvoke => granted.contains(&Grant::ModelsInvoke),
        Grant::ContextPublish => granted.contains(&Grant::ContextPublish),
    }
}

/// PURE grant-gate decision — no state, no I/O — so the security boundary can be
/// tested exhaustively. Returns whether an extension holding `granted` may invoke
/// `method` (and, on denial, the grant it was missing).
pub(crate) fn method_permitted(method: &str, granted: &[Grant]) -> GateDecision {
    match required_grant(method) {
        None => GateDecision::UnknownMethod,
        Some(required) if is_granted(granted, required) => GateDecision::Allow,
        Some(required) => GateDecision::Deny(required),
    }
}

/// The wire string for a [`Grant`] (for error messages / logs).
fn grant_wire(g: Grant) -> &'static str {
    match g {
        Grant::AgentsRead => "agents:read",
        Grant::AgentsOrchestrate => "agents:orchestrate",
        // WAVE-1 COMPILE STUB: see `is_granted` above.
        Grant::SessionsManage => "sessions:manage",
        Grant::ChatPrompt => "chat:prompt",
        Grant::ModelsInvoke => "models:invoke",
        Grant::ContextPublish => "context:publish",
    }
}

/// Parse the persisted wire strings (e.g. `"agents:read"`) an extension was
/// `granted` into [`Grant`]s. Unrecognised strings are DROPPED (fail-closed: an
/// unknown grant confers nothing). Shared with [`ExtHostManager::ensure_started_at`]
/// so the reader task can attach exactly the scopes koma persisted.
pub(crate) fn parse_grants(wire: &[String]) -> Vec<Grant> {
    wire.iter()
        .filter_map(|s| match s.as_str() {
            "agents:read" => Some(Grant::AgentsRead),
            "agents:orchestrate" => Some(Grant::AgentsOrchestrate),
            _ => None,
        })
        .collect()
}

/// Dispatch one ext→koma `agents.*` call against the ACTIVE chat session, gated by
/// the extension's `granted` scopes. Returns the JSON the extension receives as its
/// `KomaMsg::Result`.
///
/// GRANT GATE FIRST (the security boundary): a call whose required grant is absent
/// is rejected before ANY session state is read or mutated. Then the active session
/// is resolved; then the verb is dispatched. Never panics — every failure path
/// returns an `{"error": ...}` object so the extension's `call()` always unblocks.
pub fn handle_ext_call(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    client: &Option<Arc<OpenRouterClient>>,
    ext_id: &str,
    granted: &[Grant],
    method: &str,
    params: Value,
) -> Value {
    // 1. GRANT GATE — airtight, and before touching a single session field.
    match method_permitted(method, granted) {
        GateDecision::UnknownMethod => {
            return json!({ "error": format!("unknown method: {method}") });
        }
        GateDecision::Deny(required) => {
            let wire = grant_wire(required);
            // Runtime logging goes to ~/.koma/error.log, never stdout (TUI-safe).
            store::append_global_error_log(
                "extensions",
                &format!("[{ext_id}] grant denied: {method} requires {wire}"),
            );
            return json!({ "error": format!("grant denied: {method} requires {wire}") });
        }
        GateDecision::Allow => {}
    }

    // 2. Dispatch the (now-authorised) verb. `agents.spawn` ALONE resolves the
    // ACTIVE (foreground) session — spawning into "whatever chat session is in
    // front of the user right now" is the intended behavior. Every other verb
    // resolves the sub-agent through THIS extension's own [`ExtAgentRegistry`]
    // instead (never the foreground), so a foreground switch between a spawn and
    // a later poll can never redirect that poll at a different session's sub-agent.
    match method {
        "agents.spawn" => {
            let Some(sess_idx) = active_session_idx(state) else {
                return json!({ "error": "no active session" });
            };
            broker_spawn(state, ext_id, sess_idx, client, handle, &params)
        }
        "agents.list" => broker_list(state, ext_id),
        "agents.status" => broker_status(state, ext_id, &params),
        "agents.result" => broker_result(state, ext_id, &params),
        "agents.kill" => broker_kill(state, ext_id, &params),
        // Unreachable: method_permitted already rejected anything else above.
        _ => json!({ "error": format!("unknown method: {method}") }),
    }
}

/// The active chat session index: the foreground session when it is live, else the
/// first non-closed session (mirrors `AppStateRest::resolve_foreground`'s
/// fallback). `None` only when every session is closed. Used ONLY by
/// `agents.spawn` — every other verb resolves through the extension's own
/// [`ExtAgentRegistry`], never the foreground.
fn active_session_idx(state: &AppState) -> Option<usize> {
    let fg = state.rest.foreground;
    if fg < state.rest.sessions.len() && !state.rest.sessions[fg].closed {
        return Some(fg);
    }
    state.rest.sessions.iter().position(|s| !s.closed)
}

/// `agents.spawn { task, agent? }` → route through the SAME `spawn_or_queue` path
/// the model's `task` tool uses (respecting `MAX_SUBAGENTS` → queue when full),
/// into the ACTIVE (foreground) session. `agent` defaults to [`DEFAULT_AGENT`].
/// Spawned NON-detached with no tool-call id (the `/task`-command shape) so
/// completion records a display note + usage but never auto-wakes the chat
/// model. The returned `agentId` is an EXT-FACING id freshly allocated from this
/// extension's own [`ExtAgentRegistry`] (never the raw per-session sub-agent
/// id), permanently bound to the session's STABLE UUID — see the registry's doc
/// for why that containment matters.
fn broker_spawn(
    state: &mut AppState,
    ext_id: &str,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    params: &Value,
) -> Value {
    let task = params
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if task.is_empty() {
        return json!({ "error": "agents.spawn requires a non-empty 'task'" });
    }
    let agent = params
        .get("agent")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_AGENT);

    // Capture the STABLE uuid of the session being spawned into BEFORE
    // `spawn_or_queue` (which needs `state` mutably) — this is the uuid the
    // resulting ext-facing agent id stays bound to regardless of any later
    // foreground switch.
    let session_uuid = state.rest.sessions[sess_idx].id.clone();

    match spawn_or_queue(state, sess_idx, client, handle, agent, task, None, false) {
        SpawnOutcome::Spawned(local_id) => {
            let ext_agent_id = state
                .rest
                .ext_agents
                .entry(ext_id.to_string())
                .or_default()
                .insert(session_uuid, local_id);
            json!({ "agentId": ext_agent_id, "status": "spawned" })
        }
        SpawnOutcome::Queued(local_id) => {
            let ext_agent_id = state
                .rest
                .ext_agents
                .entry(ext_id.to_string())
                .or_default()
                .insert(session_uuid, local_id);
            json!({ "agentId": ext_agent_id, "status": "queued" })
        }
        SpawnOutcome::Failed => json!({
            "error": format!("failed to spawn agent '{agent}' (no client/session or unknown agent)")
        }),
    }
}

/// `agents.list {}` → ONLY this extension's own [`ExtAgentRegistry`] entries, as
/// `[{ agentId, agent, status }]` — NEVER the raw session `subagents` collection
/// (which may hold other sessions'/extensions'/the user's own sub-agents). An
/// entry whose session has since closed (or whose sub-agent is otherwise no
/// longer found there) reports `"status": "gone"` instead of being silently
/// resolved against whatever now occupies that session slot.
fn broker_list(state: &AppState, ext_id: &str) -> Value {
    let Some(registry) = state.rest.ext_agents.get(ext_id) else {
        return Value::Array(Vec::new());
    };
    let arr: Vec<Value> = registry
        .entries_sorted()
        .into_iter()
        .map(|(ext_agent_id, r)| {
            let Some(sess) = resolve_ext_session(state, &r.session_uuid) else {
                return json!({ "agentId": ext_agent_id, "status": "gone" });
            };
            if let Some(sa) = sess.subagents.iter().find(|s| s.id == r.local_subagent_id) {
                return json!({
                    "agentId": ext_agent_id,
                    "agent": sa.agent_name,
                    "status": status_label(&sa.status),
                });
            }
            if sess.pending_subagents.iter().any(|p| p.id == r.local_subagent_id) {
                return json!({ "agentId": ext_agent_id, "status": "queued" });
            }
            json!({ "agentId": ext_agent_id, "status": "gone" })
        })
        .collect();
    Value::Array(arr)
}

/// `agents.status { agentId }` → that sub-agent's status (+ live-report length), a
/// queued marker if it is still parked, or an error. `agentId` is resolved ONLY
/// through THIS extension's own [`ExtAgentRegistry`] — an id this extension was
/// never handed (another extension's, or a raw session-local id it never
/// received) is `"unknown agentId"`, and a resolved-but-closed session is
/// `"session closed"`. Neither ever falls back to whatever session happens to
/// be foreground right now.
fn broker_status(state: &AppState, ext_id: &str, params: &Value) -> Value {
    let Some(ext_agent_id) = parse_ext_agent_id(params) else {
        return json!({ "error": "agents.status requires an 'agentId'" });
    };
    let Some(r) = state
        .rest
        .ext_agents
        .get(ext_id)
        .and_then(|reg| reg.get(ext_agent_id))
    else {
        return json!({ "error": format!("unknown agentId: {ext_agent_id}") });
    };
    let Some(sess) = resolve_ext_session(state, &r.session_uuid) else {
        return json!({ "error": "session closed" });
    };
    if let Some(sa) = sess.subagents.iter().find(|s| s.id == r.local_subagent_id) {
        return json!({
            "agentId": ext_agent_id,
            "agent": sa.agent_name,
            "status": status_label(&sa.status),
            "liveTextLen": sa.live_text.len(),
        });
    }
    // A just-queued spawn (over the cap) is not yet in `subagents`.
    if sess.pending_subagents.iter().any(|p| p.id == r.local_subagent_id) {
        return json!({ "agentId": ext_agent_id, "status": "queued" });
    }
    json!({ "error": format!("unknown agentId: {ext_agent_id}") })
}

/// `agents.result { agentId }` → the final report text when done; a lifecycle
/// marker (`running` / `queued` / `killed` / `error`) otherwise; or unknown-id /
/// session-closed. Same registry-scoped resolution as [`broker_status`].
fn broker_result(state: &AppState, ext_id: &str, params: &Value) -> Value {
    let Some(ext_agent_id) = parse_ext_agent_id(params) else {
        return json!({ "error": "agents.result requires an 'agentId'" });
    };
    let Some(r) = state
        .rest
        .ext_agents
        .get(ext_id)
        .and_then(|reg| reg.get(ext_agent_id))
    else {
        return json!({ "error": format!("unknown agentId: {ext_agent_id}") });
    };
    let Some(sess) = resolve_ext_session(state, &r.session_uuid) else {
        return json!({ "error": "session closed" });
    };
    if let Some(sa) = sess.subagents.iter().find(|s| s.id == r.local_subagent_id) {
        return match &sa.status {
            SubAgentStatus::Done(result) => {
                json!({ "agentId": ext_agent_id, "status": "done", "output": result })
            }
            SubAgentStatus::Error(e) => {
                json!({ "agentId": ext_agent_id, "status": "error", "error": e })
            }
            SubAgentStatus::Killed => json!({ "agentId": ext_agent_id, "status": "killed" }),
            SubAgentStatus::Running => json!({ "agentId": ext_agent_id, "status": "running" }),
        };
    }
    if sess.pending_subagents.iter().any(|p| p.id == r.local_subagent_id) {
        return json!({ "agentId": ext_agent_id, "status": "queued" });
    }
    json!({ "error": format!("unknown agentId: {ext_agent_id}") })
}

/// `agents.kill { agentId }` → abort the spawned sub-agent's task and mark it
/// `Killed` (or drop it from the pending queue if it never started). Resolved
/// ONLY through THIS extension's own [`ExtAgentRegistry`] (same containment as
/// `status`/`result`), so an extension can never kill a sub-agent it didn't
/// spawn — not another session's, not another extension's, not the user's own.
/// Returns `{ "killed": true }` or an unknown-id / session-closed error.
/// Idempotent: killing an already-killed id again still reports `killed: true`
/// (the registry entry is deliberately left in place rather than removed, so a
/// later `status`/`result` on the same id keeps reporting the terminal
/// `Killed` outcome instead of flipping to "unknown agentId").
fn broker_kill(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let Some(ext_agent_id) = parse_ext_agent_id(params) else {
        return json!({ "error": "agents.kill requires an 'agentId'" });
    };
    let Some(r) = state
        .rest
        .ext_agents
        .get(ext_id)
        .and_then(|reg| reg.get(ext_agent_id))
        .cloned()
    else {
        return json!({ "error": format!("unknown agentId: {ext_agent_id}") });
    };
    let Some(sess_idx) = state
        .rest
        .sessions
        .iter()
        .position(|s| s.id == r.session_uuid && !s.closed)
    else {
        return json!({ "error": "session closed" });
    };

    let mut killed = false;

    if let Some(sa) = state.rest.sessions[sess_idx]
        .subagents
        .iter_mut()
        .find(|s| s.id == r.local_subagent_id)
    {
        sa.abort.abort();
        // Only transition a still-running agent; a terminal one keeps its outcome.
        if matches!(sa.status, SubAgentStatus::Running) {
            sa.status = SubAgentStatus::Killed;
        }
        killed = true;
    }

    if !killed {
        // Not spawned yet — drop a matching queued delegation. Extension spawns
        // carry no tool_call_id, so there is no parked round to unblock.
        let before = state.rest.sessions[sess_idx].pending_subagents.len();
        state.rest.sessions[sess_idx]
            .pending_subagents
            .retain(|p| p.id != r.local_subagent_id);
        killed = state.rest.sessions[sess_idx].pending_subagents.len() != before;
    }

    if killed {
        // Persist the terminal transition so a restored session doesn't show a
        // stale "running" (mirrors `drain_subagents`' status-change persist).
        crate::app::runtime::bg_persist::persist_subagents(&state.rest.sessions[sess_idx]);
        json!({ "killed": true })
    } else {
        json!({ "error": format!("unknown agentId: {ext_agent_id}") })
    }
}

/// The wire status label for a [`SubAgentStatus`] — matches the snapshot
/// projection's vocabulary (`running` / `done` / `killed` / `error`).
fn status_label(s: &SubAgentStatus) -> &'static str {
    match s {
        SubAgentStatus::Running => "running",
        SubAgentStatus::Done(_) => "done",
        SubAgentStatus::Killed => "killed",
        SubAgentStatus::Error(_) => "error",
    }
}

/// Read `params.agentId` as an EXT-FACING agent id (a key into the calling
/// extension's own [`ExtAgentRegistry`], `u64`) — accepting either a JSON
/// number (the canonical form koma emits) or a numeric string (tolerant of an
/// extension that stringifies ids). `None` when absent / unparseable. Distinct
/// from the raw per-session `usize` sub-agent id, which an extension never
/// sees directly.
fn parse_ext_agent_id(params: &Value) -> Option<u64> {
    match params.get("agentId") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::mode::Mode;
    use crate::app::state::AppState;
    use crate::app::subagent::{SubAgent, SubAgentStatus};

    /// EXHAUSTIVE grant-gate truth table — the security boundary, tested pure (no
    /// state). Columns: granted set × method → expected [`GateDecision`].
    #[test]
    fn grant_gate_truth_table() {
        use Grant::{AgentsOrchestrate as Orch, AgentsRead as Read};

        // Every recognised method partitioned by the grant it requires.
        let orchestrate_methods = ["agents.spawn", "agents.kill"];
        let read_methods = ["agents.list", "agents.status", "agents.result"];

        // granted = []  → EVERYTHING denied.
        for m in orchestrate_methods {
            assert_eq!(method_permitted(m, &[]), GateDecision::Deny(Orch), "empty grants must deny {m}");
        }
        for m in read_methods {
            assert_eq!(method_permitted(m, &[]), GateDecision::Deny(Read), "empty grants must deny {m}");
        }

        // granted = [AgentsRead] → read methods ALLOW, orchestrate methods DENY.
        for m in read_methods {
            assert_eq!(method_permitted(m, &[Read]), GateDecision::Allow, "read grant must allow {m}");
        }
        for m in orchestrate_methods {
            assert_eq!(
                method_permitted(m, &[Read]),
                GateDecision::Deny(Orch),
                "read grant must NOT allow orchestrate method {m}"
            );
        }

        // granted = [AgentsOrchestrate] → EVERYTHING allowed (orchestrate implies read).
        for m in orchestrate_methods.iter().chain(read_methods.iter()) {
            assert_eq!(
                method_permitted(m, &[Orch]),
                GateDecision::Allow,
                "orchestrate grant must allow {m} (implies read)"
            );
        }

        // granted = [Read, Orchestrate] → EVERYTHING allowed.
        for m in orchestrate_methods.iter().chain(read_methods.iter()) {
            assert_eq!(method_permitted(m, &[Read, Orch]), GateDecision::Allow, "full grants must allow {m}");
        }

        // An unrecognised verb is never a silent allow.
        assert_eq!(method_permitted("agents.bogus", &[Orch]), GateDecision::UnknownMethod);
        assert_eq!(method_permitted("filesystem.read", &[Orch]), GateDecision::UnknownMethod);
    }

    /// `parse_grants` maps known wire strings and drops unknown ones (fail-closed).
    #[test]
    fn parse_grants_maps_known_and_drops_unknown() {
        let g = parse_grants(&[
            "agents:read".to_string(),
            "agents:orchestrate".to_string(),
            "filesystem:write".to_string(),
        ]);
        assert_eq!(g, vec![Grant::AgentsRead, Grant::AgentsOrchestrate]);
        assert!(parse_grants(&["nonsense".to_string()]).is_empty());
    }

    /// Build a minimal single-session [`AppState`] fixture for the broker
    /// integration tests (mirrors the `AppState::new(Mode::Chat)` pattern the
    /// daemon tests use — one live foreground session, no creds/client).
    fn fixture_state() -> AppState {
        AppState::new(Mode::Chat)
    }

    /// Fabricate an INERT sub-agent record in a known state (mirrors
    /// `bg_persist::restore_*` / `client_shadow::shadow_subagent`): an abort handle
    /// for a task that finishes at once and a never-written receiver, so no real
    /// model/loop is needed.
    fn inert_subagent(
        handle: &tokio::runtime::Handle,
        id: usize,
        agent_name: &str,
        status: SubAgentStatus,
    ) -> SubAgent {
        let abort = handle.spawn(std::future::ready(())).abort_handle();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        SubAgent {
            id,
            agent_name: agent_name.to_string(),
            label: agent_name.to_string(),
            model_id: String::new(),
            status,
            abort,
            rx,
            transcript: Vec::new(),
            messages: Vec::new(),
            live_text: String::new(),
            tool_call_id: None,
            detached: false,
            nudged: false,
            usage_tokens_in: 0,
            usage_tokens_out: 0,
            usage_cost: 0.0,
        }
    }

    /// CRITICAL: with only `AgentsRead`, `agents.spawn` (needs orchestrate) is
    /// grant-denied and NOTHING is spawned.
    #[test]
    fn spawn_denied_without_orchestrate_grant() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        let out = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.spawn",
            json!({ "task": "do a thing" }),
        );

        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "read-only grant must be denied for agents.spawn, got {out}"
        );
        assert!(
            state.rest.sessions[0].subagents.is_empty(),
            "a denied spawn must NOT create a sub-agent"
        );
        assert!(
            state.rest.sessions[0].pending_subagents.is_empty(),
            "a denied spawn must NOT queue a sub-agent"
        );
    }

    /// With `AgentsOrchestrate`, `agents.spawn` is NOT denied — it reaches the
    /// spawn path (which fails only because the test fixture has no client/session,
    /// proving the gate let it through rather than blocking it).
    #[test]
    fn spawn_reaches_path_with_orchestrate_grant() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        let out = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsOrchestrate],
            "agents.spawn",
            json!({ "task": "do a thing" }),
        );

        let err = out.get("error").and_then(|e| e.as_str()).unwrap_or("");
        assert!(
            !err.contains("grant denied"),
            "orchestrate grant must NOT be denied, got {out}"
        );
        assert!(
            err.contains("failed to spawn"),
            "with orchestrate granted the call reaches the spawn path (which fails for \
             lack of a client/session in the fixture), got {out}"
        );
    }

    /// With no grants at all, `agents.list` (needs read) is denied.
    #[test]
    fn list_denied_without_any_grant() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        let out = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[],
            "agents.list",
            json!({}),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "empty grants must deny agents.list, got {out}"
        );
    }

    /// list/status against a fabricated session with two sub-agents in known
    /// states → assert the JSON shape + statuses (read-granted, so allowed).
    /// Sub-agents are fabricated directly into `subagents` (as before), but now
    /// registered into the extension's own [`ExtAgentRegistry`] (as `agents.spawn`
    /// itself would have done) so the ext-facing `agentId`s the assertions use are
    /// resolved the same way production code resolves them.
    #[test]
    fn list_and_status_report_fabricated_subagents() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        state.rest.sessions[0]
            .subagents
            .push(inert_subagent(rt.handle(), 7, "general", SubAgentStatus::Running));
        state.rest.sessions[0].subagents.push(inert_subagent(
            rt.handle(),
            9,
            "researcher",
            SubAgentStatus::Done("the answer is 42".to_string()),
        ));
        let sess_uuid = state.rest.sessions[0].id.clone();
        let registry = state.rest.ext_agents.entry("test.ext".to_string()).or_default();
        let ext_id_running = registry.insert(sess_uuid.clone(), 7);
        let ext_id_done = registry.insert(sess_uuid, 9);

        // agents.list → array of {agentId, agent, status}, oldest-registered first.
        let list = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.list",
            json!({}),
        );
        let arr = list.as_array().expect("agents.list returns an array");
        assert_eq!(arr.len(), 2, "both fabricated sub-agents are listed");
        assert_eq!(arr[0]["agentId"], json!(ext_id_running));
        assert_eq!(arr[0]["agent"], json!("general"));
        assert_eq!(arr[0]["status"], json!("running"));
        assert_eq!(arr[1]["agentId"], json!(ext_id_done));
        assert_eq!(arr[1]["status"], json!("done"));

        // agents.status on the running one.
        let st = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.status",
            json!({ "agentId": ext_id_running }),
        );
        assert_eq!(st["agentId"], json!(ext_id_running));
        assert_eq!(st["status"], json!("running"));

        // agents.result on the done one → its final report text.
        let res = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.result",
            json!({ "agentId": ext_id_done }),
        );
        assert_eq!(res["status"], json!("done"));
        assert_eq!(res["output"], json!("the answer is 42"));

        // Unknown id → error (not a panic, not a silent empty). Note this is an
        // ext-facing id this extension was NEVER handed, not a raw local id.
        let unknown = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.status",
            json!({ "agentId": 999 }),
        );
        assert!(
            unknown.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("unknown agentId")),
            "unknown agentId must be an error, got {unknown}"
        );
    }

    /// `agents.kill` needs orchestrate: read-only is denied; with orchestrate a
    /// known (registry-resolved) id is killed and marked `Killed`.
    #[test]
    fn kill_gated_by_orchestrate_then_marks_killed() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;
        state.rest.sessions[0]
            .subagents
            .push(inert_subagent(rt.handle(), 3, "general", SubAgentStatus::Running));
        let sess_uuid = state.rest.sessions[0].id.clone();
        let ext_agent_id = state
            .rest
            .ext_agents
            .entry("test.ext".to_string())
            .or_default()
            .insert(sess_uuid, 3);

        // Read-only → denied (no kill).
        let denied = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.kill",
            json!({ "agentId": ext_agent_id }),
        );
        assert!(
            denied.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "kill must require orchestrate, got {denied}"
        );
        assert!(
            matches!(state.rest.sessions[0].subagents[0].status, SubAgentStatus::Running),
            "a denied kill must leave the sub-agent running"
        );

        // Orchestrate → killed. (No session dir on the fixture, so the persist is a
        // best-effort no-op — the in-memory status transition is what we assert.)
        let killed = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsOrchestrate],
            "agents.kill",
            json!({ "agentId": ext_agent_id }),
        );
        assert_eq!(killed, json!({ "killed": true }));
        assert!(
            matches!(state.rest.sessions[0].subagents[0].status, SubAgentStatus::Killed),
            "an orchestrated kill marks the sub-agent Killed"
        );
    }

    /// CRITICAL regression test for the containment fix this wave lands: fabricate
    /// TWO sessions with DIFFERENT stable uuids, each holding a sub-agent at the
    /// SAME raw local id (proving that id can collide across sessions), and
    /// register the extension's ext-facing agent id against session A's entry
    /// only — exactly what `agents.spawn` would have done. Foreground session B
    /// (the scenario the review flagged: a foreground switch between an
    /// extension's spawn and its next poll), then assert `agents.status` /
    /// `agents.kill` for that ext-facing id still resolve to SESSION A's
    /// sub-agent — never session B's, despite B being foreground and sharing the
    /// same local id — and that an id this extension was never handed (including
    /// the bare raw local id, which was never itself an ext-facing id) is
    /// rejected as `"unknown agentId"` rather than silently matched against
    /// whatever session happens to be foreground.
    #[test]
    fn cross_session_isolation_resolves_by_spawn_registry_not_foreground() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        // Session A (fixture's sole session, index 0): the extension's REAL spawn
        // target, local id 5, Running.
        state.rest.sessions[0]
            .subagents
            .push(inert_subagent(rt.handle(), 5, "general", SubAgentStatus::Running));
        let session_a_uuid = state.rest.sessions[0].id.clone();

        // Session B: an unrelated (e.g. user-spawned) second session that happens
        // to have a sub-agent at the SAME local id 5, in a different agent/status
        // so a misrouted read is immediately detectable.
        let mut session_b = SessionRuntime::new();
        session_b.subagents.push(inert_subagent(
            rt.handle(),
            5,
            "researcher",
            SubAgentStatus::Done("session B's secret output".to_string()),
        ));
        let session_b_uuid = session_b.id.clone();
        assert_ne!(
            session_a_uuid, session_b_uuid,
            "fixture sessions must have distinct stable uuids"
        );
        state.rest.sessions.push(session_b);

        // Register the extension's ext-facing id against SESSION A only.
        let ext_agent_id = state
            .rest
            .ext_agents
            .entry("test.ext".to_string())
            .or_default()
            .insert(session_a_uuid, 5);

        // Foreground switches to SESSION B.
        state.rest.foreground = 1;

        // agents.status must resolve to session A's sub-agent (Running/general),
        // never session B's (Done/researcher), despite B now being foreground.
        let status = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.status",
            json!({ "agentId": ext_agent_id }),
        );
        assert_eq!(status["agentId"], json!(ext_agent_id));
        assert_eq!(
            status["agent"], json!("general"),
            "must resolve session A's sub-agent, not session B's, got {status}"
        );
        assert_eq!(
            status["status"], json!("running"),
            "must resolve session A's sub-agent, not session B's, got {status}"
        );

        // agents.kill on the same id must land on session A's sub-agent only.
        let killed = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsOrchestrate],
            "agents.kill",
            json!({ "agentId": ext_agent_id }),
        );
        assert_eq!(killed, json!({ "killed": true }));
        assert!(
            matches!(state.rest.sessions[0].subagents[0].status, SubAgentStatus::Killed),
            "kill must land on session A's sub-agent"
        );
        assert!(
            matches!(state.rest.sessions[1].subagents[0].status, SubAgentStatus::Done(_)),
            "session B's unrelated sub-agent (same raw local id) must be untouched"
        );

        // An agentId this extension was never handed — including the bare raw
        // local id 5, which was never itself an ext-facing id — must be rejected.
        let unknown = handle_ext_call(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsRead],
            "agents.status",
            json!({ "agentId": 5 }),
        );
        assert_eq!(
            unknown.get("error").and_then(|e| e.as_str()),
            Some(format!("unknown agentId: {}", 5).as_str()),
            "an id never handed to this extension must be rejected, got {unknown}"
        );
    }
}
