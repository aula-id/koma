//! Hub drive proof: exercise the hub with NO socket and assert (a) the seq'd
//! frame stream — a Snapshot on attach, then a Delta on the next state change
//! with `seq = N+1` (stage 4), and (b) that a controller's mutating request is
//! actually applied through the shared action path, single-writer is enforced,
//! the controller seat passes on detach, and QuitDaemon latches shutdown
//! (stage 5). These stand in for the accept loop so the full drive path is
//! covered without a real socket.

use super::*;
use crate::app::mode::Mode;
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, KeyCodeWire, KeyWire, StateDelta, key_mods};

/// A keyless client + a current-thread tokio runtime — the minimal context the
/// mutating-request path needs. `client = None` means a `Submit`/`Resend`-style
/// action short-circuits to "no active session" (still `Ok`, so still `Ack`),
/// while a `SendKey` editing the composer mutates `state` with no client at all.
fn ctx() -> (Option<Arc<OpenRouterClient>>, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build test runtime");
    (None, rt)
}

/// Attaching a client yields a `Snapshot{seq=1}`; a subsequent status change
/// yields a `Delta{seq=2}` carrying the new global status.
#[test]
fn attach_then_change_emits_snapshot_then_seqd_delta() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::Attach { foreground_id: None, cwd: None } }, &mut state, &mut client, &h);

    let f0 = frame_rx.try_recv().expect("hello frame on attach");
    assert_eq!(f0.seq, 1, "first frame seq");
    assert!(matches!(f0.event, DaemonEvent::Hello { .. }), "attach emits Hello first, got {:?}", f0.event);
    let f1 = frame_rx.try_recv().expect("snapshot frame on attach");
    assert_eq!(f1.seq, 2, "snapshot follows hello");
    assert!(matches!(f1.event, DaemonEvent::Snapshot(_)), "attach emits a Snapshot after Hello, got {:?}", f1.event);

    state.rest.fg_mut().status = "streaming".into();
    hub.stream_deltas(&mut state);
    let f2 = frame_rx.try_recv().expect("delta frame after change");
    assert_eq!(f2.seq, 3, "delta seq is N+1");
    match f2.event {
        DaemonEvent::Delta(StateDelta::StatusChanged { session_id, text }) => {
            assert_eq!(session_id, None, "global status delta");
            assert_eq!(text, "streaming");
        }
        other => panic!("expected StatusChanged delta, got {other:?}"),
    }

    hub.stream_deltas(&mut state);
    assert!(frame_rx.try_recv().is_err(), "no frame emitted when state is unchanged");
}

/// A structural change (a session entering tool-approval) resyncs with a full
/// `Snapshot`, not a partial delta.
#[test]
fn structural_change_emits_full_snapshot() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 7, frame_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 7, req: ClientRequest::Attach { foreground_id: None, cwd: None } }, &mut state, &mut client, &h);
    let _hello = frame_rx.try_recv().expect("attach hello");
    let _snap = frame_rx.try_recv().expect("attach snapshot");

    state.rest.fg_mut().awaiting_approval = true;
    hub.stream_deltas(&mut state);
    let f = frame_rx.try_recv().expect("frame after structural change");
    assert_eq!(f.seq, 3);
    assert!(matches!(f.event, DaemonEvent::Snapshot(_)), "structural change must resync with a full Snapshot, got {:?}", f.event);
}

/// Build-skew handshake (task #142): an attaching client's VERY FIRST frame is a
/// `Hello` carrying the hub's stored fingerprint, ahead of the initial Snapshot.
#[test]
fn attach_emits_hello_then_snapshot() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::Attach { foreground_id: None, cwd: None } }, &mut state, &mut client, &h);

    let f0 = frame_rx.try_recv().expect("hello frame on attach");
    assert_eq!(f0.seq, 1);
    match f0.event {
        DaemonEvent::Hello { version } => {
            assert_eq!(version, crate::model::store::build_fingerprint(), "Hello carries the hub's stored fingerprint");
        }
        other => panic!("expected Hello first, got {other:?}"),
    }
    let f1 = frame_rx.try_recv().expect("snapshot frame after hello");
    assert_eq!(f1.seq, 2);
    assert!(matches!(f1.event, DaemonEvent::Snapshot(_)));
}

/// An observer (second client) is rejected when it sends a mutating request,
/// and the controller (first client) is acknowledged.
#[test]
fn observer_mutation_is_rejected_controller_acked() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (ctl_tx, ctl_rx) = std::sync::mpsc::channel::<DaemonFrame>();
    let (obs_tx, obs_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: ctl_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Register { client_id: 2, frame_tx: obs_tx }, &mut state, &mut client, &h);

    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::SubmitInput { text: "hi".into() } }, &mut state, &mut client, &h);
    assert!(matches!(ctl_rx.try_recv().expect("controller reply").event, DaemonEvent::Ack));

    hub.handle_inbound(HubInbound::Request { client_id: 2, req: ClientRequest::SubmitInput { text: "nope".into() } }, &mut state, &mut client, &h);
    assert!(matches!(obs_rx.try_recv().expect("observer reply").event, DaemonEvent::Error(_)));
}

/// An unknown session UUID on a UUID-keyed control request is an Error + no-op.
#[test]
fn unknown_session_uuid_errors_not_panics() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, rx) = std::sync::mpsc::channel::<DaemonFrame>();
    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::SwitchForeground { session_id: "does-not-exist".into() } }, &mut state, &mut client, &h);
    assert!(matches!(rx.try_recv().expect("reply").event, DaemonEvent::Error(_)));
}

/// A controller's `SendKey` is routed through the SAME local input pipeline.
#[test]
fn controller_sendkey_edits_composer() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::SendKey(KeyWire { code: KeyCodeWire::Char('z'), mods: 0 }) }, &mut state, &mut client, &h);

    assert_eq!(state.rest.fg().input, "z", "key reached the composer via apply_action");
    assert!(matches!(rx.try_recv().expect("reply").event, DaemonEvent::Ack));
}

/// When the controller detaches, the seat passes to the next remaining client.
#[test]
fn controller_seat_passes_on_detach() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (c1_tx, _c1_rx) = std::sync::mpsc::channel::<DaemonFrame>();
    let (c2_tx, c2_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: c1_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Register { client_id: 2, frame_tx: c2_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::Detach }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 2, req: ClientRequest::SendKey(KeyWire { code: KeyCodeWire::Char('q'), mods: key_mods::CONTROL }) }, &mut state, &mut client, &h);
    assert!(matches!(c2_rx.try_recv().expect("promoted controller reply").event, DaemonEvent::Ack));
}

/// `QuitDaemon` from the controller latches the shutdown flag the loop polls.
#[test]
fn quit_daemon_latches_shutdown() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    assert!(!hub.should_shutdown(), "shutdown not latched before QuitDaemon");
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::QuitDaemon }, &mut state, &mut client, &h);
    assert!(hub.should_shutdown(), "QuitDaemon latches shutdown");
    assert!(matches!(rx.try_recv().expect("reply").event, DaemonEvent::Ack));
}

/// A `Disconnect` (socket EOF) deregisters the client and passes the controller seat.
#[test]
fn disconnect_deregisters_and_passes_seat() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (c1_tx, _c1_rx) = std::sync::mpsc::channel::<DaemonFrame>();
    let (c2_tx, c2_rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: c1_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Register { client_id: 2, frame_tx: c2_tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Disconnect { client_id: 1 }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 2, req: ClientRequest::SubmitInput { text: "x".into() } }, &mut state, &mut client, &h);
    assert!(matches!(c2_rx.try_recv().expect("promoted controller reply").event, DaemonEvent::Ack));
}

// ─── daemon stage 10: tombstone close + self-exit ────────────────────────

use crate::app::state::SessionRuntime;

/// Append a fresh (live) session to `state` and return its stable id.
fn push_session(state: &mut AppState) -> String {
    let rt = SessionRuntime::new();
    let id = rt.id.clone();
    state.rest.sessions.push(rt);
    id
}

/// `QuitSession` on a known id TOMBSTONES that session and Acks; the other stays live.
#[test]
fn quit_session_tombstones_keeps_slot_and_acks() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, rx) = std::sync::mpsc::channel::<DaemonFrame>();

    let id1 = push_session(&mut state);
    let id0 = state.rest.sessions[0].id.clone();
    let len_before = state.rest.sessions.len();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::QuitSession { session_id: id1.clone() } }, &mut state, &mut client, &h);

    assert!(matches!(rx.try_recv().expect("reply").event, DaemonEvent::Ack));
    assert_eq!(state.rest.sessions.len(), len_before, "tombstone keeps the slot");
    let s1 = state.rest.sessions.iter().find(|s| s.id == id1).expect("slot kept");
    assert!(s1.closed, "quit session is tombstoned");
    let s0 = state.rest.sessions.iter().find(|s| s.id == id0).expect("other slot");
    assert!(!s0.closed, "the other session stays live");
}

/// `QuitSession` on an unknown id is an Error + no-op.
#[test]
fn quit_session_unknown_id_errors_no_close() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, rx) = std::sync::mpsc::channel::<DaemonFrame>();

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::QuitSession { session_id: "nope".into() } }, &mut state, &mut client, &h);

    assert!(matches!(rx.try_recv().expect("reply").event, DaemonEvent::Error(_)));
    assert!(state.rest.sessions.iter().all(|s| !s.closed), "no session closed on unknown id");
}

/// Closing the FOREGROUND session repoints `foreground` onto a still-live one.
#[test]
fn quit_foreground_repoints_to_live_session() {
    let mut state = AppState::new(Mode::Chat);
    let (mut client, rt) = ctx();
    let h = rt.handle().clone();
    let (mut hub, _runner_tx) = DaemonHub::new();
    let (tx, _rx) = std::sync::mpsc::channel::<DaemonFrame>();

    let id1 = push_session(&mut state);
    state.rest.foreground = 1;

    hub.handle_inbound(HubInbound::Register { client_id: 1, frame_tx: tx }, &mut state, &mut client, &h);
    hub.handle_inbound(HubInbound::Request { client_id: 1, req: ClientRequest::QuitSession { session_id: id1 } }, &mut state, &mut client, &h);

    assert_eq!(state.rest.foreground, 0, "foreground repointed to the live session");
    assert!(!state.rest.fg().closed, "foreground is a live session");
}

/// A CLOSED session is SKIPPED by `service_all_sessions`.
#[test]
fn closed_session_is_skipped_by_servicer() {
    let mut state = AppState::new(Mode::Chat);
    let (client, rt) = ctx();
    let h = rt.handle().clone();

    let (ev_tx, ev_rx) = tokio::sync::mpsc::unbounded_channel::<crate::service::StreamEvent>();
    ev_tx.send(crate::service::StreamEvent::Token("hi".into())).expect("queue a token");
    state.rest.sessions[0].active_rx = Some(ev_rx);
    state.rest.sessions[0].begin_stream();
    state.rest.sessions[0].closed = true;

    let _ = service_all_sessions(&mut state, &client, &h);

    assert_eq!(state.rest.sessions[0].streaming.as_deref(), Some(""), "closed session was skipped: its streaming buffer stayed empty");
    assert!(state.rest.sessions[0].active_rx.is_some(), "closed session was skipped: its receiver was never taken/drained");
    drop(ev_tx);
}

/// `close_all_sessions` tombstones every session; `all_sessions_closed` then reports true.
#[test]
fn close_all_then_all_closed_true() {
    let mut state = AppState::new(Mode::Chat);
    let _id1 = push_session(&mut state);
    let _id2 = push_session(&mut state);
    assert!(!all_sessions_closed(&state), "not closed before kill-all");

    // `close_all_sessions` is now a method on the hub (C1.5: it also repoints the
    // per-client foreground pointers). With no clients enrolled the per-client repoint
    // is a no-op, so the `state` outcome asserted here is identical to before.
    let (mut hub, _runner_tx) = DaemonHub::new();
    hub.close_all_sessions(&mut state);

    assert!(all_sessions_closed(&state), "every session closed after kill-all");
    assert!(state.rest.sessions.iter().all(|s| !s.is_working()), "no tombstone reads as working");
}

/// The forwarded `/quit` kill-all path simulation.
#[test]
fn should_quit_flag_drives_close_all() {
    let mut state = AppState::new(Mode::Chat);
    let _id1 = push_session(&mut state);
    let (mut hub, _runner_tx) = DaemonHub::new();

    state.rest.should_quit = true;

    if state.rest.should_quit {
        hub.close_all_sessions(&mut state);
        state.rest.should_quit = false;
    }

    assert!(!state.rest.should_quit, "flag cleared by the daemon close path");
    assert!(all_sessions_closed(&state), "kill-all flag closed every session");
}

// ─── daemon stage 11: detached-approval park timeout + parked cadence ─────

/// Put session `idx` into the PARKED-on-approval state with one unanswered risky tool call.
fn park_on_approval(state: &mut AppState, idx: usize) {
    use crate::dto::chat::{FunctionCall, ToolCall};
    let s = &mut state.rest.sessions[idx];
    s.waiting = true;
    s.awaiting_approval = true;
    s.approval_reason = Some("writes outside workspace".into());
    s.pending_tool_calls = vec![ToolCall {
        id: "call-1".into(),
        kind: "function".into(),
        function: FunctionCall { name: "bash".into(), arguments: "{}".into() },
    }];
    s.tool_idx = 0;
}

/// Detached + awaiting: the first pass STAMPS the park timer but does NOT deny before the window elapses.
#[test]
fn park_timer_stamps_when_detached_no_premature_deny() {
    let mut state = AppState::new(Mode::Chat);
    park_on_approval(&mut state, 0);
    assert!(state.rest.sessions[0].park_started_at.is_none(), "no timer yet");

    let denied = service_approval_park_timeouts(&mut state, false);

    assert!(!denied, "nothing denied on the first detached tick");
    assert!(state.rest.sessions[0].park_started_at.is_some(), "the park timer is stamped on the first detached+awaiting tick");
    assert!(state.rest.sessions[0].awaiting_approval, "still parked — the window has not elapsed");
}

/// While a client IS attached, the timer never runs.
#[test]
fn park_timer_cleared_while_client_attached() {
    let mut state = AppState::new(Mode::Chat);
    park_on_approval(&mut state, 0);
    state.rest.sessions[0].park_started_at = Some(Instant::now());

    let denied = service_approval_park_timeouts(&mut state, true);

    assert!(!denied, "an attached operator is never auto-denied");
    assert!(state.rest.sessions[0].park_started_at.is_none(), "the timer is cleared while a client is attached");
    assert!(state.rest.sessions[0].awaiting_approval, "still parked, waiting for the operator");
}

/// Once a DETACHED park exceeds `APPROVAL_PARK_TIMEOUT`, the pending call is auto-denied.
#[test]
fn park_timeout_auto_denies_after_window() {
    let mut state = AppState::new(Mode::Chat);
    park_on_approval(&mut state, 0);
    let past = Instant::now()
        .checked_sub(APPROVAL_PARK_TIMEOUT + Duration::from_secs(1))
        .expect("instant far enough in the past");
    state.rest.sessions[0].park_started_at = Some(past);

    let denied = service_approval_park_timeouts(&mut state, false);

    assert!(denied, "the expired park was auto-denied");
    let s = &state.rest.sessions[0];
    assert!(!s.awaiting_approval, "auto-deny clears the approval park");
    assert!(s.pending_tool_calls.is_empty(), "pending calls were answered/drained");
    assert!(!s.waiting, "the session goes idle after the auto-deny");
    assert!(s.park_started_at.is_none(), "the park timer is cleared after the deny");
}

/// A session that is NOT awaiting approval has its timer cleared every pass.
#[test]
fn park_timer_reset_when_not_awaiting() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].park_started_at = Some(Instant::now());
    state.rest.sessions[0].awaiting_approval = false;

    let denied = service_approval_park_timeouts(&mut state, false);

    assert!(!denied, "an idle session is never denied");
    assert!(state.rest.sessions[0].park_started_at.is_none(), "a non-awaiting session's timer is cleared");
}

/// A CLOSED session is ignored by the park timer.
#[test]
fn park_timeout_skips_closed_session() {
    let mut state = AppState::new(Mode::Chat);
    park_on_approval(&mut state, 0);
    let stamp = Instant::now()
        .checked_sub(APPROVAL_PARK_TIMEOUT + Duration::from_secs(1))
        .expect("past instant");
    state.rest.sessions[0].park_started_at = Some(stamp);
    state.rest.sessions[0].closed = true;

    let denied = service_approval_park_timeouts(&mut state, false);

    assert!(!denied, "a closed session is skipped, never auto-denied");
    assert!(state.rest.sessions[0].park_started_at.is_some(), "a closed session's fields are left untouched");
}

/// Cadence predicate: detached + parked-on-approval → slow; attached → fast.
#[test]
fn cadence_slow_when_parked_detached_fast_when_attached() {
    let mut state = AppState::new(Mode::Chat);
    park_on_approval(&mut state, 0);

    assert!(all_idle_or_parked_detached(&state, false), "detached + parked-on-approval should nap on the slow cadence");
    assert!(!all_idle_or_parked_detached(&state, true), "an attached client over a parked session keeps the fast cadence");
}

/// Cadence predicate: a streaming session keeps the daemon fast.
#[test]
fn cadence_fast_when_session_streaming() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].begin_stream();

    assert!(!all_idle_or_parked_detached(&state, false), "a streaming session keeps the daemon on the fast cadence (detached)");
    assert!(!all_idle_or_parked_detached(&state, true), "a streaming session keeps the daemon on the fast cadence (attached)");
}

/// Cadence predicate: a fully idle daemon naps slow.
#[test]
fn cadence_slow_when_fully_idle() {
    let state = AppState::new(Mode::Chat);
    assert!(all_idle_or_parked_detached(&state, false), "idle + detached naps slow");
    assert!(all_idle_or_parked_detached(&state, true), "idle + an attached-but-quiet client still naps slow");
}
