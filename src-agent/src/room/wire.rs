//! Frames for the secure daemon socket.
//!
//! The length prefix is the coding daemon's. The JSON is not: room messages are
//! not variants of [`crate::ipc::proto::ClientRequest`]. A hub that asks for
//! [`ClientRequest::Status`] still gets a [`DaemonEvent::Status`] back, so a
//! secure session shows up in the session list.

use serde::{Deserialize, Serialize};

use crate::room::actor::{RoomEvent, StatusSnapshot};
use crate::room::lane::ControlCommand;

/// What an attach client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomClient {
    /// The client's first frame. The daemon answers with [`RoomServer::Hello`],
    /// replays recent history, then sends a fresh status.
    Hello,
    User { text: String },
    Control { command: RoomControl },
    Quit,
}

/// What the daemon sends to an attach client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomServer {
    Hello {
        kind: String,
        /// How many history frames follow before live ones: the latest status
        /// and the recent Chief, lane and visible lines.
        #[serde(default)]
        replayed: usize,
    },
    Chief { text: String },
    LaneChat { avatar: String, text: String },
    Visible { author: String, text: String },
    Status {
        digest: String,
        timeline: String,
        open: usize,
        verified: usize,
        returned: usize,
        reports: usize,
        paused: Vec<String>,
        #[serde(default)]
        shelf: String,
        #[serde(default)]
        trajectory: String,
    },
    Closed,
}

/// A control line, in the same shape the actor already applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomControl {
    Pause { avatar: String },
    Resume { avatar: String },
    Stop { avatar: String },
    Redirect { avatar: String, note: String },
}

impl From<RoomControl> for ControlCommand {
    fn from(command: RoomControl) -> Self {
        match command {
            RoomControl::Pause { avatar } => ControlCommand::Pause { avatar },
            RoomControl::Resume { avatar } => ControlCommand::Resume { avatar },
            RoomControl::Stop { avatar } => ControlCommand::Stop { avatar },
            RoomControl::Redirect { avatar, note } => ControlCommand::Redirect { avatar, note },
        }
    }
}

impl RoomServer {
    pub fn hello(replayed: usize) -> Self {
        RoomServer::Hello { kind: "secure".into(), replayed }
    }

    /// True for the lines a late client should still see: what the Chief and the
    /// lanes said. Status is replaced by the next snapshot and Closed ends the
    /// connection, so neither is kept as history.
    pub fn is_history(&self) -> bool {
        matches!(self, RoomServer::Chief { .. } | RoomServer::LaneChat { .. } | RoomServer::Visible { .. })
    }

    pub fn from_event(event: RoomEvent) -> Self {
        match event {
            RoomEvent::Chief(text) => RoomServer::Chief { text },
            RoomEvent::LaneChat { avatar, text } => RoomServer::LaneChat { avatar, text },
            RoomEvent::Visible { author, text } => RoomServer::Visible { author, text },
            RoomEvent::Status(status) => Self::from_status(status),
            RoomEvent::Closed => RoomServer::Closed,
        }
    }

    fn from_status(status: StatusSnapshot) -> Self {
        RoomServer::Status {
            digest: status.digest,
            timeline: status.timeline,
            open: status.counts.open,
            verified: status.counts.verified,
            returned: status.counts.returned,
            reports: status.counts.reports,
            paused: status.paused,
            shelf: status.shelf,
            trajectory: status.trajectory,
        }
    }
}
