//! The secure daemon's socket: one listener on `run/<id>.sock`.
//!
//! A hub probe (`ClientRequest::Status`) gets the coding `DaemonEvent::Status`
//! and nothing else. An attach client speaks [`crate::room::wire`] and receives
//! the actor's events. The coding [`crate::ipc::server::accept_loop`] is not
//! used here: it would hand the socket to the coding hub.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::{split_stream, IpcListener, IpcWriteHalf};
use crate::room::actor::{ActorMsg, RoomEvent};
use crate::room::wire::{RoomClient, RoomServer};

/// Accept connections until the listener is dropped. `events` is the actor's
/// receiver; the latest attach client is the one that receives them.
pub async fn run(
    listener: IpcListener,
    to_actor: UnboundedSender<ActorMsg>,
    mut events: UnboundedReceiver<RoomEvent>,
    status: SessionStatus,
    shutting_down: Arc<AtomicBool>,
) {
    let (sub_tx, mut sub_rx) = unbounded_channel();
    let mut subscriber: Option<UnboundedSender<RoomServer>> = None;
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
                if let Some(tx) = &subscriber {
                    if tx.send(frame).is_err() {
                        subscriber = None;
                    }
                }
            }
            sub = sub_rx.recv() => {
                subscriber = sub;
            }
        }
    }
}

async fn handle_conn(
    stream: crate::ipc::IpcStream,
    to_actor: UnboundedSender<ActorMsg>,
    sub_tx: UnboundedSender<UnboundedSender<RoomServer>>,
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
    let _ = sub_tx.send(tx);
    write_json(&mut write, &RoomServer::hello()).await?;
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
