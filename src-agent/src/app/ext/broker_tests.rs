#![allow(clippy::unwrap_used, clippy::expect_used)]
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
        assert_eq!(
            method_permitted(m, &[]),
            GateDecision::Deny(Orch),
            "empty grants must deny {m}"
        );
    }
    for m in read_methods {
        assert_eq!(
            method_permitted(m, &[]),
            GateDecision::Deny(Read),
            "empty grants must deny {m}"
        );
    }

    // granted = [AgentsRead] → read methods ALLOW, orchestrate methods DENY.
    for m in read_methods {
        assert_eq!(
            method_permitted(m, &[Read]),
            GateDecision::Allow,
            "read grant must allow {m}"
        );
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
        assert_eq!(
            method_permitted(m, &[Read, Orch]),
            GateDecision::Allow,
            "full grants must allow {m}"
        );
    }

    // An unrecognised verb is never a silent allow.
    assert_eq!(
        method_permitted("agents.bogus", &[Orch]),
        GateDecision::UnknownMethod
    );
    assert_eq!(
        method_permitted("filesystem.read", &[Orch]),
        GateDecision::UnknownMethod
    );

    // --- WAVE-3/12 families: each needs its OWN grant, EXACT-MATCH, no lattice edge.
    use Grant::{ChatPrompt, ContextPublish, ModelsContribute, ModelsInvoke, SessionsManage};

    // (every verb in a family, the grant that family requires).
    let new_families: [(&[&str], Grant); 5] = [
        (
            &[
                "sessions.list",
                "sessions.create",
                "sessions.switch",
                "sessions.spawn_into",
            ],
            SessionsManage,
        ),
        (&["chat.prompt"], ChatPrompt),
        (&["models.invoke"], ModelsInvoke),
        // W12/W12b: models.register/unregister AND providers.register/unregister all need
        // `models:contribute`, DISTINCT from `models:invoke` despite sharing prefixes
        // (exact-verb gate). An extension that may contribute models may also contribute
        // the key-backed gateways that serve them.
        (
            &[
                "models.register",
                "models.unregister",
                "providers.register",
                "providers.unregister",
            ],
            ModelsContribute,
        ),
        (&["context.set", "context.clear"], ContextPublish),
    ];

    for (methods, own) in new_families {
        // An UNRELATED grant to probe cross-family isolation (never `own`).
        let unrelated = if own == SessionsManage {
            ChatPrompt
        } else {
            SessionsManage
        };
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
    assert_eq!(
        method_permitted("chat.bogus", &[ChatPrompt]),
        GateDecision::UnknownMethod
    );
    assert_eq!(
        method_permitted("models.bogus", &[ModelsInvoke]),
        GateDecision::UnknownMethod
    );
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
        assert_eq!(
            parse_grants(&[grant_wire(*grant).to_string()]),
            vec![*grant]
        );
    }

    // W11: `oauth:contribute` gates no broker verb — the host→ext delegation invokes
    // (`oauth.begin`/`oauth.poll`/`oauth.cancel`) are driven BY koma, never `Call`ed by the
    // extension — so the gate treats them as UnknownMethod even when the grant is held. (The
    // later `oauth.token` READ verb, gated by the separate `oauth:read`, is the ONLY `oauth.*`
    // a `Call` may name; `oauth.begin` stays gate-unknown regardless of routing.)
    assert_eq!(
        method_permitted("oauth.begin", &[Grant::OauthContribute]),
        GateDecision::UnknownMethod
    );
    // The `oauth.` FAMILY now prefix-routes to the broker (so the `oauth.token` verb reaches
    // it), but an unrecognised `oauth.*` verb like `oauth.begin` still resolves to
    // UnknownMethod at the gate above — routing keys on the family prefix; the
    // allow/deny/unknown decision is `method_permitted`'s job (see `is_broker_method`'s doc).
    assert!(is_broker_method("oauth.begin"));
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

// ─── oauth.token broker verb + oauth:read grant (additive) ─────────────────────────────

/// `oauth:read` round-trips both mirror maps in lock-step: `grant_wire` emits the exact
/// wire string, and `parse_grants` maps it back to the variant (plus, defensively, the full
/// `parse_grants(&[grant_wire(g)]) == [g]` round-trip).
#[test]
fn oauth_read_grant_wire_roundtrips() {
    assert_eq!(grant_wire(Grant::OauthRead), "oauth:read");
    assert_eq!(
        parse_grants(&["oauth:read".to_string()]),
        vec![Grant::OauthRead]
    );
    assert_eq!(
        parse_grants(&[grant_wire(Grant::OauthRead).to_string()]),
        vec![Grant::OauthRead]
    );
}

/// The `oauth.token` verb requires `oauth:read`, routes to the broker (the `oauth.` family
/// prefix), and is gated EXACT-MATCH: `oauth:read` allows; the empty set is denied WITH the
/// required grant; an unrelated grant never confers it. `is_granted` (the lattice core)
/// exact-matches it too.
#[test]
fn oauth_token_gate_and_routing() {
    assert_eq!(required_grant("oauth.token"), Some(Grant::OauthRead));
    assert!(
        is_broker_method("oauth.token"),
        "oauth.token must route to the broker"
    );

    assert_eq!(
        method_permitted("oauth.token", &[Grant::OauthRead]),
        GateDecision::Allow
    );
    assert_eq!(
        method_permitted("oauth.token", &[]),
        GateDecision::Deny(Grant::OauthRead)
    );
    // Cross-family isolation: a neighbouring grant never confers oauth:read.
    assert_eq!(
        method_permitted("oauth.token", &[Grant::OauthContribute]),
        GateDecision::Deny(Grant::OauthRead)
    );

    // `is_granted` exact-matches oauth:read: held ⇒ true, absent ⇒ false, unrelated ⇒ false.
    assert!(is_granted(&[Grant::OauthRead], Grant::OauthRead));
    assert!(!is_granted(&[], Grant::OauthRead));
    assert!(!is_granted(&[Grant::OauthContribute], Grant::OauthRead));
}

/// [`build_oauth_token_reply`] is the PURE reply-shaper: `not_connected` when the connection
/// is absent OR the bearer is empty/blank; the success object (with the conn's own
/// email/expires_at) otherwise.
#[test]
fn oauth_token_reply_shape() {
    let not_connected = json!({ "error": "not_connected" });

    // No connection ⇒ not_connected, regardless of bearer.
    assert_eq!(build_oauth_token_reply(None, "x"), not_connected);
    // Connection present but an empty / blank bearer (expired/unrecoverable) ⇒ not_connected.
    assert_eq!(
        build_oauth_token_reply(Some(("a@b.co".to_string(), 123)), ""),
        not_connected
    );
    assert_eq!(
        build_oauth_token_reply(Some(("a@b.co".to_string(), 123)), "   "),
        not_connected
    );
    // Connection + a real bearer ⇒ the success object with the conn's own email/expires_at.
    assert_eq!(
        build_oauth_token_reply(Some(("a@b.co".to_string(), 123)), "jwt"),
        json!({ "access_token": "jwt", "expires_at": 123, "email": "a@b.co" })
    );
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
        sdlc_active_node_id: None,
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
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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
    state.rest.sessions[0].subagents.push(inert_subagent(
        rt.handle(),
        3,
        "general",
        SubAgentStatus::Running,
    ));
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
    let registry = state
        .rest
        .ext_agents
        .entry("test.ext".to_string())
        .or_default();
    let ext_running = registry.insert(sess_uuid.clone(), 3, false);
    let ext_done = registry.insert(sess_uuid.clone(), 4, false);
    let ext_queued = registry.insert(sess_uuid, 5, false);

    // Running → delivered.
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "agentId": ext_running, "message": "focus on the parser" }),
    );
    assert_eq!(
        out.get("sent").and_then(|v| v.as_bool()),
        Some(true),
        "running send must report sent, got {out}"
    );
    assert!(
        out.get("status").is_none(),
        "a running send is not queued, got {out}"
    );

    // Queued → stashed + status:queued, and the message lands in pending_injects.
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "agentId": ext_queued, "message": "also check tests" }),
    );
    assert_eq!(
        out.get("sent").and_then(|v| v.as_bool()),
        Some(true),
        "queued send must report sent, got {out}"
    );
    assert_eq!(
        out.get("status").and_then(|v| v.as_str()),
        Some("queued"),
        "queued send must mark queued, got {out}"
    );
    let pend = state.rest.sessions[0]
        .pending_subagents
        .iter()
        .find(|p| p.id == 5)
        .expect("queued agent still present");
    assert_eq!(
        pend.pending_injects,
        vec!["also check tests".to_string()],
        "queued send must stash the message"
    );

    // Terminal → refused (nothing delivered).
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "agentId": ext_done, "message": "too late" }),
    );
    assert_eq!(
        out.get("error").and_then(|v| v.as_str()),
        Some("agent is terminal"),
        "terminal send must refuse, got {out}"
    );

    // Unknown ext id → unknown agentId.
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "agentId": 9999, "message": "x" }),
    );
    assert!(
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("unknown agentId")),
        "unknown id must error, got {out}"
    );

    // Missing agentId / empty message → their own validation errors.
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "message": "x" }),
    );
    assert!(
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("requires an 'agentId'")),
        "missing agentId must error, got {out}"
    );
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.send",
        json!({ "agentId": ext_running, "message": "   " }),
    );
    assert!(
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("non-empty 'message'")),
        "empty message must error, got {out}"
    );

    // Orchestrate-gated: a read-only grant is denied outright.
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsRead],
        "agents.send",
        json!({ "agentId": ext_running, "message": "x" }),
    );
    assert!(
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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

    state.rest.sessions[0].subagents.push(inert_subagent(
        rt.handle(),
        7,
        "general",
        SubAgentStatus::Running,
    ));
    state.rest.sessions[0].subagents.push(inert_subagent(
        rt.handle(),
        9,
        "researcher",
        SubAgentStatus::Done("the answer is 42".to_string()),
    ));
    let sess_uuid = state.rest.sessions[0].id.clone();
    let registry = state
        .rest
        .ext_agents
        .entry("test.ext".to_string())
        .or_default();
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
        unknown
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("unknown agentId")),
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
    state.rest.sessions[0].subagents.push(inert_subagent(
        rt.handle(),
        3,
        "general",
        SubAgentStatus::Running,
    ));
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
        denied
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
        "kill must require orchestrate, got {denied}"
    );
    assert!(
        matches!(
            state.rest.sessions[0].subagents[0].status,
            SubAgentStatus::Running
        ),
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
        matches!(
            state.rest.sessions[0].subagents[0].status,
            SubAgentStatus::Killed
        ),
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
    state.rest.sessions[0].subagents.push(inert_subagent(
        rt.handle(),
        5,
        "general",
        SubAgentStatus::Running,
    ));
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
        status["agent"],
        json!("general"),
        "must resolve session A's sub-agent, not session B's, got {status}"
    );
    assert_eq!(
        status["status"],
        json!("running"),
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
        matches!(
            state.rest.sessions[0].subagents[0].status,
            SubAgentStatus::Killed
        ),
        "kill must land on session A's sub-agent"
    );
    assert!(
        matches!(
            state.rest.sessions[1].subagents[0].status,
            SubAgentStatus::Done(_)
        ),
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
        denied
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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
        reached
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("requires a 'session'")),
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
        cross
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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
        bogus
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("unknown method")),
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
    assert_ne!(
        ext_id_a, ext_id_b,
        "distinct ext-facing ids even for the same local id"
    );

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
    assert!(
        blank.get("error").is_some(),
        "blank text must be rejected, got {blank}"
    );
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
        full.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("queue full")),
        "the 6th distinct prompt must hit the cap, got {full}"
    );
    assert_eq!(state.rest.sessions[0].pending_ext_prompts.len(), 5);
    assert!(
        state.rest.sessions[0]
            .pending_ext_prompts
            .iter()
            .all(|(id, _)| id == "test.ext"),
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
        toobig
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("16KB")),
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
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("turn budget exhausted")),
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
    assert_eq!(
        out,
        json!({ "queued": 1 }),
        "below budget must be accepted, got {out}"
    );
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
    assert_eq!(
        set(&mut state, "a.ext", "x".repeat(8192)),
        json!({ "ok": true })
    );
    assert_eq!(
        state.rest.ext_context.get("a.ext").map(String::len),
        Some(8192)
    );

    // 8193 bytes → rejected; the prior blob is UNCHANGED.
    let toobig = set(&mut state, "a.ext", "y".repeat(8193));
    assert!(
        toobig
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("8KB")),
        "8193 bytes must be rejected, got {toobig}"
    );
    assert_eq!(
        state.rest.ext_context.get("a.ext").map(String::len),
        Some(8192)
    );

    // A DIFFERENT ext writes its OWN blob — a.ext's is untouched (isolation).
    assert_eq!(
        set(&mut state, "b.ext", "b-data".to_string()),
        json!({ "ok": true })
    );
    assert_eq!(
        state.rest.ext_context.get("a.ext").map(String::len),
        Some(8192)
    );
    assert_eq!(
        state.rest.ext_context.get("b.ext").map(String::as_str),
        Some("b-data")
    );

    // Blank text CLEARS the caller's OWN entry only.
    assert_eq!(
        set(&mut state, "a.ext", "   ".to_string()),
        json!({ "ok": true })
    );
    assert!(!state.rest.ext_context.contains_key("a.ext"));
    assert_eq!(
        state.rest.ext_context.get("b.ext").map(String::as_str),
        Some("b-data")
    );

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
    assert!(!state.rest.ext_context.contains_key("b.ext"));

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
        bad_role
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("unknown role")),
        "an unknown role must error, got {bad_role}"
    );

    // Empty prompt → error.
    let empty = invoke(&mut state, json!({ "prompt": "   " }));
    assert!(
        empty.get("error").is_some(),
        "an empty prompt must error, got {empty}"
    );

    // >32KB prompt → error.
    let big = invoke(&mut state, json!({ "prompt": "x".repeat(32_769) }));
    assert!(
        big.get("error").is_some(),
        "a >32KB prompt must error, got {big}"
    );

    // Valid role + prompt but NO client (the fixture has none) → "no llm client".
    // role=main resolves to koma-free (routable + usable), so validation reaches
    // the client check rather than short-circuiting on the route.
    let no_client = invoke(&mut state, json!({ "role": "main", "prompt": "hi" }));
    assert!(
        no_client
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("no llm client")),
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
    assert_eq!(
        arr[0]["working"],
        json!(true),
        "live row reports the daemon's working flag"
    );
    assert_eq!(arr[1]["id"], json!("dead-1"));
    assert_eq!(arr[1]["live"], json!(false));
    assert_eq!(arr[1]["working"], json!(false), "dead row is never working");
    // Live-but-unregistered appended with a null name.
    assert_eq!(arr[2]["id"], json!("ghost"));
    assert_eq!(
        arr[2]["name"],
        Value::Null,
        "unregistered live session has no name"
    );
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
    assert!(
        matches!(parse_create_workdir(&json!({})), Ok(None)),
        "missing workdir → None"
    );
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
    assert!(
        missing.is_err(),
        "an absolute non-existent path must be rejected"
    );

    let dir = std::env::temp_dir();
    let ok = parse_create_workdir(&json!({ "workdir": dir.to_str().unwrap() }));
    assert!(
        matches!(ok, Ok(Some(_))),
        "an absolute existing dir must pass, got {ok:?}"
    );
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
    assert_eq!(
        state.rest.foreground, 1,
        "a local switch moves the foreground"
    );
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
    assert_eq!(
        state.rest.foreground, 1,
        "a signaled switch must NOT move local foreground"
    );
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
        session_b.subagents.push(inert_subagent(
            rt.handle(),
            i,
            "general",
            SubAgentStatus::Running,
        ));
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
        no_session
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("'session'")),
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
        empty_task
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("non-empty 'task'")),
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
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("no connected oauth account")),
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
        out.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("account-login only")),
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
    assert!(
        config.models.iter().all(|m| m.provider_uuid == "conn-a"),
        "served by the ext conn"
    );
    assert!(
        config.models.iter().all(|m| m.roles.is_empty()),
        "ext models hold no runtime role"
    );
    let fast_uuid = config
        .models
        .iter()
        .find(|m| m.model_id == "fast")
        .unwrap()
        .uuid
        .clone();

    // Re-register "fast" with a NEW name → same uuid returned, name updated, no new entry.
    let out2 = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [{ "id": "fast", "name": "Faster" }] }),
    );
    assert_eq!(out2["registered"], json!(1));
    assert_eq!(
        out2["uuids"][0],
        json!(fast_uuid),
        "dedupe returns the STABLE uuid"
    );
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

    let big: Vec<Value> = (0..101)
        .map(|i| json!({ "id": format!("m{i}"), "name": "n" }))
        .collect();
    let over = apply_models_register(&mut config, "my.ext", &json!({ "models": big }));
    assert!(
        over.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("too many models")),
        "got {over}"
    );
    assert!(
        config.models.is_empty(),
        "an over-cap batch registers nothing"
    );

    // One empty id → the whole batch is rejected (atomic), nothing registered.
    let bad = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [{ "id": "ok", "name": "OK" }, { "id": "", "name": "Bad" }] }),
    );
    assert!(
        bad.get("error").is_some(),
        "an empty id rejects the whole batch, got {bad}"
    );
    assert!(
        config.models.is_empty(),
        "a batch with one bad entry registers NONE (atomic)"
    );
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
    apply_models_register(
        &mut config,
        "ext.b",
        &json!({ "models": [{ "id": "b1", "name": "B1" }] }),
    );
    assert_eq!(config.models.len(), 3);

    // ext A tries to unregister B's model by id → the ownership wall blocks it (0 removed).
    let blocked = apply_models_unregister(&mut config, "ext.a", &json!({ "ids": ["b1"] }));
    assert_eq!(
        blocked["removed"],
        json!(0),
        "ext A cannot touch ext B's entry"
    );
    assert_eq!(config.models.len(), 3);

    // ext A unregister with ids ABSENT → removes ALL of A's (2); B's untouched.
    let all_a = apply_models_unregister(&mut config, "ext.a", &json!({}));
    assert_eq!(all_a["removed"], json!(2));
    assert_eq!(config.models.len(), 1);
    assert_eq!(
        config.models[0].model_id, "b1",
        "only ext B's entry remains"
    );

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
        denied
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("grant denied")),
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
        reached
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("at least one model")),
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
    assert_eq!(
        p.ext_id.as_deref(),
        Some("my.ext"),
        "stamped with the caller's ext id"
    );
    assert_eq!(p.api_type, ApiType::OpenAiCompatible);
    assert_eq!(p.api_key, "sk-1");

    // Re-register the same name → rotate key + endpoint + api_type, SAME uuid, no new entry.
    let out2 = apply_providers_register(
        &mut config,
        "my.ext",
        &json!({ "name": "Gateway", "endpoint": "https://api.gw.test/v2", "api_type": "anthropic", "key": "sk-2" }),
    );
    assert_eq!(
        out2["uuid"].as_str(),
        Some(uuid.as_str()),
        "key-rotation keeps the uuid"
    );
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
    assert!(bad(
        json!({ "name": "  ", "endpoint": "https://x.test", "api_type": "openai", "key": "k" })
    )
    .get("error")
    .is_some());
    assert!(bad(
        json!({ "name": "G", "endpoint": "not a url", "api_type": "openai", "key": "k" })
    )
    .get("error")
    .is_some());
    assert!(
        bad(
            json!({ "name": "G", "endpoint": "ftp://x.test", "api_type": "openai", "key": "k" })
        )
        .get("error")
        .is_some(),
        "non-http scheme rejected"
    );
    assert!(bad(json!({ "name": "G", "endpoint": "https://x.test", "api_type": "codex", "key": "k" })).get("error").is_some(), "koma-free/codex wire types not injectable");
    assert!(bad(
        json!({ "name": "G", "endpoint": "https://x.test", "api_type": "openai", "key": "  " })
    )
    .get("error")
    .is_some());
    assert!(
        config.providers.is_empty(),
        "no invalid provider is ever stored"
    );
}

/// providers.unregister enforces the ownership wall (never another ext's / a native
/// provider), honours the id filter (uuid OR name, case-insensitive), and SWEEPS orphaned
/// models.
#[test]
fn providers_unregister_ownership_wall_and_orphan_sweep() {
    let mut config = AppConfig::default();
    config
        .providers
        .push(ext_key_provider("p-a1", "ext.a", "A1"));
    config
        .providers
        .push(ext_key_provider("p-a2", "ext.a", "A2"));
    config.providers.push(ext_key_provider("p-b", "ext.b", "B"));
    config.providers.push(ProviderConn {
        uuid: "p-native".to_string(),
        name: "native".to_string(),
        ..Default::default()
    });
    for (u, prov) in [
        ("m-a1", "p-a1"),
        ("m-a2", "p-a2"),
        ("m-b", "p-b"),
        ("m-native", "p-native"),
    ] {
        config.models.push(ModelEntry {
            uuid: u.to_string(),
            provider_uuid: prov.to_string(),
            ..Default::default()
        });
    }

    // ext A can never remove B's or native providers (ownership wall).
    let (blocked, _, _) = apply_providers_unregister(
        &mut config,
        "ext.a",
        &json!({ "ids": ["p-b", "p-native"] }),
    );
    assert_eq!(blocked["removed"], json!(0));
    assert_eq!(config.providers.len(), 4);

    // ext A remove by NAME (case-insensitive) → removes A1 + its orphaned model only.
    let (by_name, _, _) =
        apply_providers_unregister(&mut config, "ext.a", &json!({ "ids": ["a1"] }));
    assert_eq!(by_name["removed"], json!(1));
    assert!(config.providers.iter().all(|p| p.uuid != "p-a1"));
    assert!(
        config.models.iter().all(|m| m.provider_uuid != "p-a1"),
        "orphaned model swept"
    );
    assert!(
        config.models.iter().any(|m| m.uuid == "m-a2"),
        "A2's model survives"
    );

    // ext A remove ALL (ids absent) → removes A2 + its model; B + native untouched.
    let (all_a, _, _) = apply_providers_unregister(&mut config, "ext.a", &json!({}));
    assert_eq!(all_a["removed"], json!(1));
    assert!(config
        .providers
        .iter()
        .all(|p| p.ext_id.as_deref() != Some("ext.a")));
    assert!(
        config.providers.iter().any(|p| p.uuid == "p-b"),
        "B untouched"
    );
    assert!(
        config.providers.iter().any(|p| p.uuid == "p-native"),
        "native untouched"
    );
    assert!(
        config.models.iter().any(|m| m.uuid == "m-native"),
        "native model untouched"
    );
}

/// models.register anchors on a KEY-BACKED ext provider when that's the ext's only anchor
/// (no oauth conn needed anymore — the W12b generalization).
#[test]
fn models_register_anchors_on_key_backed_provider() {
    let mut config = AppConfig::default();
    config
        .providers
        .push(ext_key_provider("p-a", "my.ext", "GW"));
    let out = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [{ "id": "m1", "name": "M1" }] }),
    );
    assert_eq!(out["registered"], json!(1));
    assert_eq!(config.models.len(), 1);
    assert_eq!(
        config.models[0].provider_uuid, "p-a",
        "served by the key-backed provider"
    );
}

/// The explicit `{ provider }` param must be caller-owned; an account-login-only conn is
/// rejected; two eligible anchors without a provider param are ambiguous.
#[test]
fn models_register_provider_param_and_ambiguity() {
    let mut config = AppConfig::default();
    config
        .providers
        .push(ext_key_provider("p-a", "my.ext", "GW"));
    config.oauth_conns.push(ext_conn("c-a", "my.ext", true)); // second usable anchor
    config
        .oauth_conns
        .push(ext_conn("c-login", "my.ext", false)); // account-login-only

    // Two eligible anchors + no provider param → ambiguous.
    let ambiguous = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [{ "id": "m", "name": "M" }] }),
    );
    assert!(
        ambiguous
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("multiple providers")),
        "got {ambiguous}"
    );

    // Explicit provider not owned → rejected.
    let not_owned = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "provider": "someone-else", "models": [{ "id": "m", "name": "M" }] }),
    );
    assert!(
        not_owned
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("not owned")),
        "got {not_owned}"
    );

    // Explicit account-login-only conn → rejected.
    let login_only = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "provider": "c-login", "models": [{ "id": "m", "name": "M" }] }),
    );
    assert!(
        login_only
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("account-login only")),
        "got {login_only}"
    );

    // Explicit owned key-backed provider → registers there.
    let ok = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "provider": "p-a", "models": [{ "id": "m", "name": "M" }] }),
    );
    assert_eq!(ok["registered"], json!(1));
    assert_eq!(
        config
            .models
            .iter()
            .find(|m| m.model_id == "m")
            .unwrap()
            .provider_uuid,
        "p-a"
    );
}

/// `default: true` records the ext's preferred model + echoes `defaultUuid`; more than one
/// default in a single call is rejected ATOMICALLY (nothing registered).
#[test]
fn models_register_default_records_preferred() {
    let mut config = AppConfig::default();
    config
        .providers
        .push(ext_key_provider("p-a", "my.ext", "GW"));

    let two = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [
        { "id": "a", "name": "A", "default": true },
        { "id": "b", "name": "B", "default": true },
    ] }),
    );
    assert!(
        two.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("multiple defaults")),
        "got {two}"
    );
    assert!(
        config.models.is_empty(),
        "a multi-default batch registers nothing"
    );

    let out = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [
        { "id": "a", "name": "A" },
        { "id": "b", "name": "B", "default": true },
    ] }),
    );
    assert_eq!(out["registered"], json!(2));
    let du = out["defaultUuid"].as_str().expect("defaultUuid echoed");
    assert_eq!(
        config
            .ext_preferred_models
            .get("my.ext")
            .map(String::as_str),
        Some(du)
    );
    assert_eq!(
        config
            .models
            .iter()
            .find(|m| m.model_id == "b")
            .unwrap()
            .uuid,
        du,
        "the flagged entry's uuid"
    );
}

/// VACUUM-FILL: when Main is unset the preferred model is assigned Main (returns its name); a
/// koma-free placeholder Main also counts as unset (fill wins + steals Main); a REAL user Main
/// is untouched.
#[test]
fn vacuum_fill_only_when_main_unset_or_free() {
    let settings = Settings::default();

    // Case 1: Main unset → fill.
    let mut config = AppConfig::default();
    config
        .providers
        .push(ext_key_provider("p-a", "my.ext", "GW"));
    config.models.push(ModelEntry {
        uuid: "m-pref".to_string(),
        name: "Big".to_string(),
        provider_uuid: "p-a".to_string(),
        ..Default::default()
    });
    assert!(main_is_unset_or_free(&config, &settings));
    assert_eq!(
        try_vacuum_fill_main(&mut config, &settings, "m-pref").as_deref(),
        Some("Big")
    );
    assert!(
        config
            .models
            .iter()
            .find(|m| m.uuid == "m-pref")
            .unwrap()
            .effective_roles()
            .contains(&ModelRole::Main),
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
        c2.models
            .iter()
            .find(|m| m.uuid == "m-user")
            .unwrap()
            .effective_roles()
            .contains(&ModelRole::Main),
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
    assert!(
        main_is_unset_or_free(&c3, &settings),
        "koma-free placeholder counts as unset"
    );
    assert_eq!(
        try_vacuum_fill_main(&mut c3, &settings, "m-pref").as_deref(),
        Some("Big")
    );
    assert!(c3
        .models
        .iter()
        .find(|m| m.uuid == "m-pref")
        .unwrap()
        .effective_roles()
        .contains(&ModelRole::Main));
    assert!(
        !c3.models
            .iter()
            .find(|m| m.uuid == "kf")
            .unwrap()
            .effective_roles()
            .contains(&ModelRole::Main),
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
    assert_eq!(
        try_vacuum_fill_main(&mut c4, &free_settings, "m-pref"),
        None
    );
    assert!(
        c4.models
            .iter()
            .find(|m| m.uuid == "m-global")
            .unwrap()
            .effective_roles()
            .contains(&ModelRole::Main),
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
    let a = apply_models_register(
        &mut config,
        "ext.a",
        &json!({ "models": [{ "id": "am", "name": "Amodel", "default": true }] }),
    );
    let a_uuid = a["defaultUuid"].as_str().unwrap().to_string();
    assert_eq!(
        try_vacuum_fill_main(&mut config, &settings, &a_uuid).as_deref(),
        Some("Amodel")
    );

    // ext B registers a default → Main is now A's (a real provider) → NO fill.
    let b = apply_models_register(
        &mut config,
        "ext.b",
        &json!({ "models": [{ "id": "bm", "name": "Bmodel", "default": true }] }),
    );
    let b_uuid = b["defaultUuid"].as_str().unwrap().to_string();
    assert_eq!(
        try_vacuum_fill_main(&mut config, &settings, &b_uuid),
        None,
        "second ext must not fight"
    );

    // Main still A's; BOTH preferences recorded (B's drives the `recommendedBy` hint).
    let main = config
        .models
        .iter()
        .find(|m| m.effective_roles().contains(&ModelRole::Main))
        .unwrap();
    assert_eq!(main.model_id, "am");
    assert_eq!(
        config.ext_preferred_models.get("ext.a").map(String::as_str),
        Some(a_uuid.as_str())
    );
    assert_eq!(
        config.ext_preferred_models.get("ext.b").map(String::as_str),
        Some(b_uuid.as_str())
    );
}
