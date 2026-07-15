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

use std::collections::{HashMap, HashSet};
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
use crate::model::app_config::{new_uuid, ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn};
use crate::model::settings::Settings;
use crate::model::session_registry::RegRow;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

/// The agent an extension `agents.spawn` runs when it omits `agent` — koma's
/// built-in general-purpose agent.
const DEFAULT_AGENT: &str = "general";

/// W12 `models.register` batch cap: at most this many models per call (a DoS guard on the
/// global catalogue an extension can grow).
const MAX_REGISTER_MODELS: usize = 100;

/// W12 `models.register` per-field length cap: each model's `id` / `name` must be non-empty
/// and no longer than this.
const MAX_MODEL_FIELD_LEN: usize = 200;

/// W12b `providers.register` name cap: a key-backed provider's `name` must be non-empty and no
/// longer than this (shares the field-length ceiling with model ids/names).
const MAX_PROVIDER_NAME_LEN: usize = 200;

/// W12b `providers.register` key cap: the injected `api_key` must be non-empty and no longer
/// than this (a static bearer token / API key — a generous ceiling that still bounds abuse).
const MAX_PROVIDER_KEY_LEN: usize = 4096;

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
/// `agents.spawn` / `agents.kill` / `agents.send` MUTATE or STEER the fleet →
/// require [`Grant::AgentsOrchestrate`]. `agents.list` / `agents.status` /
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
        "agents.spawn" | "agents.kill" | "agents.send" => Some(Grant::AgentsOrchestrate),
        "agents.list" | "agents.status" | "agents.result" => Some(Grant::AgentsRead),
        "sessions.list" | "sessions.create" | "sessions.switch" | "sessions.spawn_into" => {
            Some(Grant::SessionsManage)
        }
        "chat.prompt" => Some(Grant::ChatPrompt),
        "models.invoke" => Some(Grant::ModelsInvoke),
        // W12: registering/unregistering the extension's OWN models needs `models:contribute`
        // (EXACT-MATCH, like every family below — `models:invoke` never confers it and vice
        // versa; they gate different verbs).
        "models.register" | "models.unregister" => Some(Grant::ModelsContribute),
        // W12b: registering/unregistering the extension's OWN key-backed providers reuses the
        // SAME `models:contribute` grant — an extension that may contribute models may also
        // contribute the gateways that serve them (both grow the extension's OWN catalogue;
        // there is no separate `providers:contribute` scope).
        "providers.register" | "providers.unregister" => Some(Grant::ModelsContribute),
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
        // W12: exact-match — gates `models.register`/`models.unregister` only.
        Grant::ModelsContribute => granted.contains(&Grant::ModelsContribute),
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
    const PREFIXES: [&str; 6] =
        ["agents.", "sessions.", "chat.", "models.", "providers.", "context."];
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
        Grant::ModelsContribute => "models:contribute",
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
            "models:contribute" => Some(Grant::ModelsContribute),
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
    // — each inner-bounded well under the reader's verb-scoped cap (360s for `models.invoke`,
    // 120s `EXT_CALL_TIMEOUT` default for every other verb — see `wire.rs`).
    //
    // `agents.spawn` ALONE resolves the ACTIVE (foreground) session — spawning into
    // "whatever chat session is in front of the user right now" is the intended
    // behavior. Every other `agents.*` verb resolves the sub-agent through THIS
    // extension's own [`ExtAgentRegistry`] instead (never the foreground), so a
    // foreground switch between a spawn and a later poll can never redirect that poll
    // at a different session's sub-agent.
    //
    // KNOWN INLINE-BLOCKING WINDOW: unlike `models.invoke`/`sessions.*`, `agents.spawn`
    // dispatches to `broker_spawn` SYNCHRONOUSLY on the event loop rather than moving
    // the reply into a spawned task. This is deliberate, not an oversight — `broker_spawn`
    // (via `spawn_or_queue`/`spawn_task`/`spawn_subagent`) mutates `AppState` directly
    // (the session's `subagents`/`pending_subagents`/`next_subagent_id`, this extension's
    // `ExtAgentRegistry`), and `AppState` is a unique `&mut` borrowed fresh each tick —
    // there is no `Arc<Mutex<AppState>>` (or equivalent) for a detached task to move that
    // work onto, so hoisting this off the loop would need a real architecture change
    // (e.g. a state-mutation channel back to the loop), not just a `tokio::spawn`.
    // The one blocking call on this path is `McpManager::advertise_cached` (via
    // `spawn_subagent`'s MCP-tool inherit step), which on the `Proxy` backend can run an
    // inline `McpRequest::List` bounded by `PROXY_IO_TIMEOUT` (65s) — but ONLY on a
    // genuine cold start (empty cache, never yet confirmed empty). Once confirmed empty
    // (see `McpManager::advertise_confirmed_empty_at`), that window is skipped for
    // `STATUS_CACHE_TTL` and refreshed in the background instead, so in steady state this
    // dispatch is a fast in-memory cache read. A worst-case 65s stall on a rare cold-start
    // spawn was judged acceptable rather than reworking spawn's state-mutation semantics
    // for a theoretical/rare stall.
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
        "agents.send" => {
            let _ = reply.send(broker_send(state, &ext_id, &params));
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
            // inline OR moves `reply` into a spawned one-shot task (330s < the
            // reader's 360s `EXT_MODELS_CALL_TIMEOUT` verb cap for this method)
            // that answers on completion — so the model call never blocks the
            // event loop. OWNS the reply from here.
            broker_models_invoke(state, handle, client, &params, reply);
        }
        "models.register" => {
            // W12: register the extension's OWN models into the GLOBAL catalogue, served by
            // its connected OAuth account. The config mutation + save is cheap and MUST run on
            // the loop (where `state.rest.config` is live), so this replies INLINE.
            let _ = reply.send(broker_models_register(state, &ext_id, &params));
        }
        "models.unregister" => {
            let _ = reply.send(broker_models_unregister(state, &ext_id, &params));
        }
        "providers.register" => {
            // W12b: inject a key-backed provider (a first-party gateway the extension owns) into
            // the GLOBAL catalogue. Cheap config mutation + save, MUST run on the loop.
            let _ = reply.send(broker_providers_register(state, &ext_id, &params));
        }
        "providers.unregister" => {
            let _ = reply.send(broker_providers_unregister(state, &ext_id, &params));
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
            // The reader task caps this Call at the 120s `EXT_CALL_TIMEOUT` default (this is
            // not `models.invoke`); the probe sweep is inner-bounded far below that.
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
/// tool-call id (the `/task`-command shape) but marked `ext_owned`, so its
/// completion is COMPLETELY SILENT in the human chat (no fold note, no nudge) —
/// the spawner instead receives the result via the owned `agents.done` event.
/// Usage + the persisted sub-agent record are still recorded. The returned `agentId` is
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

    // `ext_owned = true`: this is an EXTENSION-INTERNAL agent. On terminal it stays
    // COMPLETELY SILENT in the human chat (no fold note, no nudge) — the spawner
    // receives the result via the owned `agents.done` event instead (see
    // `emit_subagent_terminal`). Usage + the sub-agent record are still recorded.
    match spawn_or_queue(state, sess_idx, client, handle, agent, task, None, false, true, overrides) {
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
            super::events::emit_subagent_terminal(
                state,
                &session_uuid,
                local_id,
                &agent,
                "killed",
                None,
            );
        }
        json!({ "killed": true })
    } else {
        json!({ "error": format!("unknown agentId: {ext_agent_id}") })
    }
}

/// `agents.send { agentId, message }` → inject `message` as a follow-up USER turn
/// into the sub-agent, delivered at its next TURN BOUNDARY (never mid-stream). The
/// running sub-agent's loop folds it into its isolated history + viewer transcript
/// on its next iteration. Resolved ONLY through THIS extension's own
/// [`ExtAgentRegistry`] (same containment as `status`/`kill`), so an extension can
/// never steer a sub-agent it didn't spawn.
///
/// Running → `{ "sent": true }`; QUEUED (over the cap, not yet started) → the
/// message is stashed on the pending record and delivered at promotion, replying
/// `{ "sent": true, "status": "queued" }`; TERMINAL (done/killed/error) →
/// `{ "error": "agent is terminal" }`. Missing/empty `message` → its own error;
/// missing `agentId` / unknown id / closed session mirror the `agents.status`
/// error shapes exactly.
fn broker_send(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let Some(ext_agent_id) = parse_ext_agent_id(params) else {
        return json!({ "error": "agents.send requires an 'agentId'" });
    };
    let message = params
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if message.is_empty() {
        return json!({ "error": "agents.send requires a non-empty 'message'" });
    }
    let message = message.to_string();
    // Resolve the ext-facing id through THIS extension's registry (clone the ref so
    // the mutable session borrow below is unencumbered — mirrors `broker_kill`).
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
    // Funnel through the SAME core the `task_send` tool uses, so both surfaces
    // steer identically.
    use crate::app::subagent::InjectOutcome;
    match state.rest.sessions[sess_idx].inject_into_subagent(r.local_subagent_id, message) {
        InjectOutcome::Sent => json!({ "sent": true }),
        InjectOutcome::Queued => json!({ "sent": true, "status": "queued" }),
        InjectOutcome::Terminal => json!({ "error": "agent is terminal" }),
        // The registry entry resolved but the sub-agent is neither in `subagents`
        // nor `pending_subagents` (its session was cleared) — same shape as an
        // unknown id, which is what it now effectively is.
        InjectOutcome::Unknown => json!({ "error": format!("unknown agentId: {ext_agent_id}") }),
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

/// `models.invoke { role?, system?, prompt, format? }` → a ONE-SHOT completion
/// against the resolved model for `role` (default `"main"`), run OFF the event
/// loop.
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
/// `format`: when the string `"json"`, the request pins OpenAI-dialect strict
/// `response_format: {"type":"json_object"}` (threaded through
/// [`OpenRouterClient::complete_with`]'s `json_mode` flag); any other value, or
/// the field absent, is today's free-form-text behavior. The flag is DIALECT-
/// GATED: `complete_with` only honours it on the chat-completions branch
/// (`OpenAiCompatible`/`KomaFree`) — the Codex and Anthropic-compatible dialects
/// have no `json_object` wire equivalent, so `format:"json"` is silently ignored
/// (never an error) when the resolved route speaks either of those.
///
/// Once validated, an owned `Resolved` + an `Arc` clone of the client + the reply
/// oneshot MOVE into a spawned task (the `spawn_awareness_recompute` pattern) that
/// runs `complete_with` under a 330s `tokio::time::timeout` — 330s deliberately
/// UNDERCUTS the reader task's 360s `EXT_MODELS_CALL_TIMEOUT` verb cap (see
/// `wire.rs`) so the extension always receives a value rather than a transport
/// timeout. Reply `{ "output": <text>, "model": <id> }` on success,
/// `{ "error": "model call failed: <e>" }` on a call error, or
/// `{ "error": "model call timed out" }`. The event loop never blocks.
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
    // Optional structured-output request: only the literal "json" opts in
    // (threaded to `complete_with`'s `json_mode`, dialect-gated there — see the
    // doc comment above). Any other value, or the field absent, is today's
    // free-form behavior.
    let json_mode = params.get("format").and_then(|v| v.as_str()) == Some("json");
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
        // 330s < the reader's 360s `EXT_MODELS_CALL_TIMEOUT` verb cap: the
        // extension always gets a value back.
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(330),
            client_task.complete_with(
                route.conn(),
                &route.model_id,
                route.provider(),
                messages,
                json_mode,
            ),
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

/// W12/W12b: `models.register { models: [{ id, name, default? }], provider? }` → register the
/// extension's OWN models into the GLOBAL catalogue (`config.models`), each served by a
/// caller-owned ANCHOR. SYNC on the loop (cheap config mutation + save).
///
/// The anchor (W12b generalization — see [`pick_ext_anchor`]) is a caller-owned KEY-BACKED
/// [`ProviderConn`] (from `providers.register`) OR a caller-owned OAuth conn that is a usable
/// model provider. An explicit `{ "provider": "<uuid>" }` selects it (must be caller-owned,
/// else `{"error":"provider not owned by this extension"}`); absent, exactly one eligible
/// anchor is used, multiple → `{"error":"multiple providers; specify provider uuid"}`, zero →
/// the W12 `no-conn` / `account-login-only` errors.
///
/// Dedupe is by `(provider_uuid, model_id)`: a re-register of the same model UPDATES its
/// display `name` IN PLACE while KEEPING its uuid (the stability contract — an ext sub-agent
/// bound to that uuid keeps resolving). A new pair mints a fresh ROLE-LESS [`ModelEntry`]
/// (`provider_uuid` = the anchor uuid, so resolution binds it straight to that anchor). Caps:
/// at most [`MAX_REGISTER_MODELS`] per call, each `id`/`name` non-empty and ≤
/// [`MAX_MODEL_FIELD_LEN`]; an invalid batch is rejected ATOMICALLY (nothing registered).
///
/// W12b: at most ONE model per call may carry `"default": true` (else
/// `{"error":"multiple defaults in one call"}`); it is recorded as the extension's preferred
/// model and, when Main is currently unset / only the koma-free placeholder, VACUUM-FILLS the
/// Main role (see [`try_vacuum_fill_main`]). Reply `{ "registered": n, "uuids": [...] }` (plus
/// `"defaultUuid"` when a default was flagged), then persist `config.json`.
///
/// Thin wrapper over the PURE [`apply_models_register`] (which owns the validation + catalogue
/// mutation, unit-tested without touching `~/.koma/config.json`): persists only on a
/// successful registration (the reply carries `"registered"`; an error reply carries none, so
/// a rejected batch never re-writes the config).
fn broker_models_register(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let reply = apply_models_register(&mut state.rest.config, ext_id, params);
    if reply.get("registered").is_some() {
        // W12b: when this call marked a preferred (`default: true`) model, VACUUM-FILL the
        // Main role with it iff Main is currently unset OR only the keyless koma-free
        // placeholder — NEVER overriding a real user choice (first vacuum-fill wins; a later
        // extension only hints via the additive `recommendedBy` wire flag). The foreground
        // session's settings supply the session-override half of the "is Main set?" check.
        if let Some(default_uuid) = reply
            .get("defaultUuid")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            let settings = state
                .rest
                .fg()
                .session
                .as_ref()
                .map(|s| s.settings.clone())
                .unwrap_or_default();
            if let Some(name) = try_vacuum_fill_main(&mut state.rest.config, &settings, &default_uuid)
            {
                // The new Main was assigned GLOBALLY. Clear any koma-free placeholder
                // session-local Main override on the foreground session so /free doesn't SHADOW
                // it (the toast would otherwise lie). Snapshot the koma-free provider uuids
                // before the mutable session borrow.
                let koma_free_uuids: HashSet<String> = state
                    .rest
                    .config
                    .providers
                    .iter()
                    .filter(|p| p.api_type == ApiType::KomaFree)
                    .map(|p| p.uuid.clone())
                    .collect();
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    let before = sess.settings.session_models.len();
                    sess.settings.session_models.retain(|e| {
                        !(e.effective_roles().contains(&ModelRole::Main)
                            && koma_free_uuids.contains(&e.provider_uuid))
                    });
                    if sess.settings.session_models.len() != before {
                        let _ = sess.save();
                    }
                }
                state
                    .rest
                    .fg_mut()
                    .set_toast_info(format!("model {name} set by extension {ext_id}"));
            }
        }
        if let Err(e) = state.rest.config.save() {
            store::append_global_error_log(
                "ext models",
                &format!("[{ext_id}] models.register save failed: {e:#}"),
            );
        }
    }
    reply
}

/// PURE core of [`broker_models_register`]: validate `params` and apply the registration to
/// `config.models`, returning the reply JSON. Does NOT persist (the wrapper saves). See
/// [`broker_models_register`] for the full contract.
fn apply_models_register(config: &mut AppConfig, ext_id: &str, params: &Value) -> Value {
    let Some(models) = params.get("models").and_then(|v| v.as_array()) else {
        return json!({ "error": "models.register requires a 'models' array" });
    };
    if models.is_empty() {
        return json!({ "error": "models.register requires at least one model" });
    }
    if models.len() > MAX_REGISTER_MODELS {
        return json!({ "error": format!("too many models (max {MAX_REGISTER_MODELS})") });
    }
    // Validate + collect (id, name, is_default) up front — a bad entry rejects the WHOLE batch
    // (atomic: nothing is registered unless every entry is valid).
    let mut parsed: Vec<(String, String, bool)> = Vec::with_capacity(models.len());
    let mut default_count = 0usize;
    for m in models {
        let id = m.get("id").and_then(Value::as_str).unwrap_or("").trim();
        let name = m.get("name").and_then(Value::as_str).unwrap_or("").trim();
        if id.is_empty() || name.is_empty() {
            return json!({ "error": "each model requires a non-empty 'id' and 'name'" });
        }
        if id.len() > MAX_MODEL_FIELD_LEN || name.len() > MAX_MODEL_FIELD_LEN {
            return json!({ "error": format!("model id/name too long (max {MAX_MODEL_FIELD_LEN})") });
        }
        // W12b: at most ONE entry per call may flag itself the extension's preferred default.
        let is_default = m.get("default").and_then(Value::as_bool).unwrap_or(false);
        if is_default {
            default_count += 1;
        }
        parsed.push((id.to_string(), name.to_string(), is_default));
    }
    if default_count > 1 {
        return json!({ "error": "multiple defaults in one call" });
    }

    // Pick the ANCHOR the models are served by — a caller-owned key-backed provider OR oauth
    // conn (W12b generalization). Owned uuid, so the immutable borrow of `config` ends before
    // the mutation below.
    let anchor_uuid = match pick_ext_anchor(config, ext_id, params) {
        Ok(u) => u,
        Err(e) => return e,
    };

    // Register each model: dedupe by (provider_uuid, model_id) — update the name in place
    // (KEEP uuid, the stability contract), else mint a fresh role-less entry. Track the uuid
    // of the entry flagged `default: true` (if any) so the caller can vacuum-fill Main.
    let mut uuids: Vec<String> = Vec::with_capacity(parsed.len());
    let mut default_uuid: Option<String> = None;
    for (id, name, is_default) in parsed {
        let uuid = if let Some(existing) = config
            .models
            .iter_mut()
            .find(|e| e.provider_uuid == anchor_uuid && e.model_id == id)
        {
            existing.name = name;
            existing.uuid.clone()
        } else {
            let entry = ModelEntry {
                uuid: new_uuid(),
                name,
                model_id: id,
                provider_uuid: anchor_uuid.clone(),
                route: None,
                roles: Vec::new(),
                role: None,
                source_uuid: None,
            };
            let u = entry.uuid.clone();
            config.models.push(entry);
            u
        };
        if is_default {
            default_uuid = Some(uuid.clone());
        }
        uuids.push(uuid);
    }

    // W12b: record the extension's PREFERRED model (persisted — drives vacuum-fill Main + the
    // `recommendedBy` picker hint) and echo its uuid so the persisting wrapper can vacuum-fill.
    // Only when THIS call explicitly flagged one.
    let mut reply = json!({ "registered": uuids.len(), "uuids": uuids });
    if let Some(du) = default_uuid {
        config.ext_preferred_models.insert(ext_id.to_string(), du.clone());
        reply["defaultUuid"] = json!(du);
    }
    reply
}

/// W12b: pick the ANCHOR uuid a `models.register` call serves its models from — generalizing
/// W12's oauth-only [`pick_ext_provider_conn`] to ALSO consider the extension's key-backed
/// [`ProviderConn`]s (injected via `providers.register`). Returns the anchor uuid (used as the
/// registered models' `provider_uuid`), or an error [`Value`] the caller replies verbatim.
///
/// An explicit `{ "provider": "<uuid>" }` param must be CALLER-OWNED — a key-backed provider
/// with `ext_id == ext_id`, or an oauth conn with `ext_id == ext_id` that is a usable model
/// provider (has [`OAuthConn::ext_model_route`]). An owned-but-account-login-only conn →
/// `"provider is account-login only"`; anything else → `"provider not owned by this extension"`.
///
/// Absent, the eligible anchors are gathered (all key-backed ext providers ∪ all ext oauth
/// conns that are model providers): exactly one → use it; more than one →
/// `"multiple providers; specify provider uuid"`; zero → the SAME two W12 errors as before
/// (`"provider is account-login only"` when the ext has an account-login-only conn but no
/// usable anchor, else `"no connected oauth account for this extension"`).
fn pick_ext_anchor(config: &AppConfig, ext_id: &str, params: &Value) -> Result<String, Value> {
    // Explicit provider uuid: must be caller-owned AND a usable anchor.
    if let Some(req) = params
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // A key-backed provider owned by this extension is always usable.
        if config
            .providers
            .iter()
            .any(|p| p.uuid == req && p.ext_id.as_deref() == Some(ext_id))
        {
            return Ok(req.to_string());
        }
        // An oauth conn owned by this extension is usable only when it carries model-provider
        // meta (endpoint + api_type); otherwise it is account-login-only.
        if let Some(conn) = config
            .oauth_conns
            .iter()
            .find(|c| c.uuid == req && c.ext_id.as_deref() == Some(ext_id))
        {
            return if conn.ext_model_route().is_some() {
                Ok(req.to_string())
            } else {
                Err(json!({ "error": "provider is account-login only" }))
            };
        }
        return Err(json!({ "error": "provider not owned by this extension" }));
    }

    // No explicit provider: gather every eligible anchor this extension owns.
    let mut anchors: Vec<String> = config
        .providers
        .iter()
        .filter(|p| p.ext_id.as_deref() == Some(ext_id))
        .map(|p| p.uuid.clone())
        .collect();
    anchors.extend(
        config
            .oauth_conns
            .iter()
            .filter(|c| c.ext_id.as_deref() == Some(ext_id) && c.ext_model_route().is_some())
            .map(|c| c.uuid.clone()),
    );
    match anchors.len() {
        1 => Ok(anchors.remove(0)),
        0 => {
            // Distinguish "connected but account-login-only" from "nothing connected at all".
            let has_conn = config
                .oauth_conns
                .iter()
                .any(|c| c.ext_id.as_deref() == Some(ext_id));
            if has_conn {
                Err(json!({ "error": "provider is account-login only" }))
            } else {
                Err(json!({ "error": "no connected oauth account for this extension" }))
            }
        }
        _ => Err(json!({ "error": "multiple providers; specify provider uuid" })),
    }
}

/// W12: `models.unregister { ids?: [String] }` → remove entries from `config.models` this
/// extension OWNS (served by one of ITS OWN OAuth conns — the ownership wall: an extension
/// can never unregister another extension's or the user's own models).
///
/// `ids` absent → remove ALL of the caller's entries. `ids` present → remove only the
/// caller-owned entries whose `model_id` OR `uuid` matches one of `ids` (case-insensitively,
/// mirroring the slug-match convention). Reply `{ "removed": n }`, persisting `config.json`
/// only when something actually changed.
///
/// Thin wrapper over the PURE [`apply_models_unregister`] (unit-tested without disk):
/// persists only when at least one entry was removed.
fn broker_models_unregister(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let reply = apply_models_unregister(&mut state.rest.config, ext_id, params);
    if reply.get("removed").and_then(Value::as_u64).is_some_and(|n| n > 0) {
        if let Err(e) = state.rest.config.save() {
            store::append_global_error_log(
                "ext models",
                &format!("[{ext_id}] models.unregister save failed: {e:#}"),
            );
        }
    }
    reply
}

/// PURE core of [`broker_models_unregister`]: apply the removal to `config.models` and return
/// the reply JSON. Does NOT persist (the wrapper saves). See [`broker_models_unregister`].
fn apply_models_unregister(config: &mut AppConfig, ext_id: &str, params: &Value) -> Value {
    // The provider_uuids owned by THIS extension — its oauth conns (W11/W12) AND its key-backed
    // providers (W12b) — the ownership wall. A model served by either kind of ext-owned anchor
    // is the extension's own; a model on any other provider is never touched.
    let owned: HashSet<String> = config
        .oauth_conns
        .iter()
        .filter(|c| c.ext_id.as_deref() == Some(ext_id))
        .map(|c| c.uuid.clone())
        .chain(
            config
                .providers
                .iter()
                .filter(|p| p.ext_id.as_deref() == Some(ext_id))
                .map(|p| p.uuid.clone()),
        )
        .collect();
    if owned.is_empty() {
        return json!({ "removed": 0 });
    }

    // Optional id filter (model_id OR uuid, case-insensitive). Absent → remove all owned.
    let id_filter: Option<Vec<String>> = params.get("ids").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    });

    let before = config.models.len();
    config.models.retain(|e| {
        // Not owned by the caller → always kept (the ownership wall).
        if !owned.contains(&e.provider_uuid) {
            return true;
        }
        match &id_filter {
            // No filter → remove every owned entry.
            None => false,
            // Filter → keep unless this owned entry matches by model_id or uuid.
            Some(ids) => !ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(&e.model_id) || id.eq_ignore_ascii_case(&e.uuid)),
        }
    });
    json!({ "removed": before - config.models.len() })
}

// ─── W12b providers.register / providers.unregister + vacuum-fill ───────────────

/// W12b: `providers.register { name, endpoint, api_type, key }` → inject a KEY-BACKED provider
/// (a first-party gateway the extension owns) into the GLOBAL catalogue (`config.providers`),
/// stamped with the caller's `ext_id`. SYNC on the loop (cheap config mutation + save).
///
/// Validation: `name` non-empty ≤ [`MAX_PROVIDER_NAME_LEN`]; `endpoint` parses as an http(s)
/// URL (via the `url` crate); `api_type` ∈ {`"openai"`, `"anthropic"`} (the same normalize
/// semantics as W12's [`OAuthConn::ext_model_route`] — `"openai"` → [`ApiType::OpenAiCompatible`],
/// `"anthropic"` → [`ApiType::AnthropicCompatible`]); `key` non-empty ≤ [`MAX_PROVIDER_KEY_LEN`].
///
/// KEY-ROTATION CONTRACT: dedupe is per (caller `ext_id`, `name`) — a re-register of the same
/// name UPDATES the existing provider's `endpoint` / `api_key` / `api_type` IN PLACE while
/// KEEPING its uuid (so a registered model bound to that provider keeps resolving, and a caller
/// can rotate a leaked key without re-registering its models). Else a fresh [`ProviderConn`]
/// with a v4 uuid is minted. Reply `{ "uuid": <stable uuid> }`.
///
/// Thin wrapper over the PURE [`apply_providers_register`]: persists only on success.
fn broker_providers_register(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let reply = apply_providers_register(&mut state.rest.config, ext_id, params);
    if reply.get("uuid").is_some() {
        if let Err(e) = state.rest.config.save() {
            store::append_global_error_log(
                "ext providers",
                &format!("[{ext_id}] providers.register save failed: {e:#}"),
            );
        }
    }
    reply
}

/// PURE core of [`broker_providers_register`]: validate + apply. Does NOT persist. See that
/// function for the full contract.
fn apply_providers_register(config: &mut AppConfig, ext_id: &str, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() {
        return json!({ "error": "providers.register requires a non-empty 'name'" });
    }
    if name.len() > MAX_PROVIDER_NAME_LEN {
        return json!({ "error": format!("provider name too long (max {MAX_PROVIDER_NAME_LEN})") });
    }
    let endpoint = params.get("endpoint").and_then(Value::as_str).unwrap_or("").trim();
    if !is_http_url(endpoint) {
        return json!({ "error": "endpoint must be a valid http(s) URL" });
    }
    let api_type = match normalize_provider_api_type(params.get("api_type").and_then(Value::as_str)) {
        Some(t) => t,
        None => return json!({ "error": "api_type must be 'openai' or 'anthropic'" }),
    };
    let key = params.get("key").and_then(Value::as_str).unwrap_or("").trim();
    if key.is_empty() {
        return json!({ "error": "providers.register requires a non-empty 'key'" });
    }
    if key.len() > MAX_PROVIDER_KEY_LEN {
        return json!({ "error": format!("key too long (max {MAX_PROVIDER_KEY_LEN})") });
    }

    // Dedupe per (caller ext_id, name): update in place keeping the uuid (key-rotation), else mint.
    if let Some(existing) = config
        .providers
        .iter_mut()
        .find(|p| p.ext_id.as_deref() == Some(ext_id) && p.name == name)
    {
        existing.endpoint = endpoint.to_string();
        existing.api_key = key.to_string();
        existing.api_type = api_type;
        return json!({ "uuid": existing.uuid.clone() });
    }
    let uuid = new_uuid();
    config.providers.push(ProviderConn {
        uuid: uuid.clone(),
        name: name.to_string(),
        api_type,
        endpoint: endpoint.to_string(),
        api_key: key.to_string(),
        ext_id: Some(ext_id.to_string()),
    });
    json!({ "uuid": uuid })
}

/// W12b: `providers.unregister { ids?: [String] }` → remove KEY-BACKED providers this extension
/// OWNS (the ownership wall — an extension can never unregister another extension's or the
/// user's own providers). `ids` absent → remove ALL of the caller's; present → remove only the
/// caller-owned providers whose `uuid` OR `name` matches one of `ids` (case-insensitively).
///
/// Removing a provider ALSO removes every model whose `provider_uuid` pointed at it (orphan
/// prevention — the SAME [`AppConfig::remove_models_by_providers`] sweep the uninstall purge
/// uses). Reply `{ "removed": n }` (providers removed), persisting only when something changed.
fn broker_providers_unregister(state: &mut AppState, ext_id: &str, params: &Value) -> Value {
    let reply = apply_providers_unregister(&mut state.rest.config, ext_id, params);
    if reply.get("removed").and_then(Value::as_u64).is_some_and(|n| n > 0) {
        if let Err(e) = state.rest.config.save() {
            store::append_global_error_log(
                "ext providers",
                &format!("[{ext_id}] providers.unregister save failed: {e:#}"),
            );
        }
    }
    reply
}

/// PURE core of [`broker_providers_unregister`]: apply the removal + orphan-model sweep. Does
/// NOT persist. See that function for the full contract.
fn apply_providers_unregister(config: &mut AppConfig, ext_id: &str, params: &Value) -> Value {
    // Optional id filter (uuid OR name, case-insensitive). Absent → remove all owned.
    let id_filter: Option<Vec<String>> = params.get("ids").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    });

    // The caller-owned providers to remove (uuids), honouring the id filter.
    let dead: HashSet<String> = config
        .providers
        .iter()
        .filter(|p| p.ext_id.as_deref() == Some(ext_id))
        .filter(|p| match &id_filter {
            None => true,
            Some(ids) => ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(&p.uuid) || id.eq_ignore_ascii_case(&p.name)),
        })
        .map(|p| p.uuid.clone())
        .collect();
    if dead.is_empty() {
        return json!({ "removed": 0 });
    }
    // Orphan prevention: drop models served by a removed provider FIRST (shared sweep), then
    // the providers themselves.
    config.remove_models_by_providers(&dead);
    config.providers.retain(|p| !dead.contains(&p.uuid));
    json!({ "removed": dead.len() })
}

/// W12b: whether `endpoint` is a well-formed http(s) URL — the endpoint gate for
/// `providers.register`. Uses the `url` crate (already a dependency) and additionally requires
/// an http/https scheme (a `file:`/`data:`/etc. URL parses but is never a chat endpoint).
fn is_http_url(endpoint: &str) -> bool {
    match url::Url::parse(endpoint) {
        Ok(u) => matches!(u.scheme(), "http" | "https"),
        Err(_) => false,
    }
}

/// W12b: normalize a `providers.register` `api_type` wire string to a static-key [`ApiType`],
/// reusing W12's `"openai"` / `"anthropic"` vocabulary (see [`OAuthConn::ext_model_route`]).
/// `None` for anything unrecognised (or absent) so the caller rejects it — koma-free / Codex
/// wire types are never user-injectable through this verb.
fn normalize_provider_api_type(raw: Option<&str>) -> Option<ApiType> {
    match raw.map(str::trim) {
        Some("openai") => Some(ApiType::OpenAiCompatible),
        Some("anthropic") => Some(ApiType::AnthropicCompatible),
        _ => None,
    }
}

/// W12b: whether the Main role is currently UNSET for VACUUM-FILL purposes — NO REAL model
/// holds Main in EITHER scope (the global catalogue `config.models` OR the session override
/// layer `settings.session_models`). A "real" holder is one backed by a non-koma-free provider;
/// the keyless koma-free placeholder (`ApiType::KomaFree`, the `/free` toggle's mark and the
/// onboarding default) is NOT a deliberate provider choice and counts as unset.
///
/// Requiring BOTH scopes to be free/unset is load-bearing: a session temporarily toggled to
/// `/free` must NOT let an extension steal a real GLOBAL Main the user configured — only when
/// there is genuinely no real Main anywhere does the extension's default fill in. Mirrors
/// `commands::free::koma_free_main_idx`'s koma-free detector applied per scope.
fn main_is_unset_or_free(config: &AppConfig, settings: &Settings) -> bool {
    let is_koma_free = |e: &ModelEntry| {
        config
            .providers
            .iter()
            .any(|p| p.uuid == e.provider_uuid && p.api_type == ApiType::KomaFree)
    };
    // A REAL Main holder: holds Main AND is not koma-free-backed.
    let has_real_main = |models: &[ModelEntry]| {
        models
            .iter()
            .any(|e| e.effective_roles().contains(&ModelRole::Main) && !is_koma_free(e))
    };
    !has_real_main(&config.models) && !has_real_main(&settings.session_models)
}

/// W12b: VACUUM-FILL the Main role with the extension's preferred model `preferred_uuid` — but
/// ONLY when [`main_is_unset_or_free`] (Main unassigned / only the koma-free placeholder). This
/// is the "first vacuum-fill wins" gate: once a real model holds Main, a later extension's
/// default never fights it (it only surfaces the `recommendedBy` picker hint). Assigns Main via
/// the SAME `AppConfig::upsert_model` path the settings UI's `set_model` uses (per-role steal
/// by uuid), keeping the entry's existing model_id/name/provider_uuid and ADDING the Main role.
/// Returns `Some(model_name)` when it assigned (so the caller toasts), `None` otherwise.
fn try_vacuum_fill_main(
    config: &mut AppConfig,
    settings: &Settings,
    preferred_uuid: &str,
) -> Option<String> {
    if !main_is_unset_or_free(config, settings) {
        return None;
    }
    // The preferred model must still exist in the global catalogue (it was just registered).
    let mut entry = config.models.iter().find(|m| m.uuid == preferred_uuid)?.clone();
    let name = entry.name.clone();
    if !entry.roles.contains(&ModelRole::Main) {
        entry.roles.push(ModelRole::Main);
    }
    // Canonical Main-assignment path: per-role steal by uuid (same as the settings UI).
    config.upsert_model(entry);
    Some(name)
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
/// bounded by its own `SPAWN_CONNECT_TIMEOUT` of 3s — well under the reader's 120s
/// `EXT_CALL_TIMEOUT` default cap (this is not `models.invoke`), so no extra outer
/// timer is needed). On success, best-effort set the display `name` (the daemon
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
    use crate::model::app_config::{OAuthConn, OAuthProvider};

    /// EXHAUSTIVE grant-gate truth table — the security boundary, tested pure (no
    /// state). Columns: granted set × method → expected [`GateDecision`].
    #[test]
    fn grant_gate_truth_table() {
        use Grant::{AgentsOrchestrate as Orch, AgentsRead as Read};

        // Every recognised method partitioned by the grant it requires.
        let orchestrate_methods = ["agents.spawn", "agents.kill", "agents.send"];
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

        // --- WAVE-3/12 families: each needs its OWN grant, EXACT-MATCH, no lattice edge.
        use Grant::{ChatPrompt, ContextPublish, ModelsContribute, ModelsInvoke, SessionsManage};

        // (every verb in a family, the grant that family requires).
        let new_families: [(&[&str], Grant); 5] = [
            (
                &["sessions.list", "sessions.create", "sessions.switch", "sessions.spawn_into"],
                SessionsManage,
            ),
            (&["chat.prompt"], ChatPrompt),
            (&["models.invoke"], ModelsInvoke),
            // W12/W12b: models.register/unregister AND providers.register/unregister all need
            // `models:contribute`, DISTINCT from `models:invoke` despite sharing prefixes
            // (exact-verb gate). An extension that may contribute models may also contribute
            // the key-backed gateways that serve them.
            (
                &["models.register", "models.unregister", "providers.register", "providers.unregister"],
                ModelsContribute,
            ),
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

        // W12: `models:invoke` and `models:contribute` share the `models.` prefix but gate
        // DIFFERENT verbs — neither confers the other (exact-verb match, no prefix leak).
        assert_eq!(
            method_permitted("models.register", &[ModelsInvoke]),
            GateDecision::Deny(ModelsContribute),
            "models:invoke must NOT unlock models.register"
        );
        assert_eq!(
            method_permitted("models.invoke", &[ModelsContribute]),
            GateDecision::Deny(ModelsInvoke),
            "models:contribute must NOT unlock models.invoke"
        );
        assert_eq!(
            method_permitted("models.register", &[ModelsContribute]),
            GateDecision::Allow
        );
        assert_eq!(
            method_permitted("models.bogus", &[ModelsContribute]),
            GateDecision::UnknownMethod
        );

        // W12b: providers.register/unregister share the `models:contribute` grant with the
        // models.* contribution verbs, and stay exact-verb gated (no prefix leak).
        assert_eq!(
            method_permitted("providers.register", &[ModelsContribute]),
            GateDecision::Allow
        );
        assert_eq!(
            method_permitted("providers.unregister", &[ModelsContribute]),
            GateDecision::Allow
        );
        assert_eq!(
            method_permitted("providers.register", &[ModelsInvoke]),
            GateDecision::Deny(ModelsContribute),
            "models:invoke must NOT unlock providers.register"
        );
        assert_eq!(
            method_permitted("providers.bogus", &[ModelsContribute]),
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
            "models:contribute".to_string(),
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
                Grant::ModelsContribute,
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
            "providers.register",
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
        // A live injection sender the test can observe as "sent" for a Running
        // fixture; its receiver is dropped, so the send is a harmless no-op.
        let (inject_tx, _inject_rx) = tokio::sync::mpsc::unbounded_channel();
        SubAgent {
            id,
            agent_name: agent_name.to_string(),
            label: agent_name.to_string(),
            model_id: String::new(),
            status,
            abort,
            rx,
            inject_tx,
            transcript: Vec::new(),
            messages: Vec::new(),
            live_text: String::new(),
            tool_call_id: None,
            detached: false,
            nudged: false,
            ext_owned: false,
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

    /// `agents.send` steers a RUNNING sub-agent (`sent:true`), stashes onto a
    /// QUEUED one (`sent:true,status:queued` + the message lands in its
    /// `pending_injects`), and REFUSES a TERMINAL one (`agent is terminal`).
    /// Bad/absent params + unknown ids mirror the `agents.status` error shapes,
    /// and the verb is orchestrate-gated (a read-only grant is denied). All ids
    /// resolve through the extension's OWN registry, exactly like status/kill.
    #[test]
    fn send_steers_running_stashes_queued_refuses_terminal() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        // A running agent (steerable), a done agent (terminal), and a queued one.
        state.rest.sessions[0]
            .subagents
            .push(inert_subagent(rt.handle(), 3, "general", SubAgentStatus::Running));
        state.rest.sessions[0].subagents.push(inert_subagent(
            rt.handle(),
            4,
            "researcher",
            SubAgentStatus::Done("done".to_string()),
        ));
        state.rest.sessions[0]
            .pending_subagents
            .push_back(crate::app::subagent::PendingSubagent {
                id: 5,
                agent_name: "general".to_string(),
                prompt: "queued task".to_string(),
                tool_call_id: None,
                detached: false,
                ext_owned: false,
                overrides: None,
                pending_injects: Vec::new(),
            });
        let sess_uuid = state.rest.sessions[0].id.clone();
        let registry = state.rest.ext_agents.entry("test.ext".to_string()).or_default();
        let ext_running = registry.insert(sess_uuid.clone(), 3, false);
        let ext_done = registry.insert(sess_uuid.clone(), 4, false);
        let ext_queued = registry.insert(sess_uuid, 5, false);

        // Running → delivered.
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send",
            json!({ "agentId": ext_running, "message": "focus on the parser" }),
        );
        assert_eq!(out.get("sent").and_then(|v| v.as_bool()), Some(true), "running send must report sent, got {out}");
        assert!(out.get("status").is_none(), "a running send is not queued, got {out}");

        // Queued → stashed + status:queued, and the message lands in pending_injects.
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send",
            json!({ "agentId": ext_queued, "message": "also check tests" }),
        );
        assert_eq!(out.get("sent").and_then(|v| v.as_bool()), Some(true), "queued send must report sent, got {out}");
        assert_eq!(out.get("status").and_then(|v| v.as_str()), Some("queued"), "queued send must mark queued, got {out}");
        let pend = state.rest.sessions[0]
            .pending_subagents
            .iter()
            .find(|p| p.id == 5)
            .expect("queued agent still present");
        assert_eq!(pend.pending_injects, vec!["also check tests".to_string()], "queued send must stash the message");

        // Terminal → refused (nothing delivered).
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send",
            json!({ "agentId": ext_done, "message": "too late" }),
        );
        assert_eq!(out.get("error").and_then(|v| v.as_str()), Some("agent is terminal"), "terminal send must refuse, got {out}");

        // Unknown ext id → unknown agentId.
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send",
            json!({ "agentId": 9999, "message": "x" }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("unknown agentId")),
            "unknown id must error, got {out}"
        );

        // Missing agentId / empty message → their own validation errors.
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send", json!({ "message": "x" }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("requires an 'agentId'")),
            "missing agentId must error, got {out}"
        );
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsOrchestrate], "agents.send",
            json!({ "agentId": ext_running, "message": "   " }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("non-empty 'message'")),
            "empty message must error, got {out}"
        );

        // Orchestrate-gated: a read-only grant is denied outright.
        let out = call_broker(
            &mut state, rt.handle(), &client, "test.ext",
            &[Grant::AgentsRead], "agents.send",
            json!({ "agentId": ext_running, "message": "x" }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "read-only grant must deny agents.send, got {out}"
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

    // ─── W12 models.register / models.unregister ────────────────────────────────
    //
    // The catalogue mutation is exercised through the PURE `apply_models_*` cores so no test
    // ever writes `~/.koma/config.json` (the persisting broker wrappers save only on a real
    // change). The GRANT GATE for the verbs is proven end-to-end via `call_broker` on paths
    // that reply an error (so the wrapper never persists).

    /// An ext-backed OAuthConn owned by `ext_id`, either a MODEL provider (with the W12
    /// chat_endpoint + api_type meta) or account-login-only (no meta).
    fn ext_conn(uuid: &str, ext_id: &str, model_provider: bool) -> OAuthConn {
        OAuthConn {
            uuid: uuid.to_string(),
            provider: OAuthProvider::Extension,
            access_token: "at".to_string(),
            ext_id: Some(ext_id.to_string()),
            provider_id: Some("prov".to_string()),
            chat_endpoint: model_provider.then(|| "https://api.ext.test/v1".to_string()),
            api_type: model_provider.then(|| "openai".to_string()),
            ..Default::default()
        }
    }

    /// register with NO connected conn → the "no account" error; nothing added.
    #[test]
    fn models_register_without_conn_errors() {
        let mut config = AppConfig::default();
        let out = apply_models_register(
            &mut config,
            "my.ext",
            &json!({ "models": [{ "id": "m1", "name": "M1" }] }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("no connected oauth account")),
            "got {out}"
        );
        assert!(config.models.is_empty());
    }

    /// register when the ext's only conn is account-login-only (no meta) → "account login only".
    #[test]
    fn models_register_account_login_only_errors() {
        let mut config = AppConfig::default();
        config.oauth_conns.push(ext_conn("c1", "my.ext", false));
        let out = apply_models_register(
            &mut config,
            "my.ext",
            &json!({ "models": [{ "id": "m1", "name": "M1" }] }),
        );
        assert!(
            out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("account-login only")),
            "got {out}"
        );
        assert!(config.models.is_empty());
    }

    /// register mints role-less entries served by the ext conn, replies stable uuids; a
    /// re-register of the same (provider, id) UPDATES the name IN PLACE keeping the uuid.
    #[test]
    fn models_register_mints_and_dedupes_keeping_uuid() {
        let mut config = AppConfig::default();
        config.oauth_conns.push(ext_conn("conn-a", "my.ext", true));

        let out = apply_models_register(
            &mut config,
            "my.ext",
            &json!({ "models": [{ "id": "fast", "name": "Fast" }, { "id": "slow", "name": "Slow" }] }),
        );
        assert_eq!(out["registered"], json!(2));
        assert_eq!(out["uuids"].as_array().unwrap().len(), 2);
        assert_eq!(config.models.len(), 2);
        assert!(config.models.iter().all(|m| m.provider_uuid == "conn-a"), "served by the ext conn");
        assert!(config.models.iter().all(|m| m.roles.is_empty()), "ext models hold no runtime role");
        let fast_uuid = config.models.iter().find(|m| m.model_id == "fast").unwrap().uuid.clone();

        // Re-register "fast" with a NEW name → same uuid returned, name updated, no new entry.
        let out2 = apply_models_register(
            &mut config,
            "my.ext",
            &json!({ "models": [{ "id": "fast", "name": "Faster" }] }),
        );
        assert_eq!(out2["registered"], json!(1));
        assert_eq!(out2["uuids"][0], json!(fast_uuid), "dedupe returns the STABLE uuid");
        assert_eq!(config.models.len(), 2, "no new entry minted on re-register");
        let fast = config.models.iter().find(|m| m.model_id == "fast").unwrap();
        assert_eq!(fast.name, "Faster", "name updated in place");
        assert_eq!(fast.uuid, fast_uuid, "uuid preserved (stability contract)");
    }

    /// >100 models is rejected atomically; a batch with any invalid entry registers NONE.
    #[test]
    fn models_register_rejects_over_cap_and_bad_fields() {
        let mut config = AppConfig::default();
        config.oauth_conns.push(ext_conn("conn-a", "my.ext", true));

        let big: Vec<Value> = (0..101).map(|i| json!({ "id": format!("m{i}"), "name": "n" })).collect();
        let over = apply_models_register(&mut config, "my.ext", &json!({ "models": big }));
        assert!(
            over.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("too many models")),
            "got {over}"
        );
        assert!(config.models.is_empty(), "an over-cap batch registers nothing");

        // One empty id → the whole batch is rejected (atomic), nothing registered.
        let bad = apply_models_register(
            &mut config,
            "my.ext",
            &json!({ "models": [{ "id": "ok", "name": "OK" }, { "id": "", "name": "Bad" }] }),
        );
        assert!(bad.get("error").is_some(), "an empty id rejects the whole batch, got {bad}");
        assert!(config.models.is_empty(), "a batch with one bad entry registers NONE (atomic)");
    }

    /// Ownership wall + `ids` filter: a two-ext fixture proves ext A can NEVER remove ext B's
    /// entries, `ids` absent removes ALL of the caller's, and an id filter (case-insensitive)
    /// removes only the matching owned entry.
    #[test]
    fn models_unregister_ownership_wall_and_ids_filter() {
        let mut config = AppConfig::default();
        config.oauth_conns.push(ext_conn("conn-a", "ext.a", true));
        config.oauth_conns.push(ext_conn("conn-b", "ext.b", true));
        apply_models_register(
            &mut config,
            "ext.a",
            &json!({ "models": [{ "id": "a1", "name": "A1" }, { "id": "a2", "name": "A2" }] }),
        );
        apply_models_register(&mut config, "ext.b", &json!({ "models": [{ "id": "b1", "name": "B1" }] }));
        assert_eq!(config.models.len(), 3);

        // ext A tries to unregister B's model by id → the ownership wall blocks it (0 removed).
        let blocked = apply_models_unregister(&mut config, "ext.a", &json!({ "ids": ["b1"] }));
        assert_eq!(blocked["removed"], json!(0), "ext A cannot touch ext B's entry");
        assert_eq!(config.models.len(), 3);

        // ext A unregister with ids ABSENT → removes ALL of A's (2); B's untouched.
        let all_a = apply_models_unregister(&mut config, "ext.a", &json!({}));
        assert_eq!(all_a["removed"], json!(2));
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].model_id, "b1", "only ext B's entry remains");

        // ext B unregister by specific model_id (case-insensitive) → removes it.
        let b_by_id = apply_models_unregister(&mut config, "ext.b", &json!({ "ids": ["B1"] }));
        assert_eq!(b_by_id["removed"], json!(1));
        assert!(config.models.is_empty());
    }

    /// An extension with no connected conn removes nothing (empty ownership set), never
    /// touching another owner's models.
    #[test]
    fn models_unregister_no_conn_removes_nothing() {
        let mut config = AppConfig::default();
        config.models.push(ModelEntry {
            uuid: "x".to_string(),
            model_id: "m".to_string(),
            provider_uuid: "p".to_string(),
            ..ModelEntry::default()
        });
        let out = apply_models_unregister(&mut config, "my.ext", &json!({}));
        assert_eq!(out["removed"], json!(0));
        assert_eq!(config.models.len(), 1, "a foreign entry is never removed");
    }

    /// GATE-first end-to-end: an ungranted `models.register` is denied BEFORE the handler;
    /// a granted one reaches its real handler (which rejects an empty array). Both reply an
    /// error, so the persisting wrapper never writes config.json.
    #[test]
    fn models_register_gate_first_then_reaches_handler() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = fixture_state();
        let client: Option<Arc<OpenRouterClient>> = None;

        // models:invoke ≠ models:contribute → grant denied, never dispatched.
        let denied = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "my.ext",
            &[Grant::ModelsInvoke],
            "models.register",
            json!({ "models": [] }),
        );
        assert!(
            denied.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("grant denied")),
            "models:invoke must NOT unlock models.register, got {denied}"
        );

        // Granted → reaches the real handler's validation (empty array), not a stub/denial.
        let reached = call_broker(
            &mut state,
            rt.handle(),
            &client,
            "my.ext",
            &[Grant::ModelsContribute],
            "models.register",
            json!({ "models": [] }),
        );
        assert!(
            reached.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("at least one model")),
            "a granted models.register reaches its handler, got {reached}"
        );
    }

    // ─── W12b providers.register / providers.unregister + anchor generalization + vacuum-fill ─
    //
    // Exercised through the PURE cores (`apply_providers_*`, `apply_models_register`,
    // `try_vacuum_fill_main`, `main_is_unset_or_free`) so no test writes `~/.koma/config.json`;
    // the persisting wrappers save only on a real change. The grant gate is proven end-to-end in
    // `grant_gate_truth_table`.

    /// A KEY-BACKED ext provider owned by `ext_id`.
    fn ext_key_provider(uuid: &str, ext_id: &str, name: &str) -> ProviderConn {
        ProviderConn {
            uuid: uuid.to_string(),
            name: name.to_string(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "https://gw.test/v1".to_string(),
            api_key: "k".to_string(),
            ext_id: Some(ext_id.to_string()),
        }
    }

    /// providers.register mints a key-backed provider stamped with `ext_id`; a re-register of the
    /// same (ext, name) ROTATES key/endpoint/api_type in place KEEPING the uuid (the key-rotation
    /// contract — bound models keep resolving).
    #[test]
    fn providers_register_mints_and_rotates_keeping_uuid() {
        let mut config = AppConfig::default();
        let out = apply_providers_register(
            &mut config,
            "my.ext",
            &json!({ "name": "Gateway", "endpoint": "https://api.gw.test/v1", "api_type": "openai", "key": "sk-1" }),
        );
        let uuid = out["uuid"].as_str().expect("uuid replied").to_string();
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        assert_eq!(p.ext_id.as_deref(), Some("my.ext"), "stamped with the caller's ext id");
        assert_eq!(p.api_type, ApiType::OpenAiCompatible);
        assert_eq!(p.api_key, "sk-1");

        // Re-register the same name → rotate key + endpoint + api_type, SAME uuid, no new entry.
        let out2 = apply_providers_register(
            &mut config,
            "my.ext",
            &json!({ "name": "Gateway", "endpoint": "https://api.gw.test/v2", "api_type": "anthropic", "key": "sk-2" }),
        );
        assert_eq!(out2["uuid"].as_str(), Some(uuid.as_str()), "key-rotation keeps the uuid");
        assert_eq!(config.providers.len(), 1, "no new entry on rotation");
        let p = &config.providers[0];
        assert_eq!(p.api_key, "sk-2");
        assert_eq!(p.endpoint, "https://api.gw.test/v2");
        assert_eq!(p.api_type, ApiType::AnthropicCompatible);
    }

    /// providers.register rejects every invalid field (empty name, non-URL / non-http endpoint,
    /// bad api_type, empty key) and mutates nothing.
    #[test]
    fn providers_register_rejects_bad_input() {
        let mut config = AppConfig::default();
        let mut bad = |p: Value| apply_providers_register(&mut config, "my.ext", &p);
        assert!(bad(json!({ "name": "  ", "endpoint": "https://x.test", "api_type": "openai", "key": "k" })).get("error").is_some());
        assert!(bad(json!({ "name": "G", "endpoint": "not a url", "api_type": "openai", "key": "k" })).get("error").is_some());
        assert!(bad(json!({ "name": "G", "endpoint": "ftp://x.test", "api_type": "openai", "key": "k" })).get("error").is_some(), "non-http scheme rejected");
        assert!(bad(json!({ "name": "G", "endpoint": "https://x.test", "api_type": "codex", "key": "k" })).get("error").is_some(), "koma-free/codex wire types not injectable");
        assert!(bad(json!({ "name": "G", "endpoint": "https://x.test", "api_type": "openai", "key": "  " })).get("error").is_some());
        assert!(config.providers.is_empty(), "no invalid provider is ever stored");
    }

    /// providers.unregister enforces the ownership wall (never another ext's / a native
    /// provider), honours the id filter (uuid OR name, case-insensitive), and SWEEPS orphaned
    /// models.
    #[test]
    fn providers_unregister_ownership_wall_and_orphan_sweep() {
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a1", "ext.a", "A1"));
        config.providers.push(ext_key_provider("p-a2", "ext.a", "A2"));
        config.providers.push(ext_key_provider("p-b", "ext.b", "B"));
        config.providers.push(ProviderConn {
            uuid: "p-native".to_string(),
            name: "native".to_string(),
            ..Default::default()
        });
        for (u, prov) in [("m-a1", "p-a1"), ("m-a2", "p-a2"), ("m-b", "p-b"), ("m-native", "p-native")] {
            config.models.push(ModelEntry {
                uuid: u.to_string(),
                provider_uuid: prov.to_string(),
                ..Default::default()
            });
        }

        // ext A can never remove B's or native providers (ownership wall).
        let blocked = apply_providers_unregister(&mut config, "ext.a", &json!({ "ids": ["p-b", "p-native"] }));
        assert_eq!(blocked["removed"], json!(0));
        assert_eq!(config.providers.len(), 4);

        // ext A remove by NAME (case-insensitive) → removes A1 + its orphaned model only.
        let by_name = apply_providers_unregister(&mut config, "ext.a", &json!({ "ids": ["a1"] }));
        assert_eq!(by_name["removed"], json!(1));
        assert!(config.providers.iter().all(|p| p.uuid != "p-a1"));
        assert!(config.models.iter().all(|m| m.provider_uuid != "p-a1"), "orphaned model swept");
        assert!(config.models.iter().any(|m| m.uuid == "m-a2"), "A2's model survives");

        // ext A remove ALL (ids absent) → removes A2 + its model; B + native untouched.
        let all_a = apply_providers_unregister(&mut config, "ext.a", &json!({}));
        assert_eq!(all_a["removed"], json!(1));
        assert!(config.providers.iter().all(|p| p.ext_id.as_deref() != Some("ext.a")));
        assert!(config.providers.iter().any(|p| p.uuid == "p-b"), "B untouched");
        assert!(config.providers.iter().any(|p| p.uuid == "p-native"), "native untouched");
        assert!(config.models.iter().any(|m| m.uuid == "m-native"), "native model untouched");
    }

    /// models.register anchors on a KEY-BACKED ext provider when that's the ext's only anchor
    /// (no oauth conn needed anymore — the W12b generalization).
    #[test]
    fn models_register_anchors_on_key_backed_provider() {
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        let out = apply_models_register(&mut config, "my.ext", &json!({ "models": [{ "id": "m1", "name": "M1" }] }));
        assert_eq!(out["registered"], json!(1));
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].provider_uuid, "p-a", "served by the key-backed provider");
    }

    /// The explicit `{ provider }` param must be caller-owned; an account-login-only conn is
    /// rejected; two eligible anchors without a provider param are ambiguous.
    #[test]
    fn models_register_provider_param_and_ambiguity() {
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        config.oauth_conns.push(ext_conn("c-a", "my.ext", true)); // second usable anchor
        config.oauth_conns.push(ext_conn("c-login", "my.ext", false)); // account-login-only

        // Two eligible anchors + no provider param → ambiguous.
        let ambiguous = apply_models_register(&mut config, "my.ext", &json!({ "models": [{ "id": "m", "name": "M" }] }));
        assert!(
            ambiguous.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("multiple providers")),
            "got {ambiguous}"
        );

        // Explicit provider not owned → rejected.
        let not_owned = apply_models_register(&mut config, "my.ext", &json!({ "provider": "someone-else", "models": [{ "id": "m", "name": "M" }] }));
        assert!(
            not_owned.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("not owned")),
            "got {not_owned}"
        );

        // Explicit account-login-only conn → rejected.
        let login_only = apply_models_register(&mut config, "my.ext", &json!({ "provider": "c-login", "models": [{ "id": "m", "name": "M" }] }));
        assert!(
            login_only.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("account-login only")),
            "got {login_only}"
        );

        // Explicit owned key-backed provider → registers there.
        let ok = apply_models_register(&mut config, "my.ext", &json!({ "provider": "p-a", "models": [{ "id": "m", "name": "M" }] }));
        assert_eq!(ok["registered"], json!(1));
        assert_eq!(config.models.iter().find(|m| m.model_id == "m").unwrap().provider_uuid, "p-a");
    }

    /// `default: true` records the ext's preferred model + echoes `defaultUuid`; more than one
    /// default in a single call is rejected ATOMICALLY (nothing registered).
    #[test]
    fn models_register_default_records_preferred() {
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a", "my.ext", "GW"));

        let two = apply_models_register(&mut config, "my.ext", &json!({ "models": [
            { "id": "a", "name": "A", "default": true },
            { "id": "b", "name": "B", "default": true },
        ] }));
        assert!(
            two.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("multiple defaults")),
            "got {two}"
        );
        assert!(config.models.is_empty(), "a multi-default batch registers nothing");

        let out = apply_models_register(&mut config, "my.ext", &json!({ "models": [
            { "id": "a", "name": "A" },
            { "id": "b", "name": "B", "default": true },
        ] }));
        assert_eq!(out["registered"], json!(2));
        let du = out["defaultUuid"].as_str().expect("defaultUuid echoed");
        assert_eq!(config.ext_preferred_models.get("my.ext").map(String::as_str), Some(du));
        assert_eq!(config.models.iter().find(|m| m.model_id == "b").unwrap().uuid, du, "the flagged entry's uuid");
    }

    /// VACUUM-FILL: when Main is unset the preferred model is assigned Main (returns its name); a
    /// koma-free placeholder Main also counts as unset (fill wins + steals Main); a REAL user Main
    /// is untouched.
    #[test]
    fn vacuum_fill_only_when_main_unset_or_free() {
        let settings = Settings::default();

        // Case 1: Main unset → fill.
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        config.models.push(ModelEntry {
            uuid: "m-pref".to_string(),
            name: "Big".to_string(),
            provider_uuid: "p-a".to_string(),
            ..Default::default()
        });
        assert!(main_is_unset_or_free(&config, &settings));
        assert_eq!(try_vacuum_fill_main(&mut config, &settings, "m-pref").as_deref(), Some("Big"));
        assert!(
            config.models.iter().find(|m| m.uuid == "m-pref").unwrap().effective_roles().contains(&ModelRole::Main),
            "Main assigned"
        );

        // Case 2: a REAL user Main already set → NOT filled.
        let mut c2 = AppConfig::default();
        c2.providers.push(ProviderConn {
            uuid: "p-real".to_string(),
            name: "real".to_string(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "https://x/v1".to_string(),
            api_key: "sk".to_string(),
            ext_id: None,
        });
        c2.models.push(ModelEntry {
            uuid: "m-user".to_string(),
            provider_uuid: "p-real".to_string(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        c2.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        c2.models.push(ModelEntry {
            uuid: "m-pref".to_string(),
            name: "Big".to_string(),
            provider_uuid: "p-a".to_string(),
            ..Default::default()
        });
        assert!(!main_is_unset_or_free(&c2, &settings), "a real Main is set");
        assert_eq!(try_vacuum_fill_main(&mut c2, &settings, "m-pref"), None);
        assert!(
            c2.models.iter().find(|m| m.uuid == "m-user").unwrap().effective_roles().contains(&ModelRole::Main),
            "user Main kept"
        );

        // Case 3: koma-free placeholder Main counts as unset → fill (steals Main from placeholder).
        let mut c3 = AppConfig::default();
        c3.providers.push(ProviderConn {
            uuid: "koma".to_string(),
            name: "koma free".to_string(),
            api_type: ApiType::KomaFree,
            endpoint: "https://kf/v1".to_string(),
            api_key: String::new(),
            ext_id: None,
        });
        c3.models.push(ModelEntry {
            uuid: "kf".to_string(),
            provider_uuid: "koma".to_string(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        c3.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        c3.models.push(ModelEntry {
            uuid: "m-pref".to_string(),
            name: "Big".to_string(),
            provider_uuid: "p-a".to_string(),
            ..Default::default()
        });
        assert!(main_is_unset_or_free(&c3, &settings), "koma-free placeholder counts as unset");
        assert_eq!(try_vacuum_fill_main(&mut c3, &settings, "m-pref").as_deref(), Some("Big"));
        assert!(c3.models.iter().find(|m| m.uuid == "m-pref").unwrap().effective_roles().contains(&ModelRole::Main));
        assert!(
            !c3.models.iter().find(|m| m.uuid == "kf").unwrap().effective_roles().contains(&ModelRole::Main),
            "placeholder lost Main (per-role steal)"
        );

        // Case 4 (hardening): a session TEMPORARILY on /free (a session-local koma-free Main
        // override) must NOT let an extension steal a REAL global Main the user configured —
        // both scopes must be free/unset before a fill.
        let mut c4 = AppConfig::default();
        c4.providers.push(ProviderConn {
            uuid: "koma".to_string(),
            name: "koma free".to_string(),
            api_type: ApiType::KomaFree,
            endpoint: "https://kf/v1".to_string(),
            api_key: String::new(),
            ext_id: None,
        });
        c4.providers.push(ProviderConn {
            uuid: "p-real".to_string(),
            name: "real".to_string(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "https://x/v1".to_string(),
            api_key: "sk".to_string(),
            ext_id: None,
        });
        c4.models.push(ModelEntry {
            uuid: "m-global".to_string(),
            provider_uuid: "p-real".to_string(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        c4.providers.push(ext_key_provider("p-a", "my.ext", "GW"));
        c4.models.push(ModelEntry {
            uuid: "m-pref".to_string(),
            name: "Big".to_string(),
            provider_uuid: "p-a".to_string(),
            ..Default::default()
        });
        let mut free_settings = Settings::default();
        free_settings.session_models.push(ModelEntry {
            uuid: "s-kf".to_string(),
            provider_uuid: "koma".to_string(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        assert!(
            !main_is_unset_or_free(&c4, &free_settings),
            "a session on /free must NOT expose a real global Main to a steal"
        );
        assert_eq!(try_vacuum_fill_main(&mut c4, &free_settings, "m-pref"), None);
        assert!(
            c4.models.iter().find(|m| m.uuid == "m-global").unwrap().effective_roles().contains(&ModelRole::Main),
            "the real global Main is kept"
        );
    }

    /// TWO extensions registering defaults do not fight: the first vacuum-fills Main; the second
    /// finds Main already a real choice and only records its preference (drives the picker hint).
    #[test]
    fn two_exts_defaults_first_vacuum_wins() {
        let mut config = AppConfig::default();
        config.providers.push(ext_key_provider("p-a", "ext.a", "A"));
        config.providers.push(ext_key_provider("p-b", "ext.b", "B"));
        let settings = Settings::default();

        // ext A registers a default → vacuum-fill Main (simulate the wrapper: apply then fill).
        let a = apply_models_register(&mut config, "ext.a", &json!({ "models": [{ "id": "am", "name": "Amodel", "default": true }] }));
        let a_uuid = a["defaultUuid"].as_str().unwrap().to_string();
        assert_eq!(try_vacuum_fill_main(&mut config, &settings, &a_uuid).as_deref(), Some("Amodel"));

        // ext B registers a default → Main is now A's (a real provider) → NO fill.
        let b = apply_models_register(&mut config, "ext.b", &json!({ "models": [{ "id": "bm", "name": "Bmodel", "default": true }] }));
        let b_uuid = b["defaultUuid"].as_str().unwrap().to_string();
        assert_eq!(try_vacuum_fill_main(&mut config, &settings, &b_uuid), None, "second ext must not fight");

        // Main still A's; BOTH preferences recorded (B's drives the `recommendedBy` hint).
        let main = config.models.iter().find(|m| m.effective_roles().contains(&ModelRole::Main)).unwrap();
        assert_eq!(main.model_id, "am");
        assert_eq!(config.ext_preferred_models.get("ext.a").map(String::as_str), Some(a_uuid.as_str()));
        assert_eq!(config.ext_preferred_models.get("ext.b").map(String::as_str), Some(b_uuid.as_str()));
    }
}

// W13: additional regression suite — pure addition, sibling file, never touches the `tests`
// module above.
#[cfg(test)]
#[path = "broker_test.rs"]
mod broker_test;
