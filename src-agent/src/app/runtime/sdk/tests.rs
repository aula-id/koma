use super::binding::{refusal, BindingState, Family};
use super::coding::turn_result;
use super::*;
use crate::ipc::proto::{DaemonEvent as E, DaemonFrame, StateDelta, TurnDisposition};

#[tokio::test]
async fn bounded_fragmented_unicode_lines() {
    let bytes = "{\"text\":\"é雪\"}\r\n{}\n".as_bytes();
    let mut reader = tokio::io::BufReader::with_capacity(1, bytes);
    let a = line(&mut reader).await.unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&a).unwrap()["text"], "é雪");
    assert_eq!(line(&mut reader).await.unwrap().unwrap(), b"{}");
    assert!(line(&mut reader).await.unwrap().is_none());
    assert!(line(&mut &b"partial"[..]).await.is_err());
    assert!(line(&mut &b"\n"[..]).await.is_err());
    let huge = vec![b'x'; MAX_LINE + 1];
    assert!(line(&mut huge.as_slice()).await.is_err());
}

#[test]
fn owned_attachment_validation_and_defaults() {
    assert_eq!(Policy::default().plan, Plan::Forbid);
    assert_eq!(Policy::default().approval, Approval::Reject);
    let (text, media) = attachments(Input {
        text: "hello".into(),
        attachments: vec![],
    })
    .unwrap();
    assert_eq!(text, "hello");
    assert!(media.is_empty());
    assert!(attachments(Input {
        text: " ".into(),
        attachments: vec![]
    })
    .is_err());
    assert!(attachments(Input {
        text: "hello".into(),
        attachments: vec![Attachment::ImageFile {
            path: "relative.png".into()
        }]
    })
    .is_err());
}

#[test]
fn replacement_partial_failure_is_not_erased() {
    let value = turn_result("session", E::InterruptAndSubmitResult {
        request_id: "r".into(),
        disposition: TurnDisposition::Rejected,
        interrupted_turn_id: Some("old".into()),
        turn_id: None,
        reason: Some("startup failed".into()),
    })
    .expect("turn result");
    assert_eq!(value["interrupted_turn_id"], "old");
    assert_eq!(value["disposition"], "rejected");
    assert!(value["turn_id"].is_null());
}

fn state() -> State {
    State {
        revision: 1,
        resync: 0,
        external_approvals: false,
        pending_approval: None,
        gate: None,
        session_id: "session".into(),
        cwd: "/work".into(),
        model_id: "model".into(),
        policy: Policy::default(),
        turn: None,
        working: false,
        awaiting_approval: false,
        approval_reason: None,
        pending_count: 0,
        messages: vec![],
        streaming: Some("é".into()),
    }
}
#[test]
fn output_proven_suffix_and_rewrite() {
    let mut previous = Some(state());
    let mut seq = 0;
    let mut next = state();
    next.streaming = Some("é雪".into());
    next.revision = 2;
    let frames = event_frames(&mut previous, next, &mut seq);
    assert_eq!(frames[0]["event"], "output_append");
    assert_eq!(frames[0]["offset"], 2);
    assert_eq!(frames[0]["text"], "雪");
    let mut rewritten = state();
    rewritten.revision = 3;
    rewritten.resync = 1;
    let frames = event_frames(&mut previous, rewritten, &mut seq);
    assert_eq!(frames[0]["event"], "state");
    assert_eq!(frames[0]["resynced"], true);
}
#[test]
fn gap_suppresses_dependent_deltas() {
    let mut r = reconcile::Reconcile::new("session".into());
    let event = E::Delta(StateDelta::TokenAppended {
        session_id: "session".into(),
        text: "x".into(),
    });
    assert!(!r.apply(&DaemonFrame {
        seq: 1,
        event: event.clone()
    }));
    assert!(r.apply(&DaemonFrame {
        seq: 3,
        event: event.clone()
    }));
    assert!(r.syncing);
    assert!(r.state().is_none());
    assert!(!r.apply(&DaemonFrame { seq: 4, event }));
}
#[tokio::test]
async fn invalid_create_cannot_bind_or_submit() {
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    let r = Request {
        id: "1".into(),
        method: "create".into(),
        params: json!({"cwd":"relative"}),
    };
    assert!(dispatch(r, &mut adapter).await.is_err());
    assert!(matches!(adapter.binding, Binding::Unbound));
    let r = Request {
        id: "2".into(),
        method: "submit".into(),
        params: json!({"text":"hello"}),
    };
    assert_eq!(
        dispatch(r, &mut adapter).await.unwrap_err().code,
        "invalid_state"
    );
}

fn snapshot(seq: u64) -> DaemonFrame {
    let value = json!({"foreground_id":"session","sessions":[{
     "id":"session","name":"test","cwd":"/work","messages":[],"streaming":"é","stream_reasoning":"PRIVATE REASONING",
     "tokens_in":0,"tokens_out":0,"tokens_cached":0,"cost":0.0,"waiting":true,"awaiting_approval":false,
     "approval_reason":null,"pending_tool_calls":[],"tool_idx":0,"working":true,"finished_unseen":false,"subagents":[],"pending_subagents":[],"resolved_model_id":"model"
    }],"global":{
     "input":"PRIVATE COMPOSER","cursor":0,"scroll":0,"follow":true,"status":"","work_elapsed_ms":null,"theme":"dark","accent":"blue","palette":"default","mode":"Chat","toast":null,"models_cache":null,"models_cache_endpoint":null,"models_cache_failed":null,"agent_viewer":null,"agent_viewer_scroll":0,"agent_viewer_follow":true,"subagents_open":false,"subagent_sel":0,"palette_sel":0,"pending_attachments":[],"file_palette":null,"agent_mode":"auto","latest_version":null
    }});
    DaemonFrame {
        seq,
        event: E::Snapshot(Box::new(serde_json::from_value(value).unwrap())),
    }
}
#[test]
fn selected_projection_gap_recovery_and_private_fields() {
    let mut r = reconcile::Reconcile::new("session".into());
    r.policy = Some(Policy::default());
    r.apply(&snapshot(1));
    let first = r.state().unwrap();
    assert_eq!(first.revision, 1);
    r.apply(&DaemonFrame {
        seq: 2,
        event: E::Delta(StateDelta::TokenAppended {
            session_id: "other".into(),
            text: "NO".into(),
        }),
    });
    assert_eq!(r.state().unwrap(), first);
    r.apply(&DaemonFrame {
        seq: 3,
        event: E::Delta(StateDelta::TokenAppended {
            session_id: "session".into(),
            text: "雪".into(),
        }),
    });
    assert_eq!(r.state().unwrap().streaming.as_deref(), Some("é雪"));
    assert!(r.apply(&DaemonFrame {
        seq: 5,
        event: E::Ack
    }));
    assert!(r.state().is_none());
    r.apply(&snapshot(6));
    let recovered = r.state().unwrap();
    assert_eq!(recovered.resync, 1);
    assert_eq!(recovered.streaming.as_deref(), Some("é"));
    let wire = serde_json::to_string(&recovered).unwrap();
    assert!(!wire.contains("PRIVATE"));
    assert!(!wire.contains("resync"));
}
#[test]
fn policy_readback_does_not_splice_new_turn_into_cached_snapshot() {
    use crate::ipc::proto::turn::{
        ApprovalPolicy, PlanPolicy, SessionPolicy, TurnRecord, TurnState,
    };
    use crate::ipc::proto::RunState;

    for (id, status) in [("old", TurnState::Finished), ("new", TurnState::Running)] {
        let mut r = reconcile::Reconcile::new("session".into());
        r.policy = Some(Policy::default());
        let mut cached = snapshot(1);
        if let E::Snapshot(s) = &mut cached.event {
            s.sessions[0].turn = Some(TurnRecord {
                id: "old".into(),
                state: TurnState::Running,
                error: None,
            });
        }
        r.apply(&cached);
        let before = r.state().unwrap();
        let newer = TurnRecord {
            id: id.into(),
            state: status,
            error: None,
        };
        r.readback(&RunState {
            session_id: "session".into(),
            policy: Some(SessionPolicy {
                plan: PlanPolicy::Forbid,
                approval: ApprovalPolicy::Reject,
            }),
            turn: Some(newer.clone()),
            ..RunState::default()
        })
        .unwrap();
        assert_eq!(r.state().unwrap(), before);

        let mut refreshed = snapshot(2);
        if let E::Snapshot(s) = &mut refreshed.event {
            s.sessions[0].turn = Some(newer);
            s.sessions[0].streaming = None;
            s.sessions[0].working = status == TurnState::Running;
        }
        r.apply(&refreshed);
        let after = r.state().unwrap();
        assert_eq!(after.turn.unwrap().id, id);
        assert!(after.streaming.is_none());
        assert_eq!(after.working, status == TurnState::Running);
    }
}

#[test]
fn pending_call_approval_excludes_plan_and_mission() {
    let mut r = reconcile::Reconcile::new("session".into());
    r.policy = Some(Policy::default());
    r.external_approvals = true;
    r.apply(&snapshot(1));
    for (name, kind) in [
        ("bash", "tool"),
        ("plan_ready", "plan"),
        ("mission_ready", "mission"),
    ] {
        let s = r.session.as_mut().unwrap();
        s.awaiting_approval = true;
        s.pending_tool_calls=serde_json::from_value(json!([{"id":"actual-call","type":"function","function":{"name":name,"arguments":"{\"secret\":\"PRIVATE\"}"}}])).unwrap();
        let state = r.state().unwrap();
        assert_eq!(state.gate.as_ref().unwrap().kind, kind);
        assert_eq!(state.gate.as_ref().unwrap().supported, kind == "tool");
        assert_eq!(state.pending_approval.is_some(), kind == "tool");
        assert!(!serde_json::to_string(&state).unwrap().contains("secret"));
    }
}

fn role_params(role: &str) -> Value {
    json!({
        "providers": [{
            "name": "local",
            "endpoint": "http://127.0.0.1:1/v1",
            "api_type": "openai",
            "api_key": "sk-test-role-key",
        }],
        "models": {
            "main": {"provider": "local", "model_id": "main-model"},
            role: {"provider": "local", "model_id": "role-model"},
        },
    })
}

const CYBER_KEYS: [&str; 8] = [
    "cyber-red",
    "cyber_red",
    "cyber-blue",
    "cyber_blue",
    "cyber-research",
    "cyber_research",
    "cyber-researcher",
    "cyber_researcher",
];

#[tokio::test]
async fn configure_rejects_cyber_roles_before_any_catalogue_access() {
    // The rejection comes from the pre-parse, so the unbound path never loads
    // or saves the real catalogue here.
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    for key in CYBER_KEYS {
        let r = Request {
            id: key.into(),
            method: "configure".into(),
            params: role_params(key),
        };
        let error = dispatch(r, &mut adapter).await.unwrap_err();
        assert_eq!(error.code, "invalid_request");
        assert_eq!(error.message, format!("unknown model role '{key}'"));
        assert_eq!(error.delivery, "not_sent");
        assert!(!error.message.contains("sk-test-role-key"));
        assert!(matches!(adapter.binding, Binding::Unbound));
    }
}

#[test]
fn configure_check_and_apply_reject_cyber_roles() {
    // `configure::check` is the pre-parse `configure_call` runs on every call;
    // `configure::apply` is what a daemon on this binary runs.
    for key in CYBER_KEYS {
        let message = format!("unknown model role '{key}'");
        assert_eq!(configure::check(&role_params(key)).unwrap_err(), message);
        let mut config = crate::model::app_config::AppConfig::default();
        assert_eq!(
            configure::apply(&mut config, &role_params(key), false).unwrap_err(),
            message
        );
        assert!(config.models.is_empty());
        assert!(config.providers.is_empty());
    }
}

#[test]
fn configure_public_roles_still_pass() {
    for role in ["main", "awareness", "safeguard", "compactor", "planner"] {
        let params = role_params(role);
        configure::check(&params).unwrap();
        let mut config = crate::model::app_config::AppConfig::default();
        let applied = configure::apply(&mut config, &params, true).unwrap();
        // For `main` the role entry overrides the shared `main` entry.
        assert_eq!(applied.reply["models"][role]["model_id"], "role-model");
        assert_eq!(applied.reply["models"]["main"]["provider"], "local");
        assert!(!applied.reply.to_string().contains("sk-test-role-key"));
    }
}

const V1_METHODS: [&str; 10] = [
    "create",
    "resume",
    "state",
    "submit",
    "enqueue",
    "interrupt",
    "interrupt_and_submit",
    "approve",
    "configure",
    "detach",
];

/// The config methods and `sessions.info`, in route-table order.
const CONFIG_METHODS: [&str; 10] = [
    "config.get",
    "config.providers.delete",
    "config.models.delete",
    "config.roles.unassign",
    "config.tool_policy.set",
    "config.effort.get",
    "config.effort.set",
    "config.limits.get",
    "config.limits.set",
    "sessions.info",
];

/// The session, usage, skills and room methods, in route-table order.
const FAMILY_METHODS: [&str; 31] = [
    "sessions.list",
    "sessions.rename",
    "sessions.delete",
    "sessions.stop",
    "sessions.transcript",
    "usage.query",
    "skills.list",
    "skills.get",
    "skills.read_file",
    "skills.install",
    "skills.remove",
    "skills.create",
    "skills.update",
    "room.list",
    "room.open",
    "room.attach",
    "room.close",
    "room.send",
    "room.interrupt",
    "room.control",
    "room.status",
    "room.history",
    "room.shelf.list",
    "room.item",
    "room.trajectory",
    "room.reports",
    "room.spend",
    "room.team.get",
    "room.models.set",
    "room.avatars.upsert",
    "room.avatars.remove",
];

/// Room methods allowed in ROOM(live) and ROOM(closed).
const ROOM_SCOPED: [&str; 8] = [
    "room.close",
    "room.status",
    "room.history",
    "room.shelf.list",
    "room.item",
    "room.trajectory",
    "room.reports",
    "room.spend",
];

fn all_methods() -> Vec<&'static str> {
    V1_METHODS
        .iter()
        .chain(CONFIG_METHODS.iter())
        .chain(FAMILY_METHODS.iter())
        .copied()
        .collect()
}

/// What a call of `method` in `state` must give: `None` when the route lets
/// it through, otherwise the error code, message, reason and allowed list.
fn expected_gate(
    method: &str,
    state: BindingState,
) -> Option<(&'static str, String, &'static str, Vec<&'static str>)> {
    let unbound = state == BindingState::Unbound;
    match method {
        "create" | "resume" if unbound => None,
        "create" | "resume" => Some((
            "invalid_state",
            "session already bound".into(),
            "already_bound",
            vec!["unbound"],
        )),
        "room.open" | "room.attach"
            if unbound || state == BindingState::RoomClosed =>
        {
            None
        }
        "room.open" | "room.attach" => Some((
            "invalid_state",
            "session already bound".into(),
            "already_bound",
            vec!["unbound", "room_closed"],
        )),
        "room.send" | "room.interrupt" | "room.control" => match state {
            BindingState::RoomLive => None,
            BindingState::RoomClosed => Some((
                "invalid_state",
                "the room connection is closed".into(),
                "room_closed",
                vec!["room_live"],
            )),
            _ => Some((
                "invalid_state",
                format!(
                    "{method} requires binding room_live; this adapter is {}",
                    state.name()
                ),
                "wrong_binding",
                vec!["room_live"],
            )),
        },
        _ if ROOM_SCOPED.contains(&method) => match state {
            BindingState::RoomLive | BindingState::RoomClosed => None,
            _ => Some((
                "invalid_state",
                format!(
                    "{method} requires binding room_live or room_closed; this adapter is {}",
                    state.name()
                ),
                "wrong_binding",
                vec!["room_live", "room_closed"],
            )),
        },
        "configure" | "detach" => None,
        // Host config methods run in every binding; session scope is refused
        // by the handler.
        "config.get"
        | "config.providers.delete"
        | "config.models.delete"
        | "config.roles.unassign"
        | "config.tool_policy.set" => None,
        // Host families run in every binding; team methods refuse
        // `target:"room"` outside a room binding in the handler.
        _ if FAMILY_METHODS.contains(&method) => None,
        _ if state == BindingState::Coding => None,
        _ if !V1_METHODS.contains(&method) => Some((
            "invalid_state",
            format!(
                "{method} requires binding coding; this adapter is {}",
                state.name()
            ),
            "wrong_binding",
            vec!["coding"],
        )),
        _ if unbound => Some((
            "invalid_state",
            "no session bound".into(),
            "wrong_binding",
            vec!["coding"],
        )),
        _ => Some((
            "invalid_state",
            format!(
                "{method} requires binding coding; this adapter is {}",
                state.name()
            ),
            "wrong_binding",
            vec!["coding"],
        )),
    }
}

#[test]
fn binding_matrix() {
    let routed: Vec<&str> = methods::ROUTES.iter().map(|r| r.method).collect();
    assert_eq!(routed, all_methods());
    for route in methods::ROUTES {
        for state in BindingState::ALL {
            let got = route.check(state);
            match expected_gate(route.method, state) {
                None => assert!(got.is_ok(), "{} in {}", route.method, state.name()),
                Some((code, message, reason, allowed)) => {
                    let error = got.unwrap_err();
                    let context = format!("{} in {}", route.method, state.name());
                    assert_eq!(error.code, code, "{context}");
                    assert_eq!(error.message, message, "{context}");
                    assert_eq!(error.delivery, "not_sent", "{context}");
                    let extra = error.extra().expect("binding keys");
                    assert_eq!(extra.reason, Some(reason), "{context}");
                    assert_eq!(extra.binding, Some(state.name()), "{context}");
                    assert_eq!(extra.allowed.as_deref(), Some(&allowed[..]), "{context}");
                    assert!(!error.ends_process(route.legacy()), "{context}");
                }
            }
        }
    }
}

#[test]
fn refusals_for_families_without_methods_yet() {
    let e2 = refusal(
        "config.effort.get",
        Family::Coding,
        &[BindingState::Coding],
        BindingState::Unbound,
    );
    assert_eq!(
        serde_json::to_value(&e2).unwrap(),
        json!({"code":"invalid_state","delivery":"not_sent","message":"config.effort.get requires binding coding; this adapter is unbound","reason":"wrong_binding","binding":"unbound","allowed":["coding"]})
    );
    let reads = &[BindingState::RoomLive, BindingState::RoomClosed];
    let status = refusal("room.status", Family::Room, reads, BindingState::Coding);
    assert_eq!(
        status.message,
        "room.status requires binding room_live or room_closed; this adapter is coding"
    );
    assert_eq!(
        status.extra().unwrap().allowed.as_deref(),
        Some(&["room_live", "room_closed"][..])
    );
    let e4 = refusal(
        "room.send",
        Family::RoomSocket,
        &[BindingState::RoomLive],
        BindingState::RoomClosed,
    );
    assert_eq!(e4.message, "the room connection is closed");
    assert_eq!(e4.extra().unwrap().reason, Some("room_closed"));
    let e2_socket = refusal(
        "room.send",
        Family::RoomSocket,
        &[BindingState::RoomLive],
        BindingState::Unbound,
    );
    assert_eq!(e2_socket.extra().unwrap().reason, Some("wrong_binding"));
    let rebind = refusal(
        "room.open",
        Family::Binder,
        &[BindingState::Unbound, BindingState::RoomClosed],
        BindingState::RoomLive,
    );
    assert_eq!(rebind.message, "session already bound");
    assert_eq!(rebind.extra().unwrap().reason, Some("already_bound"));
    let host = refusal("sessions.list", Family::Host, &[], BindingState::Coding);
    assert_eq!(host.extra().unwrap().reason, Some("wrong_binding"));
}

#[tokio::test]
async fn unknown_method_stays_unsupported_without_new_keys() {
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    for method in ["sessions.nope", "nope", ""] {
        let r = Request {
            id: method.into(),
            method: method.into(),
            params: json!({}),
        };
        let error = dispatch(r, &mut adapter).await.unwrap_err();
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"unsupported_method","message":"unsupported method","delivery":"not_sent"}"#
        );
    }
}

#[test]
fn existing_error_frames_are_byte_identical() {
    let cases = [
        (
            RpcError::new("invalid_request", "duplicate request id"),
            r#"{"code":"invalid_request","message":"duplicate request id","delivery":"not_sent"}"#,
        ),
        (
            RpcError::unknown("daemon did not apply policy"),
            r#"{"code":"unavailable","message":"daemon did not apply policy","delivery":"unknown"}"#,
        ),
        (
            RpcError::unknown("x").in_process(),
            r#"{"code":"unavailable","message":"x","delivery":"unknown"}"#,
        ),
        (
            RpcError::new("unavailable", "state exceeds frame limit"),
            r#"{"code":"unavailable","message":"state exceeds frame limit","delivery":"not_sent"}"#,
        ),
    ];
    for (error, wire) in cases {
        assert_eq!(serde_json::to_string(&error).unwrap(), wire);
    }
    assert_eq!(
        serde_json::to_string(&oversized(true)).unwrap(),
        r#"{"code":"unavailable","message":"state exceeds frame limit","delivery":"not_sent"}"#
    );
    assert_eq!(oversized(false).extra().unwrap().reason, Some("frame_limit"));
    let full = RpcError::reason(Reason::Unverified, "x")
        .in_process()
        .with_detail(json!({"path":"config.json"}));
    assert_eq!(
        serde_json::to_string(&full).unwrap(),
        r#"{"code":"unavailable","message":"x","delivery":"unknown","reason":"unverified","written":true,"detail":{"path":"config.json"}}"#
    );
}

#[test]
fn a_reason_fixes_the_code_and_delivery() {
    let cases = [
        (Reason::Stale, "invalid_state", "not_sent", None),
        (Reason::OutOfRange, "invalid_request", "not_sent", None),
        (Reason::NotFound, "not_found", "not_sent", None),
        (Reason::LockTimeout, "unavailable", "not_sent", None),
        (Reason::Unverified, "unavailable", "unknown", Some(true)),
        (Reason::PartialApply, "unavailable", "unknown", None),
    ];
    for (reason, code, delivery, written) in cases {
        let error = RpcError::reason(reason, "x");
        assert_eq!((error.code, error.delivery), (code, delivery), "{reason:?}");
        let extra = error.extra().unwrap();
        assert_eq!(extra.reason, Some(reason.name()));
        assert_eq!(extra.written, written, "{reason:?}");
        // An unknown outcome ends the process unless it concerns in-process work.
        assert_eq!(error.ends_process(false), reason.unknown());
        assert!(!RpcError::reason(reason, "x").in_process().ends_process(false));
    }
}

#[test]
fn only_coding_unknowns_end_the_process() {
    assert!(RpcError::unknown("x").ends_process(true));
    assert!(RpcError::unknown("x").ends_process(false));
    assert!(RpcError::unknown("x").in_process().ends_process(true));
    assert!(!RpcError::unknown("x").in_process().ends_process(false));
    assert!(!RpcError::new("unavailable", "x").ends_process(true));
}

#[tokio::test]
async fn unbound_detach_errors_and_ends_the_loop() {
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    let mut seen = std::collections::HashSet::new();
    let r = Request {
        id: "d".into(),
        method: "detach".into(),
        params: json!({}),
    };
    let (frame, end) = respond(r, &mut seen, &mut adapter).await.unwrap();
    assert!(end);
    assert_eq!(
        serde_json::to_string(&frame).unwrap(),
        r#"{"error":{"code":"invalid_state","delivery":"not_sent","message":"no session bound"},"id":"d","type":"response"}"#
    );
    // A v1 call without a session does not end the loop, and duplicates are refused.
    let r = Request {
        id: "s".into(),
        method: "state".into(),
        params: json!({}),
    };
    let (frame, end) = respond(r, &mut seen, &mut adapter).await.unwrap();
    assert!(!end);
    assert_eq!(frame["error"]["message"], "no session bound");
    assert_eq!(frame["error"]["reason"], "wrong_binding");
    let r = Request {
        id: "s".into(),
        method: "state".into(),
        params: json!({}),
    };
    let (frame, end) = respond(r, &mut seen, &mut adapter).await.unwrap();
    assert!(!end);
    assert_eq!(frame["error"]["message"], "duplicate request id");
    assert!(frame["error"].get("reason").is_none());
}

#[tokio::test]
async fn blocking_host_work_runs_unbound_and_after_a_lost_connection() {
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    assert_eq!(adapter.blocking(|| Ok(7)).await.unwrap(), 7);
    let error = adapter
        .blocking(|| Err::<(), _>(RpcError::unknown("disk").in_process()))
        .await
        .unwrap_err();
    assert!(!error.ends_process(false));
    adapter.lost = Some(anyhow::anyhow!("gone"));
    assert_eq!(adapter.blocking(|| Ok("done")).await.unwrap(), "done");
    assert!(adapter.lost.is_some());
}

/// The `hello` definition of the published v1 schema.
const V1_HELLO_SCHEMA: &str = r#"{"type":"object","properties":{"type":{"const":"hello"},"protocol":{"const":1},"binary":{"type":"object","properties":{"path":{"type":"string","minLength":1},"version":{"type":"string","minLength":1}},"required":["path","version"]},"diagnostics":{"type":"array","items":{"type":"string"}},"capabilities":{"type":"array","uniqueItems":true,"items":{"enum":["create","resume","state","submit","enqueue","interrupt","interrupt_and_submit","approve","configure","detach"]}}},"required":["type","protocol","binary","diagnostics","capabilities"]}"#;

/// The subset of JSON Schema that the protocol schema's `hello` uses.
fn validate(schema: &Value, value: &Value, defs: &Value, at: &str) -> Result<(), String> {
    let fail = |what: &str| Err(format!("{at}: {what}"));
    if let Some(target) = schema.get("$ref").and_then(Value::as_str) {
        let name = target.trim_start_matches("#/$defs/");
        return validate(&defs[name], value, defs, at);
    }
    if let Some(any) = schema.get("anyOf").and_then(Value::as_array) {
        if !any.iter().any(|s| validate(s, value, defs, at).is_ok()) {
            return fail("anyOf");
        }
    }
    if let Some(one) = schema.get("oneOf").and_then(Value::as_array) {
        if one.iter().filter(|s| validate(s, value, defs, at).is_ok()).count() != 1 {
            return fail("oneOf");
        }
    }
    if let Some(c) = schema.get("const") {
        if c != value {
            return fail("const");
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(value) {
            return fail("enum");
        }
    }
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            other => other.as_str().into_iter().collect(),
        };
        let ok = types.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_u64() || value.is_i64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !ok {
            return fail("type");
        }
    }
    if let Some(n) = schema.get("minLength").and_then(Value::as_u64) {
        if value.as_str().is_some_and(|s| (s.chars().count() as u64) < n) {
            return fail("minLength");
        }
    }
    if let Some(n) = schema.get("minimum").and_then(Value::as_f64) {
        if value.as_f64().is_some_and(|v| v < n) {
            return fail("minimum");
        }
    }
    if let Some(n) = schema.get("maximum").and_then(Value::as_f64) {
        if value.as_f64().is_some_and(|v| v > n) {
            return fail("maximum");
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(n) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() as u64 > n {
                return fail("maxItems");
            }
        }
        if let Some(n) = schema.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < n {
                return fail("minItems");
            }
        }
        if schema.get("uniqueItems") == Some(&json!(true)) {
            for (i, item) in items.iter().enumerate() {
                if items[..i].contains(item) {
                    return fail("uniqueItems");
                }
            }
        }
        if let Some(each) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                validate(each, item, defs, &format!("{at}[{i}]"))?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        for key in schema["required"].as_array().into_iter().flatten() {
            let key = key.as_str().unwrap_or_default();
            if !object.contains_key(key) {
                return fail(&format!("missing {key}"));
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (key, sub) in props {
                if let Some(v) = object.get(key) {
                    validate(sub, v, defs, &format!("{at}.{key}"))?;
                }
            }
        }
    }
    Ok(())
}

#[test]
fn golden_hello() {
    let hello = protocol::hello("/opt/koma/bin/koma");
    let v1: Value = serde_json::from_str(V1_HELLO_SCHEMA).unwrap();
    validate(&v1, &hello, &json!({}), "hello").unwrap();
    let published = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src-core-sdk/protocol/schema.json"
    ))
    .unwrap();
    let published: Value = serde_json::from_str(&published).unwrap();
    validate(&published["$defs"]["hello"], &hello, &published["$defs"], "hello").unwrap();

    assert_eq!(hello["capabilities"], json!(V1_METHODS));
    assert_eq!(hello["methods"], json!(all_methods()));
    let mut golden = hello.clone();
    golden["binary"]["version"] = json!("<version>");
    assert_eq!(
        golden,
        json!({
            "type": "hello",
            "protocol": 1,
            "binary": {"path": "/opt/koma/bin/koma", "version": "<version>"},
            "diagnostics": [],
            "capabilities": V1_METHODS,
            "methods": all_methods(),
            "features": [
                "errors.reason",
                "state.session_info",
                "config.lock",
                "create.name",
                "bindings.room",
                "events.room",
                "room.template"
            ],
            "events": [
                "state",
                "output_append",
                "turn",
                "room.line",
                "room.delta",
                "room.status",
                "room.notice",
                "room.closed",
                "room.gap"
            ],
            "limits": {
                "frame_bytes": 41943040,
                "request_ids": 65536,
                "text_bytes": 2097152,
                "attachments": {"count": 16, "pasted_bytes": 2097152, "total_bytes": 33554432},
                "list": {
                    "sessions": 500,
                    "rooms": 500,
                    "skills": 1000,
                    "usage_groups": 500,
                    "usage_buckets": 1000,
                    "shelf_rows": 500,
                    "reports": 100,
                    "history_lines": 200,
                    "trajectory_goals": 200,
                    "trajectory_nodes": 2000,
                    "trajectory_edges": 8000
                },
                "room": {
                    "ring_lines": 8000,
                    "ring_bytes": 67108864,
                    "page_bytes": 8388608,
                    "event_text_bytes": 1048576,
                    "status_text_bytes": 262144,
                    "item_bytes": 16777216,
                    "tail_lines": 200
                },
                "skills": {
                    "inline_zip_bytes": 29360128,
                    "zip_bytes": 67108864,
                    "entry_bytes": 2097152,
                    "companion_bytes": 8388608,
                    "files": 1000,
                    "depth": 16,
                    "name_bytes": 64
                },
                "config": {"lock_wait_ms": 3000},
                "time": {
                    "ipc_ms": 15000,
                    "room_start_ms": 30000,
                    "usage_query_ms": 2000,
                    "session_stop_ms": 8000
                }
            }
        })
    );
}

#[test]
fn advertised_limits_match_enforced_bounds() {
    let text = |n: usize| Input {
        text: "x".repeat(n),
        attachments: vec![],
    };
    assert!(attachments(text(TEXT_BYTES)).is_ok());
    assert!(attachments(text(TEXT_BYTES + 1)).is_err());
    let pasted = |count: usize, bytes: usize| Input {
        text: "x".into(),
        attachments: (0..count)
            .map(|_| Attachment::PastedText {
                text: "y".repeat(bytes),
                name: None,
            })
            .collect(),
    };
    assert!(attachments(pasted(ATTACHMENT_COUNT, 1)).is_ok());
    assert!(attachments(pasted(ATTACHMENT_COUNT + 1, 1)).is_err());
    assert!(attachments(pasted(1, TEXT_BYTES + 1)).is_err());
    let per = ATTACHMENT_BYTES as usize / ATTACHMENT_COUNT;
    assert!(attachments(pasted(ATTACHMENT_COUNT, per)).is_ok());
    assert!(attachments(pasted(ATTACHMENT_COUNT, per + 1)).is_err());
}
