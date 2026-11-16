//! The room socket: connect, the `hello` handshake, writes, and the status
//! probe that tells a takeover from a daemon exit. It uses the same framing and
//! connect helpers as the terminal attach client, so it works over Unix sockets
//! and Windows named pipes alike.
use crate::app::runtime::sdk::connection::BUDGET;
use crate::ipc::{
    client,
    frame::{read_frame_from, write_frame_to, FrameReader},
    proto::{ClientRequest, DaemonEvent, DaemonFrame},
    split_stream, IpcReadHalf, IpcWriteHalf,
};
use crate::room::wire::RoomClient;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// How long the room has to answer `hello`.
const HELLO_WAIT: Duration = Duration::from_secs(5);
/// How long the status probe after an end of stream may take.
const PROBE_WAIT: Duration = Duration::from_millis(500);
/// `AttachResult.daemon.features` keeps at most this many names.
pub const MAX_FEATURES: usize = 64;

/// What the room said in its `hello`. Absent fields mean an older room.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hello {
    pub replayed: usize,
    pub features: Vec<String>,
    pub version: String,
    pub template_revision: Option<String>,
}

impl Hello {
    fn parse(frame: &Value) -> Option<Self> {
        let hello = frame.get("hello")?;
        if hello.get("kind").and_then(Value::as_str) != Some("secure") {
            return None;
        }
        Some(Self {
            replayed: hello
                .get("replayed")
                .and_then(Value::as_u64)
                .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX)),
            features: hello
                .get("features")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            version: hello
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            template_revision: hello
                .get("template_revision")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// Nothing accepts on the socket.
    NotRunning,
    /// Something accepts, but it did not answer like a secure room.
    NotARoom,
}

/// An attached room connection right after its `hello`.
pub struct Opened {
    pub read: IpcReadHalf,
    pub frames: FrameReader,
    pub socket: RoomSocket,
    pub hello: Hello,
}

/// Connect to `path`, send `hello` and read the room's `hello`.
pub async fn open(path: &Path) -> Result<Opened, OpenError> {
    let stream = client::connect(path)
        .await
        .map_err(|_| OpenError::NotRunning)?;
    let (mut read, write) = split_stream(stream);
    let mut socket = RoomSocket { write };
    socket
        .send(&RoomClient::Hello)
        .await
        .map_err(|_| OpenError::NotARoom)?;
    let mut frames = FrameReader::new();
    let first = tokio::time::timeout(HELLO_WAIT, read_frame_from(&mut read, &mut frames))
        .await
        .map_err(|_| OpenError::NotARoom)?
        .map_err(|_| OpenError::NotARoom)?;
    let value: Value = serde_json::from_slice(&first).map_err(|_| OpenError::NotARoom)?;
    let hello = Hello::parse(&value).ok_or(OpenError::NotARoom)?;
    Ok(Opened {
        read,
        frames,
        socket,
        hello,
    })
}

/// The write half of a room connection.
pub struct RoomSocket {
    write: IpcWriteHalf,
}

impl RoomSocket {
    /// Write one frame. Only the frames the adapter may send exist as
    /// `RoomClient` values here; the callers never build `paste`, `set_model`,
    /// `open`, `browse` or `paste_image`.
    pub async fn send(&mut self, message: &RoomClient) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(message)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::time::timeout(BUDGET, write_frame_to(&mut self.write, &bytes))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "room write stalled"))?
    }
}

/// Why a room stream ended without a `closed` frame, decided by a fresh
/// connection whose first frame is a status request: the room answers it
/// without subscribing that connection.
/// - answered: another client took the stream over (`taken_over`);
/// - refused, or closed without an answer: the daemon is gone (`daemon_exit`);
/// - anything else, or no answer in time: `stream_error`.
pub async fn classify_end(path: &Path) -> &'static str {
    let probe = async {
        let Ok(stream) = client::connect(path).await else {
            return "daemon_exit";
        };
        let (mut read, mut write) = split_stream(stream);
        let Ok(bytes) = serde_json::to_vec(&ClientRequest::Status) else {
            return "stream_error";
        };
        if write_frame_to(&mut write, &bytes).await.is_err() {
            return "daemon_exit";
        }
        let mut frames = FrameReader::new();
        match read_frame_from(&mut read, &mut frames).await {
            Ok(bytes) => match serde_json::from_slice::<DaemonFrame>(&bytes) {
                Ok(DaemonFrame {
                    event: DaemonEvent::Status(_),
                    ..
                }) => "taken_over",
                _ => "stream_error",
            },
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => "daemon_exit",
            Err(_) => "stream_error",
        }
    };
    tokio::time::timeout(PROBE_WAIT, probe)
        .await
        .unwrap_or("stream_error")
}
