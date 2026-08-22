#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::app::mode::Mode;

/// A fresh [`AppState`] carries no `ext_manager` and an empty `ext_agents`, so
/// both emit entry points must be pure no-ops (never panic) — the zero-extension
/// baseline every install of koma runs at.
#[test]
fn emit_without_ext_manager_is_noop() {
    let state = AppState::new(Mode::Chat);
    emit(&state, "agent.turn_end", &json!({ "session": "s" }));
    emit(
        &state,
        "session.foreground_change",
        &json!({ "session": "s" }),
    );
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
    emit(
        &state,
        "subagent.done",
        &json!({ "session": "s", "subagentId": 1 }),
    );
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
