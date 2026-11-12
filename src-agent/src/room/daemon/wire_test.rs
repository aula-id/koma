#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use tokio::sync::mpsc::unbounded_channel;

use crate::ipc::client;
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::split_stream;
use crate::room::actor::{ActorMsg, NoBuiltins, RoomConfig, RoomEvent, StatusSnapshot};
use crate::room::lane::{ModelClient, ModelReply, ModelRequest};
use crate::room::wire::{RoomClient, RoomServer};

use super::assemble::RoomSession;

struct Scripted {
    text: String,
}

impl ModelClient for Scripted {
    fn stream<'a>(&'a self, _request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let text = self.text.clone();
        Box::pin(async move { ModelReply { text, ..Default::default() } })
    }
}

fn sock_path() -> PathBuf {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    #[cfg(unix)]
    {
        let dir = std::env::temp_dir().join(format!("koma-room-wire-{nanos}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir.join("room.sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\koma-room-wire-{nanos}"))
    }
}

async fn write<T: serde::Serialize>(write: &mut crate::ipc::IpcWriteHalf, value: &T) {
    let bytes = serde_json::to_vec(value).expect("json");
    write_frame_to(write, &bytes).await.expect("write");
}

async fn read<T: serde::de::DeserializeOwned>(
    read: &mut crate::ipc::IpcReadHalf,
    reader: &mut FrameReader,
) -> T {
    let bytes = read_frame_from(read, reader).await.expect("read");
    serde_json::from_slice(&bytes).expect("decode")
}

#[tokio::test]
async fn a_client_sends_a_line_and_reads_the_chief() {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("koma-room-wire-sess-{nanos}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sock = sock_path();
    let listener = crate::ipc::server::bind(&sock).expect("bind");
    let model = Arc::new(Scripted { text: "Hello, I am the Chief.".into() });
    let mut session = RoomSession::open(&dir, model, Arc::new(NoBuiltins), RoomConfig::default()).expect("open");
    let (dummy_tx, dummy_rx) = unbounded_channel();
    drop(dummy_tx);
    let events = std::mem::replace(&mut session.room.events, dummy_rx);
    let to_actor = session.room.to_actor.clone();
    let status = SessionStatus {
        session_id: "sess-1".into(),
        name: "secure".into(),
        pwd: dir.display().to_string(),
        working: false,
    };
    let shutting_down = Arc::new(AtomicBool::new(false));
    let serve = tokio::spawn(super::serve::run(
        listener,
        to_actor,
        events,
        status,
        Arc::clone(&shutting_down),
        super::models::test_book(&dir),
        Arc::new(super::stage::Staging::new(&dir)),
    ));

    let probe = client::connect(&sock).await.expect("probe connect");
    let (mut probe_read, mut probe_write) = split_stream(probe);
    write(&mut probe_write, &ClientRequest::Status).await;
    let mut probe_reader = FrameReader::new();
    let frame: DaemonFrame = read(&mut probe_read, &mut probe_reader).await;
    match frame.event {
        DaemonEvent::Status(got) => {
            assert_eq!(got.session_id, "sess-1");
            assert_eq!(got.name, "secure");
        }
        other => panic!("status probe got {other:?}"),
    }
    drop(probe_read);
    drop(probe_write);

    let stream = client::connect(&sock).await.expect("attach connect");
    let (mut read_half, mut write_half) = split_stream(stream);
    let mut reader = FrameReader::new();
    write(&mut write_half, &RoomClient::Hello).await;
    let welcome: RoomServer = read(&mut read_half, &mut reader).await;
    assert!(matches!(&welcome, RoomServer::Hello { kind, .. } if kind == "secure"), "{welcome:?}");
    write(&mut write_half, &RoomClient::User { text: "hi".into() }).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let chief = loop {
        let frame: RoomServer = tokio::time::timeout_at(deadline, read_frame_from(&mut read_half, &mut reader))
            .await
            .expect("chief timed out")
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .expect("chief frame");
        if let RoomServer::Chief { text } = frame {
            break text;
        }
    };
    assert!(chief.contains("Chief"), "{chief}");

    write(&mut write_half, &RoomClient::Quit).await;
    let closing: RoomServer = read(&mut read_half, &mut reader).await;
    assert_eq!(closing, RoomServer::Closed);

    let stop = client::connect(&sock).await.expect("quit connect");
    let (_stop_read, mut stop_write) = split_stream(stop);
    write(&mut stop_write, &ClientRequest::QuitDaemon).await;
    drop(_stop_read);
    drop(stop_write);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !shutting_down.load(Ordering::Relaxed) {
        if tokio::time::Instant::now() > deadline {
            panic!("QuitDaemon did not stop the secure daemon");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    serve.abort();
    let _ = session.room.to_actor.send(ActorMsg::Shutdown);
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock);
}

/// A serve loop fed by hand, with no actor, so the test controls what was said and when.
struct Bench {
    sock: PathBuf,
    events: tokio::sync::mpsc::UnboundedSender<RoomEvent>,
    from_clients: tokio::sync::mpsc::UnboundedReceiver<ActorMsg>,
    serve: tokio::task::JoinHandle<()>,
}

fn bench() -> Bench {
    let sock = sock_path();
    let listener = crate::ipc::server::bind(&sock).expect("bind");
    let (events, events_rx) = unbounded_channel();
    let (to_actor, from_clients) = unbounded_channel();
    let status = SessionStatus { session_id: "s".into(), name: "secure".into(), pwd: String::new(), working: false };
    let serve = tokio::spawn(super::serve::run(listener, to_actor, events_rx, status, Arc::new(AtomicBool::new(false)), super::models::test_book(sock.parent().expect("dir")), Arc::new(super::stage::Staging::new(sock.parent().expect("dir")))));
    Bench { sock, events, from_clients, serve }
}

impl Bench {
    fn say(&self, text: &str) {
        self.events.send(RoomEvent::Chief(text.into())).expect("serve alive");
    }

    fn status(&self, open: usize) {
        let mut snapshot = StatusSnapshot::default();
        snapshot.counts.open = open;
        self.events.send(RoomEvent::Status(snapshot)).expect("serve alive");
    }

    /// Let the serve loop take what was sent before anyone attaches.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    async fn attach(&self) -> (crate::ipc::IpcReadHalf, crate::ipc::IpcWriteHalf, FrameReader, usize) {
        let stream = client::connect(&self.sock).await.expect("attach");
        let (mut read_half, mut write_half) = split_stream(stream);
        let mut reader = FrameReader::new();
        write(&mut write_half, &RoomClient::Hello).await;
        let welcome: RoomServer = read(&mut read_half, &mut reader).await;
        let RoomServer::Hello { replayed, .. } = welcome else { panic!("first frame was {welcome:?}") };
        (read_half, write_half, reader, replayed)
    }

    fn stop(self) {
        self.serve.abort();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.sock);
    }
}

async fn next(read_half: &mut crate::ipc::IpcReadHalf, reader: &mut FrameReader) -> RoomServer {
    let deadline = Duration::from_secs(5);
    let bytes = tokio::time::timeout(deadline, read_frame_from(read_half, reader))
        .await
        .expect("timed out waiting for a frame")
        .expect("read");
    serde_json::from_slice(&bytes).expect("decode")
}

#[tokio::test]
async fn a_client_that_attaches_late_gets_what_was_said_while_nobody_listened() {
    let mut b = bench();
    b.say("good morning");
    b.say("scanning now");
    b.status(1);
    b.status(2);
    b.settle().await;

    let (mut r, _w, mut reader, replayed) = b.attach().await;
    assert_eq!(replayed, 3, "the latest status and two lines");
    assert!(matches!(next(&mut r, &mut reader).await, RoomServer::Status { open: 2, .. }));
    assert_eq!(next(&mut r, &mut reader).await, RoomServer::Chief { text: "good morning".into() });
    assert_eq!(next(&mut r, &mut reader).await, RoomServer::Chief { text: "scanning now".into() });
    assert_eq!(b.from_clients.recv().await, Some(ActorMsg::Resync), "an attach asks for a current status");

    b.say("live line");
    assert_eq!(next(&mut r, &mut reader).await, RoomServer::Chief { text: "live line".into() });
    b.stop();
}

#[tokio::test]
async fn a_second_attach_sees_the_history_once_and_in_order() {
    let b = bench();
    b.say("one");
    b.settle().await;
    let (r1, w1, _reader1, replayed) = b.attach().await;
    assert_eq!(replayed, 1);
    drop((r1, w1));

    b.say("two while detached");
    b.settle().await;
    let (mut r, _w, mut reader, replayed) = b.attach().await;
    assert_eq!(replayed, 2);
    assert_eq!(next(&mut r, &mut reader).await, RoomServer::Chief { text: "one".into() });
    assert_eq!(next(&mut r, &mut reader).await, RoomServer::Chief { text: "two while detached".into() });
    b.stop();
}

#[tokio::test]
async fn an_attach_to_a_quiet_room_replays_nothing() {
    let b = bench();
    let (_r, _w, _reader, replayed) = b.attach().await;
    assert_eq!(replayed, 0);
    b.stop();
}

#[tokio::test]
async fn a_client_lists_models_and_sets_one() {
    let bench = bench();
    let (mut read_half, mut write_half, mut reader, _) = bench.attach().await;
    write(&mut write_half, &RoomClient::Models).await;
    let RoomServer::Models { choices, lanes } = next_matching(&mut read_half, &mut reader, |f| matches!(f, RoomServer::Models { .. })).await else { unreachable!() };
    assert_eq!(choices.len(), 2);
    assert!(lanes.iter().any(|l| l.avatar == "red" && l.lane == "work"));
    write(&mut write_half, &RoomClient::SetModel { avatar: "red".into(), lane: "work".into(), model: "m2".into() }).await;
    let RoomServer::Models { lanes, .. } = next_matching(&mut read_half, &mut reader, |f| matches!(f, RoomServer::Models { .. })).await else { unreachable!() };
    assert_eq!(lanes.iter().find(|l| l.avatar == "red" && l.lane == "work").expect("row").model, "Strong");
    write(&mut write_half, &RoomClient::SetModel { avatar: "red".into(), lane: "work".into(), model: "nope".into() }).await;
    let note = next_matching(&mut read_half, &mut reader, |f| matches!(f, RoomServer::Notice { .. })).await;
    assert_eq!(note, RoomServer::Notice { text: "that model is not registered".into() });
    bench.stop();
}

#[tokio::test]
async fn a_paste_is_answered_with_what_goes_in_the_draft_and_the_message_carries_the_image() {
    let mut bench = bench();
    let (mut read_half, mut write_half, mut reader, _) = bench.attach().await;
    write(&mut write_half, &RoomClient::Paste { text: "pwd".into() }).await;
    let pasted = next_matching(&mut read_half, &mut reader, |f| matches!(f, RoomServer::Pasted { .. })).await;
    assert_eq!(pasted, RoomServer::Pasted { insert: "pwd".into() });
    let long = "row\n".repeat(20);
    write(&mut write_half, &RoomClient::Paste { text: long.clone() }).await;
    let pasted = next_matching(&mut read_half, &mut reader, |f| matches!(f, RoomServer::Pasted { .. })).await;
    assert_eq!(pasted, RoomServer::Pasted { insert: "[Pasted Text #1]".into() });
    write(&mut write_half, &RoomClient::User { text: "read [Pasted Text #1]".into() }).await;
    let text = loop {
        match bench.from_clients.recv().await.expect("the actor was told") {
            ActorMsg::User(text) => break text,
            ActorMsg::Resync => {}
            other => panic!("expected a plain user message, got {other:?}"),
        }
    };
    assert!(text.contains("<<<pasted_text n=1") && text.contains(&long), "the chip is expanded for the model");
    bench.stop();
}

async fn next_matching(read_half: &mut crate::ipc::IpcReadHalf, reader: &mut FrameReader, want: impl Fn(&RoomServer) -> bool) -> RoomServer {
    loop {
        let frame = next(read_half, reader).await;
        if want(&frame) {
            return frame;
        }
    }
}

#[test]
fn a_part_frame_round_trips_and_is_kept_as_history() {
    use crate::room::actor::LanePart;
    let frame = RoomServer::Part {
        author: "chief".into(),
        lane: "chat".into(),
        part: LanePart::ToolCall { id: "c1".into(), name: "bash".into(), args: "{\"command\":\"pwd\"}".into() },
    };
    let json = serde_json::to_string(&frame).expect("encode");
    assert!(json.contains("\"part\"") && json.contains("\"kind\":\"tool_call\""), "{json}");
    assert_eq!(serde_json::from_str::<RoomServer>(&json).expect("decode"), frame);
    assert!(frame.is_history(), "a late client replays the tool calls it missed");
}

#[test]
fn live_pieces_are_not_replayed_to_a_late_client() {
    use crate::room::actor::LanePart;
    let live = RoomServer::Part { author: "chief".into(), lane: "chat".into(), part: LanePart::TextDelta { text: "Hel".into() } };
    assert!(!live.is_history());
    let json = serde_json::to_string(&live).expect("encode");
    assert_eq!(serde_json::from_str::<RoomServer>(&json).expect("decode"), live);
}
