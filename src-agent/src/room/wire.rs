//! Frames for the secure daemon socket.
//!
//! The length prefix is the coding daemon's. The JSON is not: room messages are
//! not variants of [`crate::ipc::proto::ClientRequest`]. A hub that asks for
//! [`ClientRequest::Status`] still gets a [`DaemonEvent::Status`] back, so a
//! secure session shows up in the session list.

use serde::{Deserialize, Serialize};

use crate::room::actor::{EventInfo, LaneInfo, LanePart, RoomEvent, StatusSnapshot};
use crate::room::lane::ControlCommand;

/// What an attach client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomClient {
    /// The client's first frame. The daemon answers with [`RoomServer::Hello`],
    /// replays recent history, then sends a fresh status.
    Hello,
    User { text: String },
    /// Text the user pasted into the composer. The daemon decides what it becomes.
    Paste { text: String },
    /// Ctrl+V: take an image from the clipboard.
    PasteImage,
    /// Esc: stop what the Chief and the specialists are saying. Their work keeps running.
    Interrupt,
    /// Open a browse screen: the shelf, the goals and the reports.
    Browse,
    /// Open one shelf item in full.
    Open { key: String },
    Control { command: RoomControl },
    /// Ask for the registered models and what each lane runs.
    Models,
    /// Point one lane (`chat` or `work`) at a registered model by its id. An
    /// empty id returns the lane to the session default.
    SetModel { avatar: String, lane: String, model: String },
    Quit,
}

/// A model registered in the coding agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelChoice {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub provider: String,
}

/// The model one lane runs, as a label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneRow {
    pub avatar: String,
    pub display: String,
    pub lane: String,
    pub model: String,
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
    /// What a paste turned into: text for the draft at the caret.
    Pasted { insert: String },
    /// The shelf, goals and reports, answering [`RoomClient::Browse`].
    Browse { shelf: Vec<crate::room::browse::ShelfRow>, goals: Vec<crate::room::browse::Goal>, reports: Vec<crate::room::browse::Report> },
    /// One shelf item, answering [`RoomClient::Open`].
    Item { key: String, revision: i64, updated: i64, content: String },
    Chief { text: String },
    LaneChat { avatar: String, text: String },
    Visible { author: String, text: String },
    /// Thinking, a tool call or a tool result from one lane. `lane` is `chat` or `work`.
    Part { author: String, lane: String, part: LanePart },
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
        #[serde(default)]
        lanes: Vec<LaneInfo>,
        #[serde(default)]
        events: Vec<EventInfo>,
        #[serde(default)]
        now_ms: u64,
    },
    /// The answer to [`RoomClient::Models`], and to every `SetModel`.
    Models { choices: Vec<ModelChoice>, lanes: Vec<LaneRow> },
    /// A one-line result for the client that asked, such as a refused model.
    Notice { text: String },
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
            || matches!(self, RoomServer::Part { part, .. } if !part.is_delta())
    }

    pub fn from_event(event: RoomEvent) -> Self {
        match event {
            RoomEvent::Chief(text) => RoomServer::Chief { text },
            RoomEvent::LaneChat { avatar, text } => RoomServer::LaneChat { avatar, text },
            RoomEvent::Visible { author, text } => RoomServer::Visible { author, text },
            RoomEvent::Part { author, lane, part } => RoomServer::Part { author, lane, part },
            RoomEvent::Status(status) => Self::from_status(status),
            RoomEvent::Browse(b) => RoomServer::Browse { shelf: b.shelf, goals: b.goals, reports: b.reports },
            RoomEvent::Item(i) => RoomServer::Item { key: i.key, revision: i.revision, updated: i.updated, content: i.content },
            RoomEvent::Notice(text) => RoomServer::Notice { text },
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
            lanes: status.lanes,
            events: status.events,
            now_ms: status.now_ms,
        }
    }
}
