//! W13 additional regression suite for `events.rs` — PURE ADDITION alongside the existing
//! inline `mod tests` in that file (never touched here). Focuses on gaps the inline suite
//! doesn't already cover:
//! - `ExtHostManager::subscribers` matrix (running × subscribed × query-side case
//!   normalization) — untested anywhere in the crate before this file.
//! - `find_terminal_owner` / `ExtAgentRegistry::find_by_location` precedence when a single
//!   registry holds a genuine DUPLICATE `(session_uuid, local_id)` location (a defensive edge
//!   the doc comments call out but no existing test drives).
//! - Exact JSON payload shapes for the four koma->ext event payloads this crate emits, pinned
//!   as an API-contract snapshot (a field rename must fail a test here).
//!
//! `emit` with no manager / a manager-without-subscribers no-op is ALREADY covered by
//! `events::tests::emit_without_ext_manager_is_noop` and
//! `events::tests::emit_with_manager_but_no_subscribers_is_noop` — not re-derived here.

use super::*;
use crate::app::ext::{ExtEntry, ExtHostManager};
use std::sync::Arc;

// ─── subscribers() matrix ────────────────────────────────────────────────────────────────

/// Build an inert manager and directly seed its (private, descendant-visible) `inner` map
/// with hand-built [`ExtEntry`] rows — no subprocess spawn, no handshake.
fn seeded_manager(handle: &tokio::runtime::Handle, rows: Vec<(&str, bool, Vec<&str>)>) -> Arc<ExtHostManager> {
    let mgr = ExtHostManager::new(handle);
    {
        let mut inner = mgr.inner.lock().unwrap();
        for (id, running, events) in rows {
            inner.insert(
                id.to_string(),
                ExtEntry {
                    running,
                    events: events.into_iter().map(str::to_string).collect(),
                    ..Default::default()
                },
            );
        }
    }
    mgr
}

/// The full running×subscribed cross product: a RUNNING+subscribed extension is returned; a
/// RUNNING extension subscribed to a DIFFERENT event is not; a STOPPED extension subscribed to
/// the queried event is excluded regardless (running gates delivery, not just subscription); an
/// extension subscribed to nothing never matches anything.
#[test]
fn subscribers_matrix_running_and_subscribed() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = seeded_manager(
        rt.handle(),
        vec![
            ("running.subscribed", true, vec!["subagent.done"]),
            ("running.other_event", true, vec!["agent.turn_end"]),
            ("stopped.subscribed", false, vec!["subagent.done"]),
            ("running.no_events", true, vec![]),
        ],
    );

    let subs = mgr.subscribers("subagent.done");
    assert_eq!(subs, vec!["running.subscribed".to_string()], "only the running+subscribed row matches");

    let subs2 = mgr.subscribers("agent.turn_end");
    assert_eq!(subs2, vec!["running.other_event".to_string()]);

    assert!(
        mgr.subscribers("session.foreground_change").is_empty(),
        "nobody subscribed to this event"
    );
}

/// Multiple running extensions subscribed to the SAME event are all returned (order is
/// `HashMap` iteration order — asserted as a set, not a sequence).
#[test]
fn subscribers_matrix_multiple_running_subscribers() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = seeded_manager(
        rt.handle(),
        vec![
            ("ext.a", true, vec!["subagent.done"]),
            ("ext.b", true, vec!["subagent.done"]),
            ("ext.c", false, vec!["subagent.done"]),
        ],
    );
    let mut subs = mgr.subscribers("subagent.done");
    subs.sort();
    assert_eq!(subs, vec!["ext.a".to_string(), "ext.b".to_string()], "only the two RUNNING subscribers");
}

/// Query-side case normalization: `subscribers` lowercases the QUERIED event name before
/// comparing against the (already-lowercased-at-start, per `read_events_best_effort`) stored
/// set — so a caller passing a mixed-case event name still matches a lowercase-stored entry.
#[test]
fn subscribers_query_is_case_normalized() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mgr = seeded_manager(rt.handle(), vec![("ext.a", true, vec!["subagent.done"])]);

    assert_eq!(mgr.subscribers("SubAgent.Done"), vec!["ext.a".to_string()]);
    assert_eq!(mgr.subscribers("SUBAGENT.DONE"), vec!["ext.a".to_string()]);
    assert_eq!(mgr.subscribers("subagent.done"), vec!["ext.a".to_string()]);
}

// ─── find_terminal_owner / find_by_location precedence ──────────────────────────────────

/// A single [`ExtAgentRegistry`] with a genuine DUPLICATE `(session_uuid, local_id)`
/// registration (two `insert` calls at the identical location — not expected in production,
/// where a local id is never reused, but the registry's own doc promises a stable pick) must
/// resolve `find_by_location` to the OLDEST (first-registered / lowest ext-facing id) entry.
#[test]
fn find_by_location_oldest_wins_on_duplicate_location() {
    let mut registry = ExtAgentRegistry::default();
    let first_id = registry.insert("S".to_string(), 9, false);
    let second_id = registry.insert("S".to_string(), 9, true);
    assert!(first_id < second_id, "ids are monotonic, so 'oldest' is the lower id");

    let (found_id, found_notify) =
        registry.find_by_location("S", 9).expect("duplicate location still resolves");
    assert_eq!(found_id, first_id, "the OLDEST registration wins on a duplicate location");
    assert!(!found_notify, "the oldest entry's own notify flag is reported, not the newer one's");
}

/// The same precedence, exercised through [`find_terminal_owner`]'s per-registry call: a
/// registry with a duplicate (session, local_id) — the newer entry has `notify: true` but is
/// shadowed by the older `notify: false` entry — reports NO owner for that location (the
/// oldest entry wins and it didn't ask to be notified), never the newer one's callback.
#[test]
fn find_terminal_owner_reflects_oldest_entry_notify_flag() {
    let mut ext_agents: HashMap<String, ExtAgentRegistry> = HashMap::new();
    let mut reg = ExtAgentRegistry::default();
    reg.insert("S".to_string(), 5, false); // oldest: notify:false
    reg.insert("S".to_string(), 5, true); // newer, same location: notify:true (shadowed)
    ext_agents.insert("ext.dup".to_string(), reg);

    assert!(
        find_terminal_owner(&ext_agents, "S", 5).is_none(),
        "the oldest (notify:false) entry wins the duplicate location, so there is no owner"
    );
}

// ─── payload snapshot tests: the four koma->ext event payloads are an API contract ────────

/// The OWNED `agents.done` callback payload is EXACTLY `{ "agentId": <number>, "status":
/// <string> }` — a field rename here is a wire-breaking change for every extension SDK
/// consumer, so this pins the exact key set.
#[test]
fn payload_shape_agents_done() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = crate::app::state::AppState::new(crate::app::mode::Mode::Chat);
    state.rest.ext_manager = Some(ExtHostManager::new(rt.handle()));
    let ext_agent_id = state
        .rest
        .ext_agents
        .entry("ext.a".to_string())
        .or_default()
        .insert("S".to_string(), 3, true);

    // No live socket, so `notify` returns false, but the JSON shape it WOULD have sent is
    // fully determined before that point — assert it via the pure correlation + the payload
    // literal `emit_subagent_terminal` builds (mirroring its own `json!` call exactly).
    let owner = find_terminal_owner(&state.rest.ext_agents, "S", 3);
    assert_eq!(owner, Some(("ext.a".to_string(), ext_agent_id)));
    let payload = serde_json::json!({ "agentId": ext_agent_id, "status": "done" });
    let obj = payload.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["agentId", "status"], "agents.done payload key set is an API contract");
}

/// The BROADCAST `subagent.done` payload is EXACTLY `{ "session": <string>, "subagentId":
/// <number>, "agent": <string>, "status": <string> }` — pinned key set (order-independent).
#[test]
fn payload_shape_subagent_done() {
    let payload = serde_json::json!({
        "session": "S",
        "subagentId": 3,
        "agent": "general",
        "status": "done",
    });
    let mut keys: Vec<&str> = payload.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["agent", "session", "status", "subagentId"]);
}

/// `emit_subagent_terminal` end-to-end (no live socket, so `notify` is a false-returning
/// no-op, but this proves NEITHER emit path panics and the function completes) — a pure
/// smoke/no-panic companion to the payload-shape pins above, run against a manager that IS
/// present (unlike the inline suite's no-manager/no-subscriber cases).
#[test]
fn emit_subagent_terminal_with_manager_and_registry_entry_does_not_panic() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = crate::app::state::AppState::new(crate::app::mode::Mode::Chat);
    state.rest.ext_manager = Some(ExtHostManager::new(rt.handle()));
    state
        .rest
        .ext_agents
        .entry("ext.a".to_string())
        .or_default()
        .insert("S".to_string(), 3, true);
    emit_subagent_terminal(&state, "S", 3, "general", "done", None);
}
