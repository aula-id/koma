//! The secure daemon's socket: one listener on `run/<id>.sock`.
//!
//! A hub probe (`ClientRequest::Status`) gets the coding `DaemonEvent::Status`
//! and nothing else. An attach client speaks [`crate::room::wire`] and receives
//! the actor's events. The coding [`crate::ipc::server::accept_loop`] is not
//! used here: it would hand the socket to the coding hub.
//!
//! The room keeps talking while nobody is attached (a handoff, a dropped
//! terminal), so the loop remembers the latest status and the last
//! [`BACKLOG_CAP`] spoken lines, and replays them to whoever attaches next.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::{split_stream, IpcListener, IpcWriteHalf};
use crate::room::actor::{ActorMsg, RoomEvent};
use super::models::ModelBook;
use super::stage::{Pasted, Staging};
use crate::room::wire::{RoomClient, RoomServer};

/// Spoken lines kept for a client that attaches late.
pub const BACKLOG_CAP: usize = 2000;

/// Where a room keeps what it has said, so a room entered again shows its past.
pub const LOG_FILE: &str = "room-log.jsonl";

/// What a client that attaches now has missed.
struct Replay {
    backlog: VecDeque<RoomServer>,
    status: Option<RoomServer>,
    cap: usize,
    /// The file the backlog is kept in, and how many lines it holds.
    log: Option<(std::path::PathBuf, usize)>,
}

impl Replay {
    fn new(cap: usize) -> Self {
        Self { backlog: VecDeque::new(), status: None, cap, log: None }
    }

    /// A backlog kept in `path`: what was said before is loaded, and what is said now is appended.
    fn with_log(cap: usize, path: std::path::PathBuf) -> Self {
        let mut replay = Self::new(cap);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            lines += 1;
            if let Ok(frame) = serde_json::from_str::<RoomServer>(line) {
                if frame.is_history() {
                    if replay.backlog.len() >= cap {
                        replay.backlog.pop_front();
                    }
                    replay.backlog.push_back(frame);
                }
            }
        }
        replay.log = Some((path, lines));
        replay
    }

    fn append(&mut self, frame: &RoomServer) {
        let Some((path, lines)) = &mut self.log else { return };
        let Ok(line) = serde_json::to_string(frame) else { return };
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&*path) {
            let _ = writeln!(file, "{line}");
            *lines += 1;
        }
        // Keep the file near the size of the backlog.
        if *lines > self.cap * 3 {
            let mut text = String::new();
            for frame in &self.backlog {
                if let Ok(line) = serde_json::to_string(frame) {
                    text.push_str(&line);
                    text.push('\n');
                }
            }
            if std::fs::write(&*path, text).is_ok() {
                *lines = self.backlog.len();
            }
        }
    }

    fn record(&mut self, frame: &RoomServer) {
        if frame.is_history() {
            if self.backlog.len() >= self.cap {
                self.backlog.pop_front();
            }
            self.backlog.push_back(frame.clone());
            self.append(frame);
        } else if matches!(frame, RoomServer::Status { .. }) {
            self.status = Some(frame.clone());
        }
    }

    /// The latest status first, then the lines in the order they were said.
    fn frames(&self) -> Vec<RoomServer> {
        self.status.iter().chain(self.backlog.iter()).cloned().collect()
    }
}

/// A new attach client: where to send frames, and where to learn how many of
/// the first ones are history.
struct Subscription {
    tx: UnboundedSender<RoomServer>,
    replayed: oneshot::Sender<usize>,
}

/// Accept connections until the listener is dropped. `events` is the actor's
/// receiver; the latest attach client is the one that receives them.
pub async fn run(
    listener: IpcListener,
    to_actor: UnboundedSender<ActorMsg>,
    events: UnboundedReceiver<RoomEvent>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
    book: Arc<ModelBook>,
    stage: Arc<Staging>,
) {
    serve(listener, to_actor, events, status, shutting_down, book, stage, BACKLOG_CAP).await;
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    listener: IpcListener,
    to_actor: UnboundedSender<ActorMsg>,
    mut events: UnboundedReceiver<RoomEvent>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
    book: Arc<ModelBook>,
    stage: Arc<Staging>,
    cap: usize,
) {
    let (sub_tx, mut sub_rx) = unbounded_channel::<Subscription>();
    // Lines the user typed are kept for replay without being sent back to the client that typed them.
    let (note_tx, mut note_rx) = unbounded_channel::<RoomServer>();
    let mut subscriber: Option<UnboundedSender<RoomServer>> = None;
    let mut replay = Replay::with_log(cap, stage.dir().join(LOG_FILE));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let to_actor = to_actor.clone();
                        let sub_tx = sub_tx.clone();
                        let status = status.clone();
                        let shutting_down = Arc::clone(&shutting_down);
                        let book = Arc::clone(&book);
                        let stage = Arc::clone(&stage);
                        let note_tx = note_tx.clone();
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, to_actor, sub_tx, status, shutting_down, book, stage, note_tx).await;
                        });
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            event = events.recv() => {
                let Some(event) = event else { break };
                let frame = RoomServer::from_event(event);
                replay.record(&frame);
                if let Some(tx) = &subscriber {
                    if tx.send(frame).is_err() {
                        subscriber = None;
                    }
                }
            }
            note = note_rx.recv() => {
                if let Some(note) = note {
                    replay.record(&note);
                }
            }
            sub = sub_rx.recv() => {
                let Some(Subscription { tx, replayed }) = sub else { continue };
                let frames = replay.frames();
                let count = frames.len();
                let delivered = frames.into_iter().all(|frame| tx.send(frame).is_ok());
                let _ = replayed.send(count);
                subscriber = delivered.then_some(tx);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_conn(
    stream: crate::ipc::IpcStream,
    to_actor: UnboundedSender<ActorMsg>,
    sub_tx: UnboundedSender<Subscription>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
    book: Arc<ModelBook>,
    stage: Arc<Staging>,
    note_tx: UnboundedSender<RoomServer>,
) -> std::io::Result<()> {
    let (mut read, mut write) = split_stream(stream);
    let mut reader = FrameReader::new();
    let first = read_frame_from(&mut read, &mut reader).await?;
    let value: serde_json::Value = serde_json::from_slice(&first)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Ok(ClientRequest::Status) = serde_json::from_value(value.clone()) {
        let frame = DaemonFrame { seq: 1, event: DaemonEvent::Status(status) };
        return write_json(&mut write, &frame).await;
    }
    if let Ok(ClientRequest::QuitDaemon) = serde_json::from_value(value.clone()) {
        shutting_down.store(true, Ordering::Relaxed);
        return Ok(());
    }
    let Ok(RoomClient::Hello) = serde_json::from_value(value) else {
        return Ok(());
    };
    let (tx, mut rx) = unbounded_channel();
    let (replayed_tx, replayed_rx) = oneshot::channel();
    let _ = sub_tx.send(Subscription { tx, replayed: replayed_tx });
    let replayed = replayed_rx.await.unwrap_or(0);
    write_json(&mut write, &RoomServer::hello(replayed)).await?;
    // The replayed status can be minutes old: ask for a current one.
    let _ = to_actor.send(ActorMsg::Resync);
    loop {
        tokio::select! {
            incoming = read_frame_from(&mut read, &mut reader) => {
                let bytes = match incoming {
                    Ok(bytes) => bytes,
                    Err(_) => break,
                };
                let Ok(message) = serde_json::from_slice::<RoomClient>(&bytes) else {
                    break;
                };
                match message {
                    RoomClient::Hello => {}
                    RoomClient::User { text } => {
                        if !text.trim().is_empty() {
                            let _ = note_tx.send(RoomServer::Visible { author: "you".into(), text: text.clone() });
                            let (text, images) = stage.take(&text);
                            let _ = to_actor.send(if images.is_empty() {
                                ActorMsg::User(text)
                            } else {
                                ActorMsg::UserWith { text, images }
                            });
                        }
                    }
                    RoomClient::Paste { text } => {
                        write_json(&mut write, &pasted_frame(stage.paste_text(&text))).await?;
                    }
                    RoomClient::PasteImage => {
                        let clipboard = tokio::task::spawn_blocking(crate::controller::input::read_clipboard_png).await;
                        let pasted = match clipboard {
                            Ok(Ok(bytes)) => stage.paste_image(&bytes),
                            Ok(Err(reason)) => Pasted::Refused(format!("clipboard image: {reason}")),
                            Err(_) => Pasted::Refused("clipboard image: the read was interrupted".into()),
                        };
                        write_json(&mut write, &pasted_frame(pasted)).await?;
                    }
                    RoomClient::Browse => {
                        let _ = to_actor.send(ActorMsg::Browse);
                    }
                    RoomClient::Open { key } => {
                        let _ = to_actor.send(ActorMsg::Open { key });
                    }
                    RoomClient::Interrupt => {
                        let _ = to_actor.send(ActorMsg::Interrupt);
                    }
                    RoomClient::Control { command } => {
                        let _ = to_actor.send(ActorMsg::Control(command.into()));
                    }
                    RoomClient::Models => {
                        write_json(&mut write, &models_frame(&book)).await?;
                    }
                    RoomClient::SetModel { avatar, lane, model } => {
                        if let Err(text) = book.set(&avatar, &lane, &model) {
                            write_json(&mut write, &RoomServer::Notice { text }).await?;
                        }
                        write_json(&mut write, &models_frame(&book)).await?;
                    }
                    RoomClient::Quit => {
                        let _ = write_json(&mut write, &RoomServer::Closed).await;
                        break;
                    }
                }
            }
            event = rx.recv() => {
                let Some(frame) = event else { break };
                let closed = matches!(frame, RoomServer::Closed);
                if write_json(&mut write, &frame).await.is_err() || closed {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn pasted_frame(pasted: Pasted) -> RoomServer {
    match pasted {
        Pasted::Insert(insert) => RoomServer::Pasted { insert },
        Pasted::Refused(text) => RoomServer::Notice { text },
    }
}

fn models_frame(book: &ModelBook) -> RoomServer {
    RoomServer::Models { choices: book.choices(), lanes: book.lanes() }
}

async fn write_json(write: &mut IpcWriteHalf, value: &impl serde::Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_frame_to(write, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chief(text: &str) -> RoomServer {
        RoomServer::Chief { text: text.into() }
    }

    fn status(open: usize) -> RoomServer {
        RoomServer::Status {
            digest: String::new(),
            timeline: String::new(),
            open,
            verified: 0,
            returned: 0,
            reports: 0,
            paused: Vec::new(),
            shelf: String::new(),
            trajectory: String::new(),
            lanes: Vec::new(),
            events: Vec::new(),
            now_ms: 0,
        }
    }

    #[test]
    fn a_full_backlog_drops_the_oldest_line() {
        let mut replay = Replay::new(3);
        for i in 0..5 {
            replay.record(&chief(&format!("line {i}")));
        }
        assert_eq!(replay.frames(), vec![chief("line 2"), chief("line 3"), chief("line 4")]);
    }

    #[test]
    fn only_the_latest_status_is_kept_and_it_leads_the_replay() {
        let mut replay = Replay::new(10);
        replay.record(&chief("hello"));
        replay.record(&status(1));
        replay.record(&status(2));
        assert_eq!(replay.frames(), vec![status(2), chief("hello")]);
    }

    #[test]
    fn closed_is_not_history() {
        let mut replay = Replay::new(10);
        replay.record(&RoomServer::Closed);
        assert!(replay.frames().is_empty());
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koma-replay-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn said(text: &str) -> RoomServer {
        RoomServer::Visible { author: "you".into(), text: text.into() }
    }

    #[test]
    fn what_a_room_said_is_there_when_it_is_entered_again() {
        let path = dir("again").join(LOG_FILE);
        let mut first = Replay::with_log(10, path.clone());
        first.record(&said("scan the login"));
        first.record(&RoomServer::Chief { text: "On it.".into() });
        first.record(&RoomServer::Closed); // not history
        let second = Replay::with_log(10, path);
        assert_eq!(second.frames(), vec![said("scan the login"), RoomServer::Chief { text: "On it.".into() }]);
    }

    #[test]
    fn the_log_keeps_the_latest_lines_and_does_not_grow_without_end() {
        let path = dir("cap").join(LOG_FILE);
        let mut replay = Replay::with_log(3, path.clone());
        for n in 0..40 {
            replay.record(&said(&format!("line {n}")));
        }
        let reopened = Replay::with_log(3, path.clone());
        assert_eq!(reopened.frames(), vec![said("line 37"), said("line 38"), said("line 39")]);
        assert!(std::fs::read_to_string(&path).unwrap().lines().count() <= 10, "the file was compacted");
    }
}
