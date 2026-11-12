#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use tokio::sync::mpsc::unbounded_channel;

use crate::ipc::client;
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::split_stream;
use crate::room::actor::{ActorMsg, NoBuiltins, RoomConfig};
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
    let serve = tokio::spawn(super::serve::run(listener, to_actor, events, status));

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
    assert_eq!(welcome, RoomServer::hello());
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

    serve.abort();
    let _ = session.room.to_actor.send(ActorMsg::Shutdown);
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock);
}
