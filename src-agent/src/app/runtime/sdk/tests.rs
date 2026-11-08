use super::*;
use crate::ipc::proto::{DaemonFrame, StateDelta, TurnDisposition};

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
    let mut connection = None;
    let r = Request {
        id: "1".into(),
        method: "create".into(),
        params: json!({"cwd":"relative"}),
    };
    assert!(dispatch(r, &mut connection, &events).await.is_err());
    assert!(connection.is_none());
    let r = Request {
        id: "2".into(),
        method: "submit".into(),
        params: json!({"text":"hello"}),
    };
    assert_eq!(
        dispatch(r, &mut connection, &events)
            .await
            .unwrap_err()
            .code,
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
