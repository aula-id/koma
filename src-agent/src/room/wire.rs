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
    /// A whole message with its attachments, answered by [`RoomAck::SdkAck`]
    /// with the same `id`. The attachments are composed for this message alone;
    /// nothing staged by [`RoomClient::Paste`] is used or emptied.
    SdkUser {
        id: String,
        text: String,
        #[serde(default)]
        attachments: Vec<SdkAttachment>,
    },
    /// Re-read the room's registry files, answered by [`RoomAck::SdkAck`]
    /// with the registry version and its load warnings.
    SdkReload { id: String },
}

/// One attachment of an [`RoomClient::SdkUser`] message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SdkAttachment {
    /// An image file, by absolute path. The daemon copies it into the room.
    ImageFile { path: String },
    /// Pasted text: long or multi-line text is kept on disk as a chip, short
    /// text goes into the message inline.
    PastedText { text: String },
}

/// The frames this daemon understands beyond the terminal's, as listed in
/// [`RoomServer::Hello`].
pub const SDK_FEATURES: &[&str] = &["sdk_user", "sdk_reload"];

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
        /// The optional frames this daemon understands ([`SDK_FEATURES`]).
        /// Older daemons send none.
        #[serde(default)]
        features: Vec<String>,
        /// The daemon's build fingerprint. Empty from older daemons.
        #[serde(default)]
        version: String,
        /// The revision of the template layer the room read at start, or
        /// `None` when it was started without one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template_revision: Option<String>,
    },
    /// What a paste turned into: text for the draft at the caret.
    Pasted { insert: String },
    /// The shelf, goals and reports, answering [`RoomClient::Browse`].
    Browse { shelf: Vec<crate::room::browse::ShelfRow>, goals: Vec<crate::room::browse::Goal>, reports: Vec<crate::room::browse::Report> },
    /// One shelf item, answering [`RoomClient::Open`].
    Item { key: String, revision: i64, updated: i64, content: String },
    Chief { text: String },
    LaneChat { avatar: String, text: String },
    Visible {
        author: String,
        text: String,
        /// Older saved lines have none; the screen then works it out from the author.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience: Option<crate::room::work::Audience>,
    },
    /// Thinking, a tool call or a tool result from one lane. `lane` is `chat` or `work`.
    Part {
        author: String,
        lane: String,
        part: LanePart,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience: Option<crate::room::work::Audience>,
    },
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

/// The answer to [`RoomClient::SdkUser`] and [`RoomClient::SdkReload`], sent
/// only to the client that asked: `{"sdk_ack":{...}}`. It is a type of its own,
/// not a [`RoomServer`] variant, so it can never be kept as history, replayed or
/// written to the room log, and a client that never sends those frames never
/// receives one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomAck {
    SdkAck {
        id: String,
        ok: bool,
        /// A short machine-readable reason when `ok` is false.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// What each attachment became in the message, in order: a marker
        /// such as `[Image #3]`, or a short paste's own text.
        #[serde(default)]
        inserts: Vec<String>,
        /// The registry version after a reload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<u64>,
        /// The registry's load warnings after a reload.
        #[serde(default)]
        warnings: Vec<String>,
    },
}

impl RoomAck {
    /// A delivered [`RoomClient::SdkUser`]: what each attachment became.
    pub fn sent(id: String, inserts: Vec<String>) -> Self {
        RoomAck::SdkAck { id, ok: true, code: None, error: None, inserts, version: None, warnings: Vec::new() }
    }

    /// A refused request: nothing was delivered or changed.
    pub fn refused(id: String, code: String, error: String) -> Self {
        RoomAck::SdkAck {
            id,
            ok: false,
            code: Some(code),
            error: Some(error),
            inserts: Vec::new(),
            version: None,
            warnings: Vec::new(),
        }
    }

    /// A finished [`RoomClient::SdkReload`].
    pub fn reloaded(id: String, version: u64, warnings: Vec<String>) -> Self {
        RoomAck::SdkAck { id, ok: true, code: None, error: None, inserts: Vec::new(), version: Some(version), warnings }
    }
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
    /// The handshake: how many history frames follow, the optional frames
    /// this daemon understands, its build and its template revision.
    pub fn hello(replayed: usize, version: String, template_revision: Option<String>) -> Self {
        RoomServer::Hello {
            kind: "secure".into(),
            replayed,
            features: SDK_FEATURES.iter().map(|f| f.to_string()).collect(),
            version,
            template_revision,
        }
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
            RoomEvent::Visible { author, text, audience } => RoomServer::Visible { author, text, audience: Some(audience) },
            RoomEvent::Part { author, lane, part, audience } => RoomServer::Part { author, lane, part, audience: Some(audience) },
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
