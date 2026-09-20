use super::*;
use std::sync::{Arc, Mutex, mpsc};
use crate::app::runtime::client::connect::TransportKind;
use crate::ipc::proto::{DaemonFrame, RunExtension};

fn initial_state() -> RunState {
    RunState {
        session_id: "session-a".into(),
        name: "session-a".into(),
        mode: "auto".into(),
        short_send: true,
        extensions: vec![RunExtension {
            id: "run.koma.one".into(),
            kind: "oneshot".into(),
            name: "One".into(),
            activation: "on-demand".into(),
            enabled: true,
            active: false,
            running: false,
        }],
        ..Default::default()
    }
}

fn connection(
    runtime: &tokio::runtime::Runtime,
    mut state: RunState,
    mut apply: impl FnMut(&ClientRequest, &mut RunState) -> Option<DaemonEvent> + Send + 'static,
) -> (
    Connection,
    Arc<Mutex<Vec<ClientRequest>>>,
    std::thread::JoinHandle<()>,
) {
    let (req_tx, req_rx) = mpsc::channel::<ClientRequest>();
    let (frame_tx, frame_rx) = mpsc::channel();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let thread = std::thread::spawn(move || {
        for request in req_rx {
            recorded.lock().unwrap().push(request.clone());
            let event = if let ClientRequest::GetRunState { req_seq } = request {
                // A stale reply must never satisfy the new setup barrier.
                let _ = frame_tx.send(DaemonFrame {
                    seq: 0,
                    event: DaemonEvent::RunState {
                        req_seq: req_seq.wrapping_add(100),
                        state: RunState::default(),
                    },
                });
                Some(DaemonEvent::RunState {
                    req_seq,
                    state: state.clone(),
                })
            } else {
                apply(&request, &mut state)
            };
            if let Some(event) = event {
                let frame = DaemonFrame { seq: 1, event };
                let frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
                if frame_tx.send(frame).is_err() {
                    break;
                }
            }
        }
    });
    (
        Connection {
            req_tx,
            frame_rx,
            writer_handle: runtime.spawn(async {}),
            prebuffered: vec![],
            daemon_version: None,
            transport: TransportKind::Local {
                session_id: "session-a".into(),
            },
        },
        requests,
        thread,
    )
}

#[test]
fn drss_and_extension_setup_is_confirmed_before_submit() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (conn, requests, worker) = connection(&rt, initial_state(), |request, state| {
        match request {
            ClientRequest::SetSessionPrefs {
                short_send,
                max_output_tokens,
                context_window_limit,
                context_model_alias,
                ..
            } => {
                state.short_send = short_send.unwrap();
                state.max_output_tokens = max_output_tokens.unwrap();
                state.context_window_limit = context_window_limit.unwrap();
                state.context_model_alias = context_model_alias.clone().unwrap();
            }
            ClientRequest::SetSessionExtensions { load, .. } => {
                state.extensions[0].active = load.contains(&state.extensions[0].id);
            }
            ClientRequest::SubmitInput { .. } => {
                assert!(!state.short_send);
                assert!(state.extensions[0].active);
                assert_eq!(state.context_model_alias, "context-alias");
            }
            _ => {}
        }
        Some(DaemonEvent::Ack)
    });
    let cli = RunCli {
        short_send: Some(false),
        max_tokens: Some(0),
        context_window_limit: Some(128_000),
        context_model_alias: Some("context-alias".into()),
        extensions: vec!["run.koma.one".into()],
        ..Default::default()
    };
    assert_eq!(
        run_attached(&conn, &cli, Some("go".into())).unwrap(),
        EXIT_OK
    );
    drop(conn);
    worker.join().unwrap();
    let calls = requests.lock().unwrap();
    let submit = calls
        .iter()
        .position(|c| matches!(c, ClientRequest::SubmitInput { .. }))
        .unwrap();
    assert!(calls[..submit]
        .iter()
        .any(|c| matches!(c, ClientRequest::SetSessionExtensions { .. })));
    assert!(matches!(
        calls[submit - 1],
        ClientRequest::GetRunState { .. }
    ));
}

#[test]
fn rejected_or_unapplied_setup_never_submits_prompt() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    for explicit_error in [true, false] {
        let (conn, requests, worker) = connection(&rt, initial_state(), move |request, _| {
            if matches!(request, ClientRequest::SetSessionExtensions { .. }) && explicit_error {
                Some(DaemonEvent::Error("extension setup rejected".into()))
            } else {
                Some(DaemonEvent::Ack)
            }
        });
        let cli = RunCli {
            extensions: vec!["run.koma.one".into()],
            ..Default::default()
        };
        assert!(run_attached(&conn, &cli, Some("go".into())).is_err());
        drop(conn);
        worker.join().unwrap();
        assert!(!requests
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, ClientRequest::SubmitInput { .. })));
    }
}

#[test]
fn status_is_inspection_only_and_approval_keeps_exit_three() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    for status_only in [true, false] {
        let (conn, requests, worker) = connection(
            &rt,
            RunState {
                awaiting_approval: true,
                ..initial_state()
            },
            |_, _| Some(DaemonEvent::Ack),
        );
        let prompt = if status_only { None } else { Some("go".into()) };
        assert_eq!(
            run_attached(&conn, &RunCli::default(), prompt).unwrap(),
            if status_only { EXIT_OK } else { EXIT_APPROVAL }
        );
        drop(conn);
        worker.join().unwrap();
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .all(|c| matches!(c, ClientRequest::GetRunState { .. })));
    }
}

#[test]
fn state_disconnect_is_an_error_instead_of_successful_setup() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (conn, _, worker) = connection(&rt, initial_state(), |_, _| None);
    let state = request_state(&conn, Duration::from_millis(100)).unwrap();
    assert_eq!(state.session_id, "session-a");
    drop(conn);
    worker.join().unwrap();
    // Disconnected transport, with no worker left to supply a confirmation.
    let (req_tx, _req_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::channel();
    drop(frame_tx);
    let dead = Connection {
        req_tx,
        frame_rx,
        writer_handle: rt.spawn(async {}),
        prebuffered: vec![],
        daemon_version: None,
        transport: TransportKind::Local {
            session_id: "session-a".into(),
        },
    };
    assert!(request_state(&dead, Duration::from_millis(10)).is_err());
}
