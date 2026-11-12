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
use crate::room::wire::{RoomClient, RoomServer};

/// Spoken lines kept for a client that attaches late.
pub const BACKLOG_CAP: usize = 500;

/// What a client that attaches now has missed.
struct Replay {
    backlog: VecDeque<RoomServer>,
    status: Option<RoomServer>,
    cap: usize,
}

impl Replay {
    fn new(cap: usize) -> Self {
        Self { backlog: VecDeque::new(), status: None, cap }
    }

    fn record(&mut self, frame: &RoomServer) {
        if frame.is_history() {
            if self.backlog.len() >= self.cap {
                self.backlog.pop_front();
            }
            self.backlog.push_back(frame.clone());
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
) {
    serve(listener, to_actor, events, status, shutting_down, BACKLOG_CAP).await;
}

async fn serve(
    listener: IpcListener,
    to_actor: UnboundedSender<ActorMsg>,
    mut events: UnboundedReceiver<RoomEvent>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
    cap: usize,
) {
    let (sub_tx, mut sub_rx) = unbounded_channel::<Subscription>();
    let mut subscriber: Option<UnboundedSender<RoomServer>> = None;
    let mut replay = Replay::new(cap);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let to_actor = to_actor.clone();
                        let sub_tx = sub_tx.clone();
                        let status = status.clone();
                        let shutting_down = Arc::clone(&shutting_down);
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, to_actor, sub_tx, status, shutting_down).await;
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

async fn handle_conn(
    stream: crate::ipc::IpcStream,
    to_actor: UnboundedSender<ActorMsg>,
    sub_tx: UnboundedSender<Subscription>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
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
                            let _ = to_actor.send(ActorMsg::User(text));
                        }
                    }
                    RoomClient::Control { command } => {
                        let _ = to_actor.send(ActorMsg::Control(command.into()));
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
}
