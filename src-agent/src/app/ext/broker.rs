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
//! is live) by `drain_ext_calls`, which hands the whole request to
//! [`handle_ext_call`] here — the broker itself now replies over the request's
//! `reply` oneshot (it takes `req` BY VALUE so that oneshot can later move into a
//! spawned task for the async verbs). This mirrors the `drain_oauth`
//! background→event-loop hand-off.
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

use crate::app::runtime::{
    handle_live_switch, list_live_sessions, spawn_into_session, spawn_or_queue, SpawnIntoReply,
    SpawnOutcome,
};
use crate::app::state::{AppState, SessionRuntime, EXT_TURN_BUDGET};
use crate::app::subagent::SubAgentStatus;
use crate::ipc::proto::{ClientRequest, SessionStatus};
use crate::model::app_config::ModelRole;
use crate::model::session_registry::RegRow;
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
    /// remember where it really lives (+ whether this extension asked to be
    /// notified on completion). Returns the new ext-facing id.
    ///
    /// `pub(crate)` so the W5 event-fan-out tests (`app::ext::events`) can build
    /// registry fixtures the same way `agents.spawn` populates them, without
    /// re-implementing id allocation.
    pub(crate) fn insert(&mut self, session_uuid: String, local_subagent_id: usize, notify: bool) -> u64 {
        let ext_agent_id = self.next_id;
        self.next_id += 1;
        self.map.insert(
            ext_agent_id,
            ExtAgentRef {
                session_uuid,
                local_subagent_id,
                notify,
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

    /// Resolve a sub-agent's LOCATION — `(session_uuid, local_subagent_id)`, the
    /// identity a spawn/drain site observes — back to `(ext-facing id, notify)`.
    /// Consumed by the W5 event wave to correlate a terminating sub-agent with
    /// the extension that spawned it (and whether it asked for an `agents.done`
    /// notification), without that drain site needing to know anything about
    /// ext-facing ids itself. Oldest-registered entry wins on a duplicate
    /// `(session_uuid, local_subagent_id)` pair (should not happen in practice —
    /// ids are never reused — but this picks a stable winner over an arbitrary
    /// `HashMap` iteration order).
    pub(crate) fn find_by_location(&self, session_uuid: &str, local_id: usize) -> Option<(u64, bool)> {
        self.entries_sorted()
            .into_iter()
            .find(|(_, r)| r.session_uuid == session_uuid && r.local_subagent_id == local_id)
            .map(|(id, r)| (id, r.notify))
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
    /// Whether this extension asked `agents.spawn` to notify it (an
    /// `agents.done` event, delivered in the W5 event wave) when this
    /// sub-agent reaches a terminal state. `false` = poll-only (the extension
    /// calls `agents.status`/`agents.result` itself), today's only behavior.
    notify: bool,
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

/// An ext→koma broker call awaiting dispatch on the event loop.
///
/// Built by the extension's [`reader_task`](super::wire::reader_task) (which has
/// no [`AppState`] access) and pushed onto `AppStateRest::ext_call_tx`; drained
/// each tick by `drain_ext_calls`, which hands the whole request to
/// [`handle_ext_call`] — the broker replies over `reply` itself (it owns the
/// oneshot, so a future async verb can move it into a spawned task). Carries the
/// extension's `granted` scopes so the gate is evaluated against exactly what koma
/// extended to THIS extension.
pub struct ExtCallRequest {
    /// The calling extension's id (for logging / diagnostics).
    pub ext_id: String,
    /// The scopes koma granted this extension (its manifest `requires`, echoed at
    /// handshake). The grant gate is evaluated against this set.
    pub granted: Vec<Grant>,
    /// The canonical method — an `agents.*` verb (`spawn` | `list` | `status` |
    /// `result` | `kill`), or one of the newer `sessions.*` / `chat.prompt` /
    /// `models.invoke` / `context.*` families (see [`is_broker_method`]).
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

/// The grant a broker method requires, or `None` if the method is unknown — an
/// unrecognised verb in a routed family (e.g. `sessions.bogus`) lands on the `None`
/// arm, so the gate reports [`GateDecision::UnknownMethod`] rather than a silent
/// allow.
///
/// `agents.spawn` / `agents.kill` MUTATE the fleet → require
/// [`Grant::AgentsOrchestrate`]. `agents.list` / `agents.status` /
/// `agents.result` only READ → require [`Grant::AgentsRead`] (satisfied by
/// orchestrate too; see [`is_granted`]). Each newer family requires its OWN grant,
/// EXACT-MATCH — no cross-family or lattice implication (orchestrate⇒read stays the
/// sole edge): `sessions.*` → [`Grant::SessionsManage`], `chat.prompt` →
/// [`Grant::ChatPrompt`], `models.invoke` → [`Grant::ModelsInvoke`], `context.set`
/// / `context.clear` → [`Grant::ContextPublish`].
///
/// NOTE: [`Grant::OauthContribute`] (W11) gates NO broker `Call` verb — it gates the
/// OPPOSITE direction (host→ext `oauth.begin`/`oauth.poll`/`oauth.cancel` invokes) plus
/// whether the extension's declared OAuth providers surface as picker rows. So it
/// deliberately has NO arm here (an extension never `Call`s koma to drive an OAuth flow;
/// koma drives the extension).
fn required_grant(method: &str) -> Option<Grant> {
    match method {
        "agents.spawn" | "agents.kill" => Some(Grant::AgentsOrchestrate),
        "agents.list" | "agents.status" | "agents.result" => Some(Grant::AgentsRead),
        "sessions.list" | "sessions.create" | "sessions.switch" | "sessions.spawn_into" => {
            Some(Grant::SessionsManage)
        }
        "chat.prompt" => Some(Grant::ChatPrompt),
        "models.invoke" => Some(Grant::ModelsInvoke),
        "context.set" | "context.clear" => Some(Grant::ContextPublish),
        _ => None,
    }
}

/// Whether `granted` satisfies `required`. Orchestrate IMPLIES read: a
/// read-requiring method is permitted by either `AgentsRead` or
/// `AgentsOrchestrate`; an orchestrate-requiring method needs `AgentsOrchestrate`
/// outright (read alone never grants it). That orchestrate⇒read edge is the SOLE
/// lattice implication — every grant below is EXACT-MATCH: holding one never
/// confers another, and orchestrate never confers any of them.
fn is_granted(granted: &[Grant], required: Grant) -> bool {
    match required {
        Grant::AgentsOrchestrate => granted.contains(&Grant::AgentsOrchestrate),
        Grant::AgentsRead => {
            granted.contains(&Grant::AgentsRead) || granted.contains(&Grant::AgentsOrchestrate)
        }
        // EXACT-MATCH: one grant per family, no implication in or out (see doc above).
        Grant::SessionsManage => granted.contains(&Grant::SessionsManage),
        Grant::ChatPrompt => granted.contains(&Grant::ChatPrompt),
        Grant::ModelsInvoke => granted.contains(&Grant::ModelsInvoke),
        Grant::ContextPublish => granted.contains(&Grant::ContextPublish),
        // W11: exact-match like the rest. `required_grant` never returns this (it
        // gates no broker verb), so this arm is here only for exhaustiveness / a
        // future direct check.
        Grant::OauthContribute => granted.contains(&Grant::OauthContribute),
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

/// Whether `method` belongs to a family this broker owns — the SINGLE SOURCE OF
/// TRUTH shared by the wire router ([`super::wire::reader_task`], which decides
/// whether to hand a `Call` to the broker or answer the wire-level "unknown koma
/// method" stub) and the gate here, so the two can NEVER diverge. Routing keys on
/// the family PREFIX only; the exact-verb allow/deny/unknown decision is
/// [`method_permitted`]'s job — so a routed-but-unrecognised verb (e.g.
/// `sessions.bogus`) flows to the broker and comes back as
/// [`GateDecision::UnknownMethod`], never the wire stub.
pub(crate) fn is_broker_method(method: &str) -> bool {
    const PREFIXES: [&str; 5] = ["agents.", "sessions.", "chat.", "models.", "context."];
    PREFIXES.iter().any(|p| method.starts_with(p))
}

/// The wire string for a [`Grant`] (for error messages / logs). Inverse of
/// [`parse_grants`]'s mapping — keep the two in lock-step.
fn grant_wire(g: Grant) -> &'static str {
    match g {
        Grant::AgentsRead => "agents:read",
        Grant::AgentsOrchestrate => "agents:orchestrate",
        Grant::SessionsManage => "sessions:manage",
        Grant::ChatPrompt => "chat:prompt",
        Grant::ModelsInvoke => "models:invoke",
        Grant::ContextPublish => "context:publish",
        Grant::OauthContribute => "oauth:contribute",
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
            "sessions:manage" => Some(Grant::SessionsManage),
            "chat:prompt" => Some(Grant::ChatPrompt),
            "models:invoke" => Some(Grant::ModelsInvoke),
            "context:publish" => Some(Grant::ContextPublish),
            "oauth:contribute" => Some(Grant::OauthContribute),
            _ => None,
        })
        .collect()
}

/// Dispatch one ext→koma broker [`ExtCallRequest`] against the ACTIVE chat session,
/// gated by the extension's `granted` scopes, and REPLY on the request's `reply`
/// oneshot with the JSON the extension receives as its `KomaMsg::Result`.
///
/// Takes `req` BY VALUE so the `reply` oneshot can move into a spawned task for the async
/// verbs — `models.invoke` and the `sessions.list`/`sessions.create` /
/// `sessions.spawn_into`-cross paths, which touch the network or another daemon's socket and
/// so reply from a `spawn_blocking` task; every other verb replies INLINE before returning.
/// The caller (`drain_ext_calls`) therefore no longer sends the reply itself — this function
/// owns that. `client` is `&mut` because `sessions.switch` rebuilds the keyless client at the
/// session boundary via [`handle_live_switch`].
///
/// GRANT GATE FIRST (the security boundary): a call whose required grant is absent
/// is rejected before ANY session state is read or mutated. Then the active session
/// is resolved; then the verb is dispatched. Never panics — every path replies an
/// `{"error": ...}` object (or a real result) so the extension's `call()` always
/// unblocks. A dropped `reply` receiver (the reader task already timed out) simply
/// discards the reply — never a hang.
pub fn handle_ext_call(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    client: &mut Option<Arc<OpenRouterClient>>,
    req: ExtCallRequest,
) {
    // Own every field: `reply` must be movable into the gate arms (early reply +
    // return) and, for the async verbs a later wave adds, into a spawned task.
    let ExtCallRequest {
        ext_id,
        granted,
        method,
        params,
        reply,
    } = req;

    // 1. GRANT GATE — airtight, and before touching a single session field.
    match method_permitted(&method, &granted) {
        GateDecision::UnknownMethod => {
            let _ = reply.send(json!({ "error": format!("unknown method: {method}") }));
            return;
        }
        GateDecision::Deny(required) => {
            let wire = grant_wire(required);
            // Runtime logging goes to ~/.koma/error.log, never stdout (TUI-safe).
            store::append_global_error_log(
                "extensions",
                &format!("[{ext_id}] grant denied: {method} requires {wire}"),
            );
            let _ =
                reply.send(json!({ "error": format!("grant denied: {method} requires {wire}") }));
            return;
        }
        GateDecision::Allow => {}
    }

    // 2. Dispatch the (now-authorised) verb by family. Most verbs run their real logic and
    // reply INLINE; the ones that touch the network / other daemons' sockets
    // (`models.invoke`, `sessions.list`/`create`/`spawn_into`-cross) validate + resolve on
    // the loop then MOVE the reply oneshot into a spawned task so the event loop never blocks
    // — each inner-bounded well under the reader's 30s cap.
    //
    // `agents.spawn` ALONE resolves the ACTIVE (foreground) session — spawning into
    // "whatever chat session is in front of the user right now" is the intended
    // behavior. Every other `agents.*` verb resolves the sub-agent through THIS
    // extension's own [`ExtAgentRegistry`] instead (never the foreground), so a
    // foreground switch between a spawn and a later poll can never redirect that poll
    // at a different session's sub-agent.
    match method.as_str() {
        "agents.spawn" => {
            let v = match active_session_idx(state) {
                Some(sess_idx) => broker_spawn(state, &ext_id, sess_idx, client, handle, &params),
                None => json!({ "error": "no active session" }),
            };
            let _ = reply.send(v);
        }
        "agents.list" => {
            let _ = reply.send(broker_list(state, &ext_id));
        }
        "agents.status" => {
            let _ = reply.send(broker_status(state, &ext_id, &params));
        }
        "agents.result" => {
            let _ = reply.send(broker_result(state, &ext_id, &params));
        }
        "agents.kill" => {
            let _ = reply.send(broker_kill(state, &ext_id, &params));
        }
        "chat.prompt" => {
            // Resolve the ACTIVE session (the same fallback `agents.spawn` uses), then
            // BUFFER the prompt — never inject from here (that risks corrupting an
            // in-flight turn's tool_call/tool_result ordering). The event-loop
            // `deferred` drain injects the buffer as one synthetic user turn when the
            // session next goes idle. Synchronous reply either way.
            let v = match active_session_idx(state) {
                Some(sess_idx) => broker_chat_prompt(state, &ext_id, sess_idx, &params),
                None => json!({ "error": "no active session" }),
            };
            let _ = reply.send(v);
        }
        "models.invoke" => {
            // Validates + resolves ON the loop, then either replies a sync error
            // inline OR moves `reply` into a spawned one-shot task (25s < the 30s
            // reader cap) that answers on completion — so the model call never
            // blocks the event loop. OWNS the reply from here.
            broker_models_invoke(state, handle, client, &params, reply);
        }
        "context.set" => {
            let _ = reply.send(broker_context_set(state, &ext_id, &params));
        }
        "context.clear" => {
            let _ = reply.send(broker_context_clear(state, &ext_id));
        }
        "sessions.list" => {
            // No sync state needed: enumerate the session registry + probe live daemons OFF
            // the loop (sqlite read + blocking per-socket probes), then reply the merged array.
            // The reader task caps this Call at 30s; the probe sweep is inner-bounded far below.
            handle.spawn_blocking(move || {
                let rows = crate::model::session_registry::list_all().unwrap_or_default();
                let live = list_live_sessions();
                let _ = reply.send(merge_sessions(rows, live));
            });
        }
        "sessions.create" => {
            // Sync validation + uuid mint; then spawn-or-attach the session's own daemon OFF
            // the loop (the daemon create-or-loads the session itself). OWNS the reply.
            broker_sessions_create(handle, &params, reply);
        }
        "sessions.switch" => {
            // Fully SYNC: an in-daemon live session switches foreground here; a non-local
            // uuid latches `ext_switch_pending` for the hub to signal the client next tick.
            let _ = reply.send(broker_sessions_switch(state, client, &params));
        }
        "sessions.spawn_into" => {
            // Sync local-vs-cross decision; the LOCAL branch replies inline, the CROSS branch
            // moves the reply into a `spawn_blocking` that speaks to the target daemon's socket.
            // OWNS the reply either way.
            broker_sessions_spawn_into(state, &ext_id, client, handle, &params, reply);
        }
        // Unreachable: method_permitted already rejected anything else above.
        _ => {
            let _ = reply.send(json!({ "error": format!("unknown method: {method}") }));
        }
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

/// `agents.spawn { task, agent?, model?, effort?, notify? }` → route through the
/// SAME `spawn_or_queue` path the model's `task` tool uses (respecting
/// `MAX_SUBAGENTS` → queue when full), into the ACTIVE (foreground) session.
/// `agent` defaults to [`DEFAULT_AGENT`]. Spawned NON-detached with no
/// tool-call id (the `/task`-command shape) so completion records a display
/// note + usage but never auto-wakes the chat model. The returned `agentId` is
/// an EXT-FACING id freshly allocated from this extension's own
/// [`ExtAgentRegistry`] (never the raw per-session sub-agent id), permanently
/// bound to the session's STABLE UUID — see the registry's doc for why that
/// containment matters.
///
/// `model` / `effort` are optional per-call overrides (see
/// [`crate::app::subagent::SpawnOverrides`]) that steer THIS spawn's route
/// without touching the named agent's own definition; an empty string for
/// either is treated as absent (not an override). `notify` (default `false`)
/// records whether this extension wants an `agents.done` event when the
/// sub-agent finishes (consumed by the W5 event wave via
/// [`ExtAgentRegistry::find_by_location`]) — unused until then, but recorded
/// now so a spawn made this wave doesn't need to be re-issued once W5 lands.
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

    // Optional per-call overrides. An empty string is treated as absent (an
    // extension that sends `"model": ""` should not force an override).
    let non_empty_string = |key: &str| -> Option<String> {
        params
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let model = non_empty_string("model");
    let effort = non_empty_string("effort");
    let overrides = if model.is_some() || effort.is_some() {
        Some(crate::app::subagent::SpawnOverrides { model, effort })
    } else {
        None
    };
    let notify = params.get("notify").and_then(|v| v.as_bool()).unwrap_or(false);

    // Capture the STABLE uuid of the session being spawned into BEFORE
    // `spawn_or_queue` (which needs `state` mutably) — this is the uuid the
    // resulting ext-facing agent id stays bound to regardless of any later
    // foreground switch.
    let session_uuid = state.rest.sessions[sess_idx].id.clone();

    match spawn_or_queue(state, sess_idx, client, handle, agent, task, None, false, overrides) {
        SpawnOutcome::Spawned(local_id) => {
            let ext_agent_id = state
                .rest
                .ext_agents
                .entry(ext_id.to_string())
                .or_default()
                .insert(session_uuid, local_id, notify);
            json!({ "agentId": ext_agent_id, "status": "spawned" })
        }
        SpawnOutcome::Queued(local_id) => {
            let ext_agent_id = state
                .rest
                .ext_agents
                .entry(ext_id.to_string())
                .or_default()
                .insert(session_uuid, local_id, notify);
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
///
/// `"killed": true` means the target was FOUND and is (now) terminal — it does NOT
/// mean this call necessarily caused a state transition. Calling `agents.kill` on a
/// sub-agent that had already settled as `Done`/`Error` (finished on its own before
/// this call arrived) still replies `killed: true`, but that settled outcome is
/// PRESERVED (only a still-`Running` agent is transitioned to `Killed` — see the
/// `matches!(sa.status, SubAgentStatus::Running)` guard below) and no terminal event
/// is re-emitted (the W5 fan-out only fires on a genuine Running→Killed transition,
/// captured in `killed_transition`). A subsequent `agents.status`/`agents.result` on
/// the same id keeps reporting whatever it had already settled to (`done`/`error`),
/// never `killed`.
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
    // W5: capture the fan-out triple ONLY on a genuine Running->Killed transition
    // (never the idempotent re-kill of an already-terminal agent, never the
    // pending-queue drop of one that never ran). Emitted AFTER the &mut kill work
    // below so the &AppState the event fan-out needs is a clean reborrow.
    let mut killed_transition: Option<(String, usize, String)> = None;

    if let Some(sa) = state.rest.sessions[sess_idx]
        .subagents
        .iter_mut()
        .find(|s| s.id == r.local_subagent_id)
    {
        sa.abort.abort();
        // Only transition a still-running agent; a terminal one keeps its outcome.
        if matches!(sa.status, SubAgentStatus::Running) {
            sa.status = SubAgentStatus::Killed;
            killed_transition =
                Some((r.session_uuid.clone(), r.local_subagent_id, sa.agent_name.clone()));
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
        // W5: fan out the terminal event AFTER the mutable kill work. Fires only on
        // a genuine Running->Killed transition (captured above) — a pending-queue
        // drop or an idempotent re-kill of an already-terminal agent emits nothing,
        // and the next `drain_subagents` tick won't re-emit (the agent is no longer
        // Running there, so its was-running edge is false).
        if let Some((session_uuid, local_id, agent)) = killed_transition {
            super::events::emit_subagent_terminal(state, &session_uuid, local_id, &agent, "killed");
        }
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

/// `chat.prompt { text }` → BUFFER `text` into the ACTIVE session's
/// [`pending_ext_prompts`](crate::app::state::SessionRuntime::pending_ext_prompts)
/// for the event loop to inject as one synthetic user turn when the session next
/// goes idle. NEVER injects here (that would risk corrupting an in-flight turn's
/// tool_call/tool_result ordering) — buffer-only, synchronous reply.
///
/// Validation, IN ORDER: `text` trimmed non-empty; `text.len() <= 16384` (16KB);
/// the session's consecutive-injection BUDGET
/// ([`EXT_TURN_BUDGET`](crate::app::state::EXT_TURN_BUDGET), the cost-DoS guard —
/// review finding) must not already be exhausted, else the call is refused outright
/// rather than buffered (this is the "front door" half of the belt-and-braces pair
/// with the deferred injection gate's own budget check — see
/// `event_loop::sessions::deferred::ext_prompts_ready`); a prompt IDENTICAL to the
/// buffer's LAST entry is a consecutive-duplicate and is dropped (reports the
/// unchanged queue length, no growth — checked BEFORE the cap so a repeat never
/// trips it); a buffer already at the cap of 5 rejects further prompts. Otherwise
/// `(ext_id, text)` is pushed. Reply `{ "queued": <len> }` on accept/dedupe, else
/// `{ "error": ... }`.
fn broker_chat_prompt(state: &mut AppState, ext_id: &str, sess_idx: usize, params: &Value) -> Value {
    let text = params
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if text.is_empty() {
        return json!({ "error": "chat.prompt requires a non-empty 'text'" });
    }
    if text.len() > 16_384 {
        return json!({ "error": "prompt exceeds 16KB" });
    }
    // Cost-DoS guard (review finding): once this session has injected
    // EXT_TURN_BUDGET consecutive extension turns with no real user activity in
    // between, refuse to even buffer a further prompt. A prompt already buffered
    // before the budget tripped is untouched (it stays parked — see the deferred
    // injection gate's own belt-and-braces check).
    if state.rest.sessions[sess_idx].ext_injected_turns >= EXT_TURN_BUDGET {
        return json!({ "error": "extension turn budget exhausted; waiting for user activity" });
    }
    let buf = &mut state.rest.sessions[sess_idx].pending_ext_prompts;
    // Consecutive-duplicate dedupe FIRST (before the cap): an extension resending the
    // same text as the last buffered prompt gets the unchanged length back with no
    // append, so it can neither fill the buffer with duplicates nor trip the cap on a
    // repeat. Compared against the LAST entry only, matching "consecutive".
    if buf.last().map(|(_, t)| t.as_str() == text).unwrap_or(false) {
        return json!({ "queued": buf.len() });
    }
    if buf.len() >= 5 {
        return json!({ "error": "prompt queue full (5)" });
    }
    buf.push((ext_id.to_string(), text.to_string()));
    json!({ "queued": buf.len() })
}

/// `models.invoke { role?, system?, prompt }` → a ONE-SHOT completion against the
/// resolved model for `role` (default `"main"`), run OFF the event loop.
///
/// Validated + resolved SYNCHRONOUSLY on the loop: `prompt` non-empty and
/// `<= 32768` bytes; `role` one of `main`/`awareness`/`safeguard`/`compactor`/
/// `planner` (unknown → error); [`Settings`](crate::model::settings::Settings)
/// cloned from the FOREGROUND session (else default); the route resolved via
/// [`resolve_role_dispatch`](crate::app::resolve::resolve_role_dispatch) (koma-free
/// backs Main/Awareness/Safeguard/Compactor; Planner has no fallback → `None` is an
/// error), gated on [`Resolved::is_routable`](crate::app::resolve::Resolved::is_routable)
/// / [`is_usable`](crate::app::resolve::Resolved::is_usable); a client must exist.
/// Any of these fail → a sync `{"error": ...}` reply and NO task is spawned.
///
/// Once validated, an owned `Resolved` + an `Arc` clone of the client + the reply
/// oneshot MOVE into a spawned task (the `spawn_awareness_recompute` pattern) that
/// runs `complete_with` under a 25s `tokio::time::timeout` — 25s deliberately
/// UNDERCUTS the reader task's 30s `EXT_CALL_TIMEOUT` so the extension always
/// receives a value rather than a transport timeout. Reply `{ "output": <text>,
/// "model": <id> }` on success, `{ "error": "model call failed: <e>" }` on a call
/// error, or `{ "error": "model call timed out" }`. The event loop never blocks.
fn broker_models_invoke(
    state: &AppState,
    handle: &tokio::runtime::Handle,
    client: &Option<Arc<OpenRouterClient>>,
    params: &Value,
    reply: tokio::sync::oneshot::Sender<Value>,
) {
    let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    if prompt.trim().is_empty() {
        let _ = reply.send(json!({ "error": "models.invoke requires a non-empty 'prompt'" }));
        return;
    }
    if prompt.len() > 32_768 {
        let _ = reply.send(json!({ "error": "prompt exceeds 32KB" }));
        return;
    }
    // role: default "main"; an unrecognised value is an error (never silently Main).
    let role_str = params
        .get("role")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    let role = match role_str {
        "main" => ModelRole::Main,
        "awareness" => ModelRole::Awareness,
        "safeguard" => ModelRole::Safeguard,
        "compactor" => ModelRole::Compactor,
        "planner" => ModelRole::Planner,
        _ => {
            let _ = reply.send(json!({ "error": "unknown role" }));
            return;
        }
    };
    // Optional system prompt (absent / blank → no System message prepended).
    let system = params
        .get("system")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Settings from the FOREGROUND session (else default) — mirrors how the
    // stream/awareness snapshots obtain a `Settings` before a resolve.
    let settings = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.clone())
        .unwrap_or_default();

    // Resolve the requested role to a concrete dispatch route.
    let route = match crate::app::resolve::resolve_role_dispatch(&state.rest.config, &settings, role)
    {
        Some(r) => r,
        None => {
            let _ = reply.send(json!({ "error": format!("no usable route for role {role_str}") }));
            return;
        }
    };
    if !route.is_routable() {
        let _ = reply.send(json!({
            "error": format!("role {role_str} route is not dispatchable (Anthropic-compatible not wired)")
        }));
        return;
    }
    if !route.is_usable() {
        let _ = reply.send(json!({ "error": format!("role {role_str} route has no usable auth") }));
        return;
    }
    let Some(client_arc) = client.as_ref() else {
        let _ = reply.send(json!({ "error": "no llm client" }));
        return;
    };

    // Owned move-ins for the 'static task (it holds NO borrow of `state`): the
    // resolved route, an `Arc` clone of the client, the owned prompt, and the reply
    // oneshot. The model id is snapshotted for the success reply.
    let client_task = Arc::clone(client_arc);
    let model_id = route.model_id.clone();
    let prompt_owned = prompt.to_string();
    handle.spawn(async move {
        let mut messages: Vec<crate::dto::chat::ChatMessage> = Vec::new();
        if let Some(sys) = system {
            messages.push(crate::dto::chat::ChatMessage::new(crate::dto::chat::Role::System, sys));
        }
        messages.push(crate::dto::chat::ChatMessage::new(
            crate::dto::chat::Role::User,
            prompt_owned,
        ));
        // 25s < the 30s reader cap: the extension always gets a value back.
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            client_task.complete_with(route.conn(), &route.model_id, route.provider(), messages),
        )
        .await;
        let v = match out {
            Ok(Ok(s)) => json!({ "output": s, "model": model_id }),
            Ok(Err(e)) => json!({ "error": format!("model call failed: {e}") }),
            Err(_) => json!({ "error": "model call timed out" }),
        };
        let _ = reply.send(v);
    });
}

/// `context.set { text }` → PUBLISH `text` as this extension's persistent context
/// blob, keyed by the CALLER's `ext_id` (so an extension can only ever read/replace
/// its OWN entry, never another's). The blob rides the System-prompt VOLATILE TAIL
/// on every turn (see `stream::run::append_ext_context`), AFTER the cache split, so
/// it survives compaction without busting the cached head. `text.len() > 8192`
/// (8KB) is rejected; an empty/whitespace `text` REMOVES the entry (publishing
/// "nothing" is a clear). Reply `{ "ok": true }` on any accepted set/remove.
fn broker_context_set(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
    if text.len() > 8_192 {
        return json!({ "error": "context exceeds 8KB" });
    }
    if text.trim().is_empty() {
        state.rest.ext_context.remove(ext_id);
    } else {
        state.rest.ext_context.insert(ext_id.to_string(), text.to_string());
    }
    json!({ "ok": true })
}

/// `context.clear {}` → REMOVE this extension's published context blob (keyed by the
/// caller's `ext_id`; other extensions' blobs are untouched). Idempotent — clearing
/// an absent entry still replies `{ "ok": true }`.
fn broker_context_clear(state: &mut AppState, ext_id: &str) -> Value {
    state.rest.ext_context.remove(ext_id);
    json!({ "ok": true })
}

// ─── sessions.* (W7) ──────────────────────────────────────────────────────────

/// Read `params[key]` as a TRIMMED non-empty owned `String`, else `None` — the shared
/// "empty string is treated as absent" convention the `agents.spawn` overrides use, in an
/// owned form the async `spawn_blocking` bodies can move.
fn non_empty_owned(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Merge the session REGISTRY rows against the currently-LIVE daemons into the
/// `sessions.list` reply array: `[{ id, name, workdir, live, working }]`. PURE (no I/O) so
/// the merge is unit-testable; the caller supplies both data sources.
///
/// Field mapping is best-effort from what each source actually carries:
/// - Registry [`RegRow`]: `id ← uuid`, `name ← name`, `workdir ← workdir`.
/// - A row whose uuid is among the live daemons → `live: true`, `working: <that daemon's
///   working flag>`; a row with no live daemon → `live: false`, `working: false`.
/// - A LIVE daemon with NO registry row (spawned but not yet/ever registered) is still
///   included, keyed by [`SessionStatus`]: `id ← session_id`, `name: null`, `workdir ← pwd`,
///   `live: true`, `working ← working`.
///
/// Registry rows come first (registry order = most-recently-updated first), then any
/// live-but-unregistered sessions, so the list is stable + deterministic.
fn merge_sessions(rows: Vec<RegRow>, live: Vec<SessionStatus>) -> Value {
    let mut out: Vec<Value> = Vec::with_capacity(rows.len() + live.len());
    for row in &rows {
        let live_match = live.iter().find(|s| s.session_id == row.uuid);
        out.push(json!({
            "id": row.uuid,
            "name": row.name,
            "workdir": row.workdir,
            "live": live_match.is_some(),
            "working": live_match.map(|s| s.working).unwrap_or(false),
        }));
    }
    // Live sessions with no registry row: include with a null name (nothing to name them by).
    for s in &live {
        if rows.iter().any(|r| r.uuid == s.session_id) {
            continue;
        }
        out.push(json!({
            "id": s.session_id,
            "name": Value::Null,
            "workdir": s.pwd,
            "live": true,
            "working": s.working,
        }));
    }
    Value::Array(out)
}

/// Map a cross-daemon [`spawn_into_session`] transport failure (an [`std::io::ErrorKind`])
/// to the extension-facing error JSON. PURE — factored out so the async path's failure
/// mapping is unit-testable without a live socket. A refused/absent socket means the target
/// session's daemon is not accepting (`"session not live"`); every other kind (write / read /
/// decode / EOF / timeout / frame-cap) means it accepted the connection but did not speak the
/// expected reply (`"target daemon incompatible or unavailable"`).
fn spawn_into_error_json(kind: std::io::ErrorKind) -> Value {
    use std::io::ErrorKind::{ConnectionRefused, NotFound};
    match kind {
        ConnectionRefused | NotFound => json!({ "error": "session not live" }),
        _ => json!({ "error": "target daemon incompatible or unavailable" }),
    }
}

/// `sessions.switch { session }` → move the user's FOREGROUND to `session`. Fully SYNC.
///
/// If `session` is a LIVE (non-closed) session in THIS daemon's `sessions` Vec, apply the
/// SAME [`handle_live_switch`] chokepoint the hub's `SwitchForeground` uses (foreground
/// repoint + flat-UI reset + keyless-client rebuild) and reply `{ ok, delivery: "local" }`.
/// `handle_live_switch` already fans out `session.foreground_change` (W5), so this must NOT
/// emit it again. Otherwise the target lives in ANOTHER daemon's process: latch
/// `ext_switch_pending` for the hub to broadcast a one-shot `AttachSession` to attached
/// clients next tick, and reply `{ ok, delivery: "signaled" }` (the actual attach is the
/// client's job — GUI wiring lands later; the TUI may ignore it).
fn broker_sessions_switch(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    params: &Value,
) -> Value {
    let Some(uuid) = params
        .get("session")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({ "error": "sessions.switch requires a 'session'" });
    };

    if let Some(target) = state
        .rest
        .sessions
        .iter()
        .position(|s| s.id == uuid && !s.closed)
    {
        // Infallible in practice (the index was just resolved live); ignore the `Result`
        // rather than surface a spurious error. Do NOT emit foreground_change — the switch
        // chokepoint already did.
        let _ = handle_live_switch(target, state, client);
        return json!({ "ok": true, "delivery": "local" });
    }

    state.rest.ext_switch_pending = Some(uuid.to_string());
    json!({ "ok": true, "delivery": "signaled" })
}

/// `sessions.create { workdir?, name? }` → mint a fresh session uuid and spawn-or-attach ITS
/// OWN session-daemon (which create-or-loads the session itself — never pre-created here).
/// OWNS `reply`.
///
/// SYNC on the loop: validate `workdir` (present ⇒ must be an absolute, existing path), mint
/// the uuid, capture the optional `name`. Then MOVE the reply into a `spawn_blocking` that
/// calls [`ensure_daemon_running`](crate::app::runtime::ensure_daemon_running) (blocking:
/// spawn a detached `koma --daemon --session <uuid>` and poll-connect until it accepts,
/// bounded by its own `SPAWN_CONNECT_TIMEOUT` of 3s — well under the reader's 30s cap, so no
/// extra outer timer is needed). On success, best-effort set the display `name` (the daemon
/// registers its registry row during startup, which can lag the socket coming up, so retry
/// once after a short sleep if the row isn't there yet; a failure to name is NOT an error).
/// Reply `{ id }` on success, `{ error }` on a spawn failure.
/// Validate the optional `workdir` of a `sessions.create`. PURE (path metadata only) so the
/// sync validation is unit-testable: `None` when absent/blank (the daemon buckets the new
/// session under its own launch cwd), `Ok(Some(path))` when present AND an absolute, existing
/// directory, or `Err(error json)` when present but relative / non-existent.
fn parse_create_workdir(params: &Value) -> Result<Option<std::path::PathBuf>, Value> {
    match params
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(w) => {
            let p = std::path::Path::new(w);
            if !p.is_absolute() || !p.exists() {
                return Err(json!({ "error": "workdir must be an absolute existing path" }));
            }
            Ok(Some(p.to_path_buf()))
        }
        None => Ok(None),
    }
}

fn broker_sessions_create(
    handle: &tokio::runtime::Handle,
    params: &Value,
    reply: tokio::sync::oneshot::Sender<Value>,
) {
    let workdir = match parse_create_workdir(params) {
        Ok(w) => w,
        Err(e) => {
            let _ = reply.send(e);
            return;
        }
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let name = non_empty_owned(params, "name");

    handle.spawn_blocking(move || {
        let v = match crate::app::runtime::ensure_daemon_running(&uuid, false, workdir.as_deref()) {
            Ok(()) => {
                if let Some(name) = name {
                    // The spawned daemon registers its registry row during startup, which can
                    // lag the socket accepting. Best-effort: name it; if the row isn't there
                    // yet, wait once and retry. A failure to name is NOT a create failure.
                    let _ = crate::model::session_registry::set_name(&uuid, &name);
                    if crate::model::session_registry::get(&uuid).ok().flatten().is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        let _ = crate::model::session_registry::set_name(&uuid, &name);
                    }
                }
                json!({ "id": uuid })
            }
            Err(e) => json!({ "error": format!("{e:#}") }),
        };
        let _ = reply.send(v);
    });
}

/// `sessions.spawn_into { session, task, agent?, model?, effort?, notify? }` → spawn a
/// sub-agent into `session`. OWNS `reply`.
///
/// Validate `session` + a non-empty `task` up front (both branches need them). If `session`
/// is a LIVE session in THIS daemon, take the SYNC LOCAL branch: reuse [`broker_spawn`] with
/// that session's index — the SAME `spawn_or_queue` path + W4 overrides + notify registry
/// binding `agents.spawn` uses — so the reply carries an ext-facing `agentId` (poll-able via
/// `agents.status`/`result`), identical in shape to `agents.spawn`.
///
/// Otherwise the target lives in ANOTHER daemon's process: MOVE the reply into a
/// `spawn_blocking` that fires a one-shot [`ClientRequest::SpawnAgent`] at the target's keyed
/// socket via [`spawn_into_session`] (no attach, no streaming, no retry, no auto-spawn). Reply
/// `{ status: "sent", session }` on Ack, `{ error }` on the target's Error, or a
/// [`spawn_into_error_json`]-mapped transport error. NOTE: the ext-facing [`ExtAgentRegistry`]
/// does NOT track cross-process spawns — v1 has no cross-daemon polling, so a cross spawn
/// returns no `agentId`.
fn broker_sessions_spawn_into(
    state: &mut AppState,
    ext_id: &str,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    params: &Value,
    reply: tokio::sync::oneshot::Sender<Value>,
) {
    let Some(uuid) = params
        .get("session")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        let _ = reply.send(json!({ "error": "sessions.spawn_into requires a 'session'" }));
        return;
    };
    let task = params.get("task").and_then(|v| v.as_str()).unwrap_or("").trim();
    if task.is_empty() {
        let _ = reply.send(json!({ "error": "sessions.spawn_into requires a non-empty 'task'" }));
        return;
    }

    // LOCAL: a live session in THIS daemon → reuse the agents.spawn path with THAT index.
    if let Some(sess_idx) = state
        .rest
        .sessions
        .iter()
        .position(|s| s.id == uuid && !s.closed)
    {
        let v = broker_spawn(state, ext_id, sess_idx, client, handle, params);
        let _ = reply.send(v);
        return;
    }

    // CROSS-PROCESS: fire-and-forget over the target daemon's socket, OFF the loop.
    let session = uuid.to_string();
    let agent = non_empty_owned(params, "agent");
    let model = non_empty_owned(params, "model");
    let effort = non_empty_owned(params, "effort");
    let task_owned = task.to_string();
    handle.spawn_blocking(move || {
        let v = match store::daemon_sock_path(&session) {
            Ok(path) => {
                let req = ClientRequest::SpawnAgent {
                    agent,
                    task: task_owned,
                    model,
                    effort,
                };
                match spawn_into_session(&path, &req) {
                    Ok(SpawnIntoReply::Accepted) => json!({ "status": "sent", "session": session }),
                    Ok(SpawnIntoReply::Rejected(msg)) => json!({ "error": msg }),
                    Err(e) => spawn_into_error_json(e.kind()),
                }
            }
            // Resolving the session's socket path failed (base-dir error): treat as an
            // unavailable target rather than a hang.
            Err(_) => json!({ "error": "target daemon incompatible or unavailable" }),
        };
        let _ = reply.send(v);
    });
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

        // --- WAVE-3 families: each needs its OWN grant, EXACT-MATCH, no lattice edge.
        use Grant::{ChatPrompt, ContextPublish, ModelsInvoke, SessionsManage};

        // (every verb in a family, the grant that family requires).
        let new_families: [(&[&str], Grant); 4] = [
            (
                &["sessions.list", "sessions.create", "sessions.switch", "sessions.spawn_into"],
                SessionsManage,
            ),
            (&["chat.prompt"], ChatPrompt),
            (&["models.invoke"], ModelsInvoke),
            (&["context.set", "context.clear"], ContextPublish),
        ];

        for (methods, own) in new_families {
            // An UNRELATED grant to probe cross-family isolation (never `own`).
            let unrelated = if own == SessionsManage { ChatPrompt } else { SessionsManage };
            for m in methods {
                // No grants → denied (missing its own grant).
                assert_eq!(
                    method_permitted(m, &[]),
                    GateDecision::Deny(own),
                    "empty grants must deny {m}"
                );
                // Its OWN grant → allowed.
                assert_eq!(
                    method_permitted(m, &[own]),
                    GateDecision::Allow,
                    "{m} must be allowed under its own grant"
                );
                // An unrelated grant → still denied (exact-match, no cross-family unlock).
                assert_eq!(
                    method_permitted(m, &[unrelated]),
                    GateDecision::Deny(own),
                    "an unrelated grant must NOT unlock {m}"
                );
                // agents:orchestrate is the ONLY lattice edge and it stops at agents.* —
                // it must NOT unlock any new family.
                assert_eq!(
                    method_permitted(m, &[Orch]),
                    GateDecision::Deny(own),
                    "orchestrate must NOT unlock the new-family method {m}"
                );
            }
        }

        // A bogus verb in each new family is UnknownMethod even when the family grant
        // IS held — the gate keys on the exact verb, never the family prefix.
        assert_eq!(
            method_permitted("sessions.bogus", &[SessionsManage]),
            GateDecision::UnknownMethod
        );
        assert_eq!(method_permitted("chat.bogus", &[ChatPrompt]), GateDecision::UnknownMethod);
        assert_eq!(method_permitted("models.bogus", &[ModelsInvoke]), GateDecision::UnknownMethod);
        assert_eq!(
            method_permitted("context.bogus", &[ContextPublish]),
            GateDecision::UnknownMethod
        );
    }

    /// `parse_grants` maps known wire strings and drops unknown ones (fail-closed).
    #[test]
    fn parse_grants_maps_known_and_drops_unknown() {
        let g = parse_grants(&[
            "agents:read".to_string(),
            "agents:orchestrate".to_string(),
            "sessions:manage".to_string(),
            "chat:prompt".to_string(),
            "models:invoke".to_string(),
            "context:publish".to_string(),
            "oauth:contribute".to_string(),
            "filesystem:write".to_string(),
        ]);
        // Input order is preserved; the unknown "filesystem:write" is dropped.
        assert_eq!(
            g,
            vec![
                Grant::AgentsRead,
                Grant::AgentsOrchestrate,
                Grant::SessionsManage,
                Grant::ChatPrompt,
                Grant::ModelsInvoke,
                Grant::ContextPublish,
                Grant::OauthContribute,
            ]
        );
        assert!(parse_grants(&["nonsense".to_string()]).is_empty());

        // Round-trip lock-step with `grant_wire` for every parsed grant.
        for grant in &g {
            assert_eq!(parse_grants(&[grant_wire(*grant).to_string()]), vec![*grant]);
        }

        // W11: `oauth:contribute` gates no broker verb, so the gate still treats every
        // `oauth.*` method as UnknownMethod even when the grant is held (it is not a
        // broker `Call` family at all — koma drives the extension, not the reverse).
        assert_eq!(
            method_permitted("oauth.begin", &[Grant::OauthContribute]),
            GateDecision::UnknownMethod
        );
        assert!(!is_broker_method("oauth.begin"));
    }

    /// `is_broker_method` recognises every broker family prefix (one representative
    /// verb each) and nothing else — the single source of truth the wire router and
    /// the gate both consult, so they can never disagree on what routes here.
    #[test]
    fn is_broker_method_covers_all_families() {
        for m in [
            "agents.spawn",
            "sessions.list",
            "chat.prompt",
            "models.invoke",
            "context.set",
        ] {
            assert!(is_broker_method(m), "{m} must route to the broker");
        }
        // Other ext→koma families (panel/tool) and the empty method must NOT route here.
        for m in ["tool.call", "panel.msg", ""] {
            assert!(!is_broker_method(m), "{m:?} must NOT route to the broker");
        }
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

    /// Drive [`handle_ext_call`] for one method and return the JSON it replies on
    /// the request's oneshot. Every arm THESE TESTS exercise replies INLINE (no task
    /// spawn) — `models.invoke` only moves the reply into a spawned task once a
    /// client AND a resolvable route exist, which no fixture here provides, so the
    /// oneshot is always fulfilled by the time `handle_ext_call` returns and
    /// `try_recv` never races — restoring the old `-> Value` test ergonomics after
    /// the by-value / async-ready signature change, with identical semantics.
    fn call_broker(
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        client: &Option<Arc<OpenRouterClient>>,
        ext_id: &str,
        granted: &[Grant],
        method: &str,
        params: Value,
    ) -> Value {
        let (reply, mut reply_rx) = tokio::sync::oneshot::channel::<Value>();
        let req = ExtCallRequest {
            ext_id: ext_id.to_string(),
            granted: granted.to_vec(),
            method: method.to_string(),
            params,
            reply,
        };
        // `handle_ext_call` now takes `&mut Option<..>` (W7 `sessions.switch` rebuilds the
        // keyless client at the session boundary via `handle_live_switch`). No test here
        // observes that rebuild — they assert on `state` + the reply — so a throwaway clone
        // keeps every existing call site passing `&client` unchanged.
        let mut client_local = client.clone();
        handle_ext_call(state, handle, &mut client_local, req);
        reply_rx
            .try_recv()
            .expect("broker must reply inline on the oneshot in this wave")
    }

    /// CRITICAL: with only `AgentsRead`, `agents.spawn` (needs orchestrate) is
    /// grant-denied and NOTHING is spawned.
    #[test]
    fn spawn_denied_without_orchestrate_grant() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        let out = call_broker(
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

        let out = call_broker(
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

        let out = call_broker(
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
        let ext_id_running = registry.insert(sess_uuid.clone(), 7, false);
        let ext_id_done = registry.insert(sess_uuid, 9, false);

        // agents.list → array of {agentId, agent, status}, oldest-registered first.
        let list = call_broker(
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
        let st = call_broker(
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
        let res = call_broker(
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
        let unknown = call_broker(
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
            .insert(sess_uuid, 3, false);

        // Read-only → denied (no kill).
        let denied = call_broker(
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
        let killed = call_broker(
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
            .insert(session_a_uuid, 5, false);

        // Foreground switches to SESSION B.
        state.rest.foreground = 1;

        // agents.status must resolve to session A's sub-agent (Running/general),
        // never session B's (Done/researcher), despite B now being foreground.
        let status = call_broker(
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
        let killed = call_broker(
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
        let unknown = call_broker(
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

    /// A NEW-family verb is gated by its OWN grant and, once allowed, reaches its
    /// real handler (the W6/W7 bodies are now implemented, so there is no longer a
    /// not-implemented stub). Proves the gate-first invariant holds for the new
    /// families exactly as for `agents.*`: ungranted is denied BEFORE any dispatch,
    /// an unrelated grant never unlocks it, and the reply travels back over the
    /// request's `reply` oneshot.
    #[test]
    fn new_family_verbs_gate_first_then_reach_handler() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        // Ungranted → grant denied (never reaches the handler, no state touched).
        let denied = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[],
            "sessions.switch",
            json!({ "session": "x" }),
        );
        assert!(
            denied.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "sessions.switch without sessions:manage must be denied, got {denied}"
        );

        // Granted → passes the gate and reaches the REAL sessions.switch handler, which
        // (with no 'session' param) replies its own validation error INLINE — proving the
        // gate let it through to the implemented body, not a stub and not a denial.
        let reached = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.switch",
            json!({}),
        );
        assert!(
            reached.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("requires a 'session'")),
            "a granted sessions.switch reaches its real handler's validation, got {reached}"
        );

        // Cross-family: orchestrate must NOT unlock chat.prompt (exact-match gate).
        let cross = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::AgentsOrchestrate],
            "chat.prompt",
            json!({}),
        );
        assert!(
            cross.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "orchestrate must NOT unlock chat.prompt, got {cross}"
        );

        // A routed-but-unknown verb in a granted family → UnknownMethod, not the stub.
        let bogus = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.bogus",
            json!({}),
        );
        assert!(
            bogus.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("unknown method")),
            "sessions.bogus must be UnknownMethod even with the family grant, got {bogus}"
        );
    }

    /// Wave 4: `ExtAgentRegistry::insert` records `notify`, and
    /// `find_by_location` resolves a `(session_uuid, local_id)` pair back to the
    /// exact `(ext-facing id, notify)` it was registered with.
    #[test]
    fn insert_records_notify_and_find_by_location_resolves_pair() {
        let mut registry = ExtAgentRegistry::default();
        let sess_uuid = "session-x".to_string();

        let id_no_notify = registry.insert(sess_uuid.clone(), 10, false);
        let id_notify = registry.insert(sess_uuid.clone(), 11, true);

        let (found_id, found_notify) = registry
            .find_by_location(&sess_uuid, 10)
            .expect("location resolves");
        assert_eq!(found_id, id_no_notify);
        assert!(!found_notify, "notify: false must round-trip");

        let (found_id2, found_notify2) = registry
            .find_by_location(&sess_uuid, 11)
            .expect("location resolves");
        assert_eq!(found_id2, id_notify);
        assert!(found_notify2, "notify: true must round-trip");

        // A location never registered resolves to nothing.
        assert!(registry.find_by_location(&sess_uuid, 999).is_none());
        assert!(registry.find_by_location("other-session", 10).is_none());
    }

    /// Wave 4: `find_by_location` is scoped by session uuid — a fixture with TWO
    /// sessions sharing the SAME local sub-agent id (mirroring
    /// `cross_session_isolation_resolves_by_spawn_registry_not_foreground`'s
    /// setup) resolves each `(session_uuid, local_id)` pair to its OWN entry,
    /// never the other session's.
    #[test]
    fn find_by_location_scoped_by_session_not_just_local_id() {
        let mut registry = ExtAgentRegistry::default();
        let session_a = "session-a".to_string();
        let session_b = "session-b".to_string();

        // Same local id (5) registered against two DIFFERENT sessions.
        let ext_id_a = registry.insert(session_a.clone(), 5, true);
        let ext_id_b = registry.insert(session_b.clone(), 5, false);
        assert_ne!(ext_id_a, ext_id_b, "distinct ext-facing ids even for the same local id");

        let (found_a, notify_a) = registry
            .find_by_location(&session_a, 5)
            .expect("session A location resolves");
        assert_eq!(found_a, ext_id_a);
        assert!(notify_a);

        let (found_b, notify_b) = registry
            .find_by_location(&session_b, 5)
            .expect("session B location resolves");
        assert_eq!(found_b, ext_id_b);
        assert!(!notify_b);
    }

    /// W6 chat.prompt: BUFFERS into the active session's `pending_ext_prompts`
    /// (never injects here). Blank rejected; consecutive-dup returns the unchanged
    /// length without growth; cap 5 (the 6th distinct errors); >16KB rejected; each
    /// buffered entry carries the CALLER's ext id.
    #[test]
    fn chat_prompt_buffers_with_cap_dedupe_and_size_limit() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;
        let prompt = |state: &mut AppState, text: &str| {
            call_broker(
                state,
                rt.handle(),
                &client,
                "test.ext",
                &[Grant::ChatPrompt],
                "chat.prompt",
                json!({ "text": text }),
            )
        };

        // Blank text → rejected, nothing buffered.
        let blank = prompt(&mut state, "   ");
        assert!(blank.get("error").is_some(), "blank text must be rejected, got {blank}");
        assert!(state.rest.sessions[0].pending_ext_prompts.is_empty());

        // First accepted → queued: 1. Consecutive dup → still 1, NO growth.
        assert_eq!(prompt(&mut state, "one"), json!({ "queued": 1 }));
        assert_eq!(prompt(&mut state, "one"), json!({ "queued": 1 }));
        assert_eq!(state.rest.sessions[0].pending_ext_prompts.len(), 1);

        // Fill to the cap of 5 with distinct texts.
        assert_eq!(prompt(&mut state, "two"), json!({ "queued": 2 }));
        assert_eq!(prompt(&mut state, "three"), json!({ "queued": 3 }));
        assert_eq!(prompt(&mut state, "four"), json!({ "queued": 4 }));
        assert_eq!(prompt(&mut state, "five"), json!({ "queued": 5 }));

        // 6th distinct → cap error; buffer stays at 5.
        let full = prompt(&mut state, "six");
        assert!(
            full.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("queue full")),
            "the 6th distinct prompt must hit the cap, got {full}"
        );
        assert_eq!(state.rest.sessions[0].pending_ext_prompts.len(), 5);
        assert!(
            state.rest.sessions[0].pending_ext_prompts.iter().all(|(id, _)| id == "test.ext"),
            "every buffered entry must carry the caller's ext id"
        );

        // >16KB → rejected (independent of the cap), on a fresh session.
        let mut fresh = fixture_state();
        let big = "x".repeat(16_385);
        let toobig = call_broker(
            &mut fresh,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::ChatPrompt],
            "chat.prompt",
            json!({ "text": big }),
        );
        assert!(
            toobig.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("16KB")),
            "a >16KB prompt must be rejected, got {toobig}"
        );
        assert!(fresh.rest.sessions[0].pending_ext_prompts.is_empty());
    }

    /// Cost-DoS guard (review finding): `chat.prompt` refuses to buffer once the
    /// session's `ext_injected_turns` counter is AT the budget, but still accepts
    /// one below it. Mirrors `EXT_TURN_BUDGET` (10).
    #[test]
    fn chat_prompt_respects_turn_budget() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let client: Option<Arc<OpenRouterClient>> = None;

        // At budget (10) → refused, nothing buffered.
        let mut at_budget = fixture_state();
        at_budget.rest.sessions[0].ext_injected_turns = EXT_TURN_BUDGET;
        let out = call_broker(
            &mut at_budget,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::ChatPrompt],
            "chat.prompt",
            json!({ "text": "please respond" }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("turn budget exhausted")),
            "at budget must be refused, got {out}"
        );
        assert!(
            at_budget.rest.sessions[0].pending_ext_prompts.is_empty(),
            "a budget-refused prompt must NOT be buffered"
        );

        // One below budget (9) → accepted normally.
        let mut below_budget = fixture_state();
        below_budget.rest.sessions[0].ext_injected_turns = EXT_TURN_BUDGET - 1;
        let out = call_broker(
            &mut below_budget,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::ChatPrompt],
            "chat.prompt",
            json!({ "text": "please respond" }),
        );
        assert_eq!(out, json!({ "queued": 1 }), "below budget must be accepted, got {out}");
        assert_eq!(below_budget.rest.sessions[0].pending_ext_prompts.len(), 1);
    }

    /// W6 context.set / context.clear: keyed by the CALLER's ext id, 8KB
    /// boundary-exact (8192 ok, 8193 err), blank text clears, two ext ids fully
    /// isolated, and clear is idempotent.
    #[test]
    fn context_set_clear_isolation_and_size_boundary() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;
        let set = |state: &mut AppState, ext: &str, text: String| {
            call_broker(
                state,
                rt.handle(),
                &client,
                ext,
                &[Grant::ContextPublish],
                "context.set",
                json!({ "text": text }),
            )
        };

        // 8192 bytes EXACTLY → OK (boundary).
        assert_eq!(set(&mut state, "a.ext", "x".repeat(8192)), json!({ "ok": true }));
        assert_eq!(state.rest.ext_context.get("a.ext").map(String::len), Some(8192));

        // 8193 bytes → rejected; the prior blob is UNCHANGED.
        let toobig = set(&mut state, "a.ext", "y".repeat(8193));
        assert!(
            toobig.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("8KB")),
            "8193 bytes must be rejected, got {toobig}"
        );
        assert_eq!(state.rest.ext_context.get("a.ext").map(String::len), Some(8192));

        // A DIFFERENT ext writes its OWN blob — a.ext's is untouched (isolation).
        assert_eq!(set(&mut state, "b.ext", "b-data".to_string()), json!({ "ok": true }));
        assert_eq!(state.rest.ext_context.get("a.ext").map(String::len), Some(8192));
        assert_eq!(state.rest.ext_context.get("b.ext").map(String::as_str), Some("b-data"));

        // Blank text CLEARS the caller's OWN entry only.
        assert_eq!(set(&mut state, "a.ext", "   ".to_string()), json!({ "ok": true }));
        assert!(state.rest.ext_context.get("a.ext").is_none());
        assert_eq!(state.rest.ext_context.get("b.ext").map(String::as_str), Some("b-data"));

        // context.clear removes the caller's entry, leaving others intact.
        let cleared = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "b.ext",
            &[Grant::ContextPublish],
            "context.clear",
            json!({}),
        );
        assert_eq!(cleared, json!({ "ok": true }));
        assert!(state.rest.ext_context.get("b.ext").is_none());

        // Clearing an already-absent entry is idempotent — still ok.
        let again = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "b.ext",
            &[Grant::ContextPublish],
            "context.clear",
            json!({}),
        );
        assert_eq!(again, json!({ "ok": true }));
    }

    /// W6 models.invoke SYNC validation (the network path is untested, matching the
    /// awareness stance): unknown role, empty prompt, >32KB, and no-client all reply
    /// INLINE before any task is spawned.
    #[test]
    fn models_invoke_sync_validation_errors() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;
        let invoke = |state: &mut AppState, params: Value| {
            call_broker(
                state,
                rt.handle(),
                &client,
                "test.ext",
                &[Grant::ModelsInvoke],
                "models.invoke",
                params,
            )
        };

        // Unknown role → error.
        let bad_role = invoke(&mut state, json!({ "role": "wizard", "prompt": "hi" }));
        assert!(
            bad_role.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("unknown role")),
            "an unknown role must error, got {bad_role}"
        );

        // Empty prompt → error.
        let empty = invoke(&mut state, json!({ "prompt": "   " }));
        assert!(empty.get("error").is_some(), "an empty prompt must error, got {empty}");

        // >32KB prompt → error.
        let big = invoke(&mut state, json!({ "prompt": "x".repeat(32_769) }));
        assert!(big.get("error").is_some(), "a >32KB prompt must error, got {big}");

        // Valid role + prompt but NO client (the fixture has none) → "no llm client".
        // role=main resolves to koma-free (routable + usable), so validation reaches
        // the client check rather than short-circuiting on the route.
        let no_client = invoke(&mut state, json!({ "role": "main", "prompt": "hi" }));
        assert!(
            no_client.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("no llm client")),
            "a missing client must error, got {no_client}"
        );
    }

    // ─── W7 sessions.* ────────────────────────────────────────────────────────

    /// `merge_sessions` (the pure core of `sessions.list`) maps registry rows and
    /// live daemons into the reply array: a registered+live row reports the live
    /// daemon's `working`; a registered+dead row reports `live:false, working:false`;
    /// a live daemon with NO registry row is appended with a `null` name. Registry
    /// rows come first, live-but-unregistered after.
    #[test]
    fn merge_sessions_maps_registry_and_live() {
        let rows = vec![
            RegRow {
                uuid: "live-1".into(),
                pwd_hash: "h".into(),
                name: "Live One".into(),
                workdir: "/w/1".into(),
                updated_at: 2,
            },
            RegRow {
                uuid: "dead-1".into(),
                pwd_hash: "h".into(),
                name: "Dead One".into(),
                workdir: "/w/dead".into(),
                updated_at: 1,
            },
        ];
        let live = vec![
            SessionStatus {
                session_id: "live-1".into(),
                name: "ignored".into(),
                pwd: "/w/1".into(),
                working: true,
            },
            SessionStatus {
                session_id: "ghost".into(),
                name: "Ghost".into(),
                pwd: "/w/ghost".into(),
                working: false,
            },
        ];
        let arr = merge_sessions(rows, live);
        let arr = arr.as_array().expect("sessions.list is an array");
        assert_eq!(arr.len(), 3);
        // Registry rows first, in registry order.
        assert_eq!(arr[0]["id"], json!("live-1"));
        assert_eq!(arr[0]["name"], json!("Live One"));
        assert_eq!(arr[0]["workdir"], json!("/w/1"));
        assert_eq!(arr[0]["live"], json!(true));
        assert_eq!(arr[0]["working"], json!(true), "live row reports the daemon's working flag");
        assert_eq!(arr[1]["id"], json!("dead-1"));
        assert_eq!(arr[1]["live"], json!(false));
        assert_eq!(arr[1]["working"], json!(false), "dead row is never working");
        // Live-but-unregistered appended with a null name.
        assert_eq!(arr[2]["id"], json!("ghost"));
        assert_eq!(arr[2]["name"], Value::Null, "unregistered live session has no name");
        assert_eq!(arr[2]["workdir"], json!("/w/ghost"));
        assert_eq!(arr[2]["live"], json!(true));
        assert_eq!(arr[2]["working"], json!(false));
    }

    /// `spawn_into_error_json` (the pure failure map for the cross-process
    /// `sessions.spawn_into` branch, which is otherwise hard to unit-test): a
    /// refused/absent socket ⇒ "session not live"; every other io kind (timeout /
    /// EOF / decode / broken-pipe / …) ⇒ "target daemon incompatible or unavailable".
    #[test]
    fn spawn_into_error_json_maps_io_kinds() {
        use std::io::ErrorKind;
        assert_eq!(
            spawn_into_error_json(ErrorKind::ConnectionRefused),
            json!({ "error": "session not live" })
        );
        assert_eq!(
            spawn_into_error_json(ErrorKind::NotFound),
            json!({ "error": "session not live" })
        );
        for k in [
            ErrorKind::TimedOut,
            ErrorKind::WouldBlock,
            ErrorKind::UnexpectedEof,
            ErrorKind::InvalidData,
            ErrorKind::BrokenPipe,
        ] {
            assert_eq!(
                spawn_into_error_json(k),
                json!({ "error": "target daemon incompatible or unavailable" }),
                "io kind {k:?} must map to unavailable"
            );
        }
    }

    /// `parse_create_workdir` (the sync validation core of `sessions.create`): a
    /// missing/blank workdir is `Ok(None)` (the daemon buckets under its own cwd); a
    /// relative or non-existent path is rejected; an absolute existing dir is
    /// `Ok(Some)`.
    #[test]
    fn parse_create_workdir_validation() {
        assert!(matches!(parse_create_workdir(&json!({})), Ok(None)), "missing workdir → None");
        assert!(
            matches!(parse_create_workdir(&json!({ "workdir": "   " })), Ok(None)),
            "blank workdir → None"
        );

        let rel = parse_create_workdir(&json!({ "workdir": "relative/dir" }));
        assert!(rel.is_err(), "a relative workdir must be rejected");
        assert!(rel
            .unwrap_err()
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("absolute")));

        let missing = parse_create_workdir(&json!({ "workdir": "/no/such/koma/test/dir/xyz" }));
        assert!(missing.is_err(), "an absolute non-existent path must be rejected");

        let dir = std::env::temp_dir();
        let ok = parse_create_workdir(&json!({ "workdir": dir.to_str().unwrap() }));
        assert!(matches!(ok, Ok(Some(_))), "an absolute existing dir must pass, got {ok:?}");
    }

    /// `sessions.switch` (fully sync): a LIVE local session uuid actually moves the
    /// daemon's foreground (via the shared `handle_live_switch` chokepoint) and
    /// replies `delivery: "local"`; a non-local uuid instead latches
    /// `ext_switch_pending` for the hub to signal the client and replies
    /// `delivery: "signaled"` without moving the local foreground.
    #[test]
    fn sessions_switch_local_moves_foreground_remote_signals() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state(); // session A at idx 0 (foreground)
        let client: Option<Arc<OpenRouterClient>> = None;
        let session_b = SessionRuntime::new();
        let b_uuid = session_b.id.clone();
        state.rest.sessions.push(session_b); // B at idx 1
        assert_eq!(state.rest.foreground, 0);

        // Local-live switch to B → foreground moves, delivery "local", no attach signal.
        let local = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.switch",
            json!({ "session": b_uuid }),
        );
        assert_eq!(local, json!({ "ok": true, "delivery": "local" }));
        assert_eq!(state.rest.foreground, 1, "a local switch moves the foreground");
        assert!(
            state.rest.ext_switch_pending.is_none(),
            "a local switch must NOT latch an attach signal"
        );

        // Non-local uuid → ext_switch_pending latched, delivery "signaled", foreground unmoved.
        let remote = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.switch",
            json!({ "session": "no-such-session" }),
        );
        assert_eq!(remote, json!({ "ok": true, "delivery": "signaled" }));
        assert_eq!(
            state.rest.ext_switch_pending.as_deref(),
            Some("no-such-session"),
            "a remote switch latches the attach signal"
        );
        assert_eq!(state.rest.foreground, 1, "a signaled switch must NOT move local foreground");
    }

    /// `sessions.spawn_into` LOCAL branch: a two-session fixture, spawning into the
    /// NON-foreground session B by uuid, routes through `broker_spawn` with B's index
    /// (not the foreground A's) — the queued sub-agent lands in B and the reply carries
    /// an ext-facing `agentId`. Forced onto the QUEUE path (session filled to the
    /// sub-agent cap) so no real model/task is needed; a client must exist for the
    /// queue. Also covers the up-front `session`/`task` validation guards.
    #[test]
    fn spawn_into_local_queues_into_named_session_with_agentid() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state(); // session A (foreground) at idx 0

        // Session B (non-foreground) at idx 1 with a minimal real `Session` (so the queue
        // path's `session.is_none()` guard passes), filled to the sub-agent cap so
        // `spawn_or_queue` takes the QUEUE branch (pure in-memory — no task/disk/network).
        let mut session_b = SessionRuntime::new();
        let b_uuid = session_b.id.clone();
        session_b.session = Some(crate::model::session::Session::new(
            b_uuid.clone(),
            std::path::PathBuf::from("/tmp/koma-spawn-into-test"),
            "hash".into(),
            crate::model::settings::Settings::default(),
            crate::model::conversation::Conversation::new(""),
        ));
        for i in 0..crate::app::subagent::MAX_SUBAGENTS {
            session_b
                .subagents
                .push(inert_subagent(rt.handle(), i, "general", SubAgentStatus::Running));
        }
        session_b.next_subagent_id = crate::app::subagent::MAX_SUBAGENTS;
        state.rest.sessions.push(session_b);

        // A client must exist for the queue path (it never runs a task here).
        let client: Option<Arc<OpenRouterClient>> = Some(crate::app::runtime::build_client());

        let out = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.spawn_into",
            json!({ "session": b_uuid, "task": "do it" }),
        );
        assert!(
            out.get("agentId").is_some(),
            "a local spawn_into must return an ext-facing agentId, got {out}"
        );
        assert_eq!(out["status"], json!("queued"));
        // The queued spawn landed in session B (idx 1), NOT the foreground A (idx 0).
        assert_eq!(
            state.rest.sessions[1].pending_subagents.len(),
            1,
            "the spawn queued into the named non-foreground session B"
        );
        assert!(
            state.rest.sessions[0].pending_subagents.is_empty(),
            "the foreground session A must be untouched"
        );

        // Validation guards: a missing 'session' and an empty 'task' both error inline.
        let no_session = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.spawn_into",
            json!({ "task": "x" }),
        );
        assert!(
            no_session.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("'session'")),
            "a missing session must error, got {no_session}"
        );
        let empty_task = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "test.ext",
            &[Grant::SessionsManage],
            "sessions.spawn_into",
            json!({ "session": b_uuid, "task": "   " }),
        );
        assert!(
            empty_task.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("non-empty 'task'")),
            "an empty task must error, got {empty_task}"
        );
    }
}
