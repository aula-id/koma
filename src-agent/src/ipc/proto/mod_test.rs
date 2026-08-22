#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// The cross-daemon spawn request (W7 `sessions.spawn_into`) survives a
/// serde round-trip intact — it crosses the unix socket between two
/// session-daemons, so its wire shape must be stable (all four fields,
/// including the `Option` absences).
#[test]
fn spawn_agent_serde_roundtrip() {
    let full = ClientRequest::SpawnAgent {
        agent: Some("researcher".into()),
        task: "summarise the diff".into(),
        model: Some("gpt-5".into()),
        effort: Some("high".into()),
    };
    let bytes = serde_json::to_vec(&full).expect("serialise SpawnAgent");
    let back: ClientRequest = serde_json::from_slice(&bytes).expect("deserialise SpawnAgent");
    assert_eq!(back, full);

    // Optional fields absent (the common `sessions.spawn_into { session, task }` shape).
    let minimal = ClientRequest::SpawnAgent {
        agent: None,
        task: "do the thing".into(),
        model: None,
        effort: None,
    };
    let back2: ClientRequest =
        serde_json::from_slice(&serde_json::to_vec(&minimal).unwrap()).unwrap();
    assert_eq!(back2, minimal);
}

/// The attach-hand-off signal (W7 `sessions.switch` to a non-local session)
/// round-trips — it is broadcast to attached clients, so its wire shape must
/// hold.
#[test]
fn attach_session_serde_roundtrip() {
    let ev = DaemonEvent::AttachSession {
        session_id: "abc-123".into(),
    };
    let back: DaemonEvent = serde_json::from_slice(&serde_json::to_vec(&ev).unwrap()).unwrap();
    assert_eq!(back, ev);
}

/// The panel→daemon request (W8 panel bridge) round-trips intact, INCLUDING the
/// `req_id` present/absent forms and an arbitrary JSON `payload` — it crosses the
/// unix socket from the GUI host to the daemon, so its wire shape must be stable.
#[test]
fn ext_panel_msg_serde_roundtrip() {
    let with_id = ClientRequest::ExtPanelMsg {
        ext_id: "run.koma.example".into(),
        panel_id: "sidebar".into(),
        req_id: Some("r-7".into()),
        payload: serde_json::json!({ "action": "refresh", "n": 3 }),
    };
    let back: ClientRequest =
        serde_json::from_slice(&serde_json::to_vec(&with_id).unwrap()).unwrap();
    assert_eq!(back, with_id);

    // Fire-and-forget shape (no correlation id).
    let no_id = ClientRequest::ExtPanelMsg {
        ext_id: "run.koma.example".into(),
        panel_id: "sidebar".into(),
        req_id: None,
        payload: serde_json::Value::Null,
    };
    let back2: ClientRequest =
        serde_json::from_slice(&serde_json::to_vec(&no_id).unwrap()).unwrap();
    assert_eq!(back2, no_id);
}

/// The panel-reply event (W8) round-trips: both the ok+payload and the
/// error (`ok:false`, no payload) shapes cross the wire to the requesting client.
#[test]
fn ext_panel_reply_serde_roundtrip() {
    let ok = DaemonEvent::ExtPanelReply {
        ext_id: "run.koma.example".into(),
        panel_id: "sidebar".into(),
        req_id: Some("r-7".into()),
        ok: true,
        payload: Some(serde_json::json!({ "rows": [1, 2, 3] })),
        error: None,
    };
    let back: DaemonEvent = serde_json::from_slice(&serde_json::to_vec(&ok).unwrap()).unwrap();
    assert_eq!(back, ok);

    let err = DaemonEvent::ExtPanelReply {
        ext_id: "run.koma.example".into(),
        panel_id: "sidebar".into(),
        req_id: None,
        ok: false,
        payload: None,
        error: Some("extension not available".into()),
    };
    let back2: DaemonEvent =
        serde_json::from_slice(&serde_json::to_vec(&err).unwrap()).unwrap();
    assert_eq!(back2, err);
}

/// The unsolicited daemon→panel push (W8) round-trips — it is broadcast to every
/// attached client, so its wire shape must hold.
#[test]
fn ext_panel_push_serde_roundtrip() {
    let ev = DaemonEvent::ExtPanelPush {
        ext_id: "run.koma.example".into(),
        panel_id: "sidebar".into(),
        payload: serde_json::json!({ "tick": 42 }),
    };
    let back: DaemonEvent = serde_json::from_slice(&serde_json::to_vec(&ev).unwrap()).unwrap();
    assert_eq!(back, ev);
}

/// The lifecycle error report (remote connect failure) round-trips — the
/// thin client pushes this to the daemon so the user sees a toast.
#[test]
fn connect_failed_serde_roundtrip() {
    let req = ClientRequest::ConnectFailed {
        error: "ssh: auth failed".into(),
    };
    let back: ClientRequest =
        serde_json::from_slice(&serde_json::to_vec(&req).unwrap()).unwrap();
    assert_eq!(back, req);
}
