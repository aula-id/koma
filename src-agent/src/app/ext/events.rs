//! koma->ext EVENT FAN-OUT (Wave 5).
//!
//! The emission side of the extension event system. koma-internal lifecycle
//! moments — a sub-agent reaching a terminal state, a chat turn ending, a
//! foreground switch — are broadcast to the RUNNING daemon extensions that
//! subscribed to them under `contributes.events`, plus a targeted `agents.done`
//! callback to the extension that spawned a now-terminal sub-agent (when it asked
//! for one via `agents.spawn { notify: true }`).
//!
//! ## Two delivery shapes
//!
//! - **Subscribed broadcast** ([`emit`]): fan a named event out to EVERY running
//!   extension whose manifest `contributes.events` lists it. `subagent.done`,
//!   `agent.turn_end`, and `session.foreground_change` all travel this way.
//! - **Owned callback** ([`emit_subagent_terminal`]'s `agents.done`): delivered
//!   ONLY to the single extension that spawned the sub-agent with `notify: true`,
//!   and NOT subscription-gated — it is a direct answer to that extension's own
//!   `agents.spawn`, independent of `contributes.events`.
//!
//! No report/transcript text is ever placed in a payload — only ids, a session
//! uuid, an agent name, and a terminal status label.
//!
//! ## Safe on the event loop
//!
//! Every function here is synchronous and NON-BLOCKING. [`ExtHostManager::notify`]
//! only serializes a frame and hands it to the per-extension writer channel (an
//! unbounded `mpsc::send`); it never awaits and never blocks. So these emit points
//! can be called directly from the event loop / action handlers with no risk of
//! stalling a tick.
//!
//! ## Zero-subscriber no-op
//!
//! With no `ext_manager` (the common case — no extensions installed) every emit is
//! an early return. With an `ext_manager` but nothing subscribed to the event,
//! [`ExtHostManager::subscribers`] returns empty and the fan-out loop does nothing.
//! Either way the surrounding flow (toasts, `was_working`, kill semantics) is
//! completely unaffected — the event system is purely additive.
//!
//! [`ExtHostManager::notify`]: crate::app::ext::ExtHostManager::notify
//! [`ExtHostManager::subscribers`]: crate::app::ext::ExtHostManager::subscribers

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::app::ext::ExtAgentRegistry;
use crate::app::state::AppState;

/// Non-blocking broadcast to every RUNNING daemon extension whose manifest
/// subscribes to `event`. Safe on the event loop: `notify()` only queues on the
/// writer channel. No `ext_manager` (no extensions installed) or no subscribers ->
/// pure no-op.
pub fn emit(state: &AppState, event: &str, params: &Value) {
    let Some(mgr) = state.rest.ext_manager.as_ref() else {
        return;
    };
    for ext_id in mgr.subscribers(event) {
        // Best-effort per subscriber; a not-running subscriber (raced a stop) just
        // returns false. `params` is cloned per recipient — each writer owns its frame.
        mgr.notify(&ext_id, event, params.clone());
    }
}

/// A sub-agent reached a terminal state (`status` is `"done"` | `"error"` |
/// `"killed"`). Fan out the two events this wave owns, in this order:
///
/// 1. The OWNED `agents.done` callback — delivered ONLY to the extension that
///    spawned this sub-agent with `notify: true`, and NOT subscription-gated (it is
///    a direct answer to that extension's own spawn, independent of
///    `contributes.events`). Payload `{ agentId, status }` carries the EXT-FACING
///    agent id that extension was handed, never koma's raw session-local id.
/// 2. The subscribed `subagent.done` BROADCAST — delivered to every running
///    extension subscribed to `subagent.done`, regardless of who (if anyone)
///    spawned the sub-agent. Payload `{ session, subagentId, agent, status }`
///    carries the STABLE session uuid, the per-session sub-agent id, the agent
///    name, and the status.
///
/// `session_uuid` is the sub-agent's stable session uuid; `local_id` its
/// per-session sub-agent id — the pair [`ExtAgentRegistry::find_by_location`]
/// correlates back to a spawner. `error` is `Some(text)` for an `"error"`
/// settlement (the [`SubAgentStatus::Error`](crate::app::subagent::SubAgentStatus::Error)
/// text) AND for a `"killed"` settlement that carries a REASON — the daemon-shutdown
/// death notice (see `lifecycle::notify_ext_owned_subagents_on_shutdown`) passes
/// `Some("daemon restart")` so a notify:true spawner can tell a host restart from a
/// genuine kill (a user Ctrl+X kill via `broker_kill` passes `None`, so its payload is
/// unchanged). It is carried into the `agents.done` payload as an ADDITIVE `"error"`
/// field so a notify:true spawner learns WHY its sub-agent died without a separate
/// `agents.result` round-trip. Old extensions that don't read the field are
/// unaffected (they still see `agentId`/`status`); `agents.result` remains the
/// pull path for the full terminal payload (including the `"output"` report on a
/// `done` settlement, which never travels over this event).
pub fn emit_subagent_terminal(
    state: &AppState,
    session_uuid: &str,
    local_id: usize,
    agent: &str,
    status: &str,
    error: Option<&str>,
) {
    // 1. Owned agents.done callback -> the notify:true spawner only (if any). This
    //    is deliberately NOT gated on a `subagent.done` subscription: an extension
    //    that spawned with notify:true gets its completion callback whether or not
    //    it also subscribes to the broadcast.
    if let Some(mgr) = state.rest.ext_manager.as_ref() {
        if let Some((spawner_ext, ext_agent_id)) =
            find_terminal_owner(&state.rest.ext_agents, session_uuid, local_id)
        {
            let mut payload = json!({ "agentId": ext_agent_id, "status": status });
            // The optional `error` reason rides along on an `"error"` settlement (the
            // failure text) AND on a `"killed"` settlement that carries a reason — the
            // daemon-shutdown death notice passes `Some("daemon restart")` so a
            // notify:true spawner can tell a host restart from a real kill. A caller that
            // passes `None` (e.g. `broker_kill`'s user Ctrl+X) adds no field, so its
            // payload is unchanged.
            if status == "error" || status == "killed" {
                if let Some(e) = error {
                    payload["error"] = json!(e);
                }
            }
            mgr.notify(&spawner_ext, "agents.done", payload);
        }
    }

    // 2. Subscribed subagent.done broadcast -> every subscriber, regardless of owner.
    emit(
        state,
        "subagent.done",
        &json!({
            "session": session_uuid,
            "subagentId": local_id,
            "agent": agent,
            "status": status,
        }),
    );
}

/// PURE correlation lookup for [`emit_subagent_terminal`]'s owned `agents.done`
/// callback: reverse-scan every per-extension registry for the one that spawned the
/// sub-agent living at `(session_uuid, local_id)` AND asked to be notified on
/// completion (`agents.spawn { notify: true }`). Returns that extension's id + the
/// EXT-FACING agent id it was handed, or `None` when no notify:true owner spawned
/// it (a poll-only spawn, or a sub-agent no extension spawned at all).
///
/// A given `(session_uuid, local_id)` names exactly ONE physical sub-agent, created
/// by exactly one `agents.spawn`, so at most one registry ever matches — the
/// `HashMap` iteration order is therefore immaterial. Factored out of the
/// `ext_manager`-touching shell above so the correlation (the interesting logic) can
/// be unit-tested over plain registries without faking an `ExtHostManager`.
fn find_terminal_owner(
    ext_agents: &HashMap<String, ExtAgentRegistry>,
    session_uuid: &str,
    local_id: usize,
) -> Option<(String, u64)> {
    ext_agents.iter().find_map(|(ext_id, registry)| {
        registry
            .find_by_location(session_uuid, local_id)
            .filter(|(_, notify)| *notify)
            .map(|(ext_agent_id, _)| (ext_id.clone(), ext_agent_id))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::app::mode::Mode;

    /// A fresh [`AppState`] carries no `ext_manager` and an empty `ext_agents`, so
    /// both emit entry points must be pure no-ops (never panic) — the zero-extension
    /// baseline every install of koma runs at.
    #[test]
    fn emit_without_ext_manager_is_noop() {
        let state = AppState::new(Mode::Chat);
        emit(&state, "agent.turn_end", &json!({ "session": "s" }));
        emit(&state, "session.foreground_change", &json!({ "session": "s" }));
        emit_subagent_terminal(&state, "s", 1, "general", "done", None);
    }

    /// With an `ext_manager` present but NO running extensions,
    /// [`ExtHostManager::subscribers`] is empty and every emit still no-ops (the
    /// fan-out loop iterates nothing) — proving the mgr-touching shell is safe with a
    /// real manager, not just a `None` one.
    #[test]
    fn emit_with_manager_but_no_subscribers_is_noop() {
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let mut state = AppState::new(Mode::Chat);
        state.rest.ext_manager = Some(crate::app::ext::ExtHostManager::new(rt.handle()));
        emit(&state, "subagent.done", &json!({ "session": "s", "subagentId": 1 }));
        emit_subagent_terminal(&state, "s", 1, "general", "killed", None);
    }

    /// The correlation lookup that decides who gets the owned `agents.done`: build
    /// two ext registries where ext A spawned into session S at local 3 (notify) and
    /// ext B spawned at the SAME local id 3 but a DIFFERENT session S2 (also notify,
    /// so the winner is chosen by SESSION, not merely the local id or the flag).
    /// A terminal at `(S, 3)` must resolve to A only; `(S2, 3)` to B only.
    #[test]
    fn find_terminal_owner_correlates_notify_spawner_by_session() {
        let mut ext_agents: HashMap<String, ExtAgentRegistry> = HashMap::new();

        let mut reg_a = ExtAgentRegistry::default();
        let a_ext_agent_id = reg_a.insert("S".to_string(), 3, true);
        ext_agents.insert("ext.a".to_string(), reg_a);

        let mut reg_b = ExtAgentRegistry::default();
        let b_ext_agent_id = reg_b.insert("S2".to_string(), 3, true);
        ext_agents.insert("ext.b".to_string(), reg_b);

        // (session S, local 3) resolves to ext A only — never ext B (scoped to S2).
        assert_eq!(
            find_terminal_owner(&ext_agents, "S", 3),
            Some(("ext.a".to_string(), a_ext_agent_id)),
            "session S / local 3 must correlate to ext A, its notify:true spawner"
        );
        // (session S2, local 3) resolves to ext B only.
        assert_eq!(
            find_terminal_owner(&ext_agents, "S2", 3),
            Some(("ext.b".to_string(), b_ext_agent_id)),
            "session S2 / local 3 must correlate to ext B, not A (same local id, other session)"
        );

        // A location nobody spawned into → no owner (no spurious agents.done).
        assert!(find_terminal_owner(&ext_agents, "S", 99).is_none());
        assert!(find_terminal_owner(&ext_agents, "S-nope", 3).is_none());
    }

    /// A sub-agent spawned WITHOUT `notify` (poll-only, today's default) has no
    /// terminal owner — it never receives an owned `agents.done` (only the
    /// subscribed `subagent.done` broadcast, which is not gated on ownership).
    #[test]
    fn find_terminal_owner_skips_non_notify_spawner() {
        let mut ext_agents: HashMap<String, ExtAgentRegistry> = HashMap::new();
        let mut reg = ExtAgentRegistry::default();
        reg.insert("S".to_string(), 7, false);
        ext_agents.insert("ext.poll".to_string(), reg);

        assert!(
            find_terminal_owner(&ext_agents, "S", 7).is_none(),
            "a notify:false spawner must not be a terminal owner"
        );
    }
}

// W13: additional regression suite — pure addition, sibling file, never touches the `tests`
// module above.
#[cfg(test)]
#[path = "events_test.rs"]
mod events_test;
