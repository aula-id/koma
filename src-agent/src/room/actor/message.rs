//! What flows into and out of the [`RoomActor`](super::RoomActor).
//!
//! The actor is a single-threaded state machine behind one command channel.
//! Everything — the user, the control channel, the tick, the lanes, and the
//! tools the lanes call — arrives as an [`ActorMsg`]. The actor's visible
//! output (the room's chat and status) leaves as a [`RoomEvent`].

use crate::room::lane::{ControlCommand, InboxItem, LaneEvent};
use crate::room::work::Effect;

/// One thing for the actor to handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ActorMsg {
    /// A message the user typed. `@avatar` addresses that member; anything else
    /// goes to the Chief. The clerk is not addressable.
    User(String),
    /// A message with images the user shared (pasted or named in the composer).
    UserWith { text: String, images: Vec<crate::room::lane::UserImage> },
    /// A control command (pause / resume / stop / redirect a lane).
    Control(ControlCommand),
    /// The periodic tick that drives the reconciler and the clerk.
    Tick,
    /// An event from a running lane, tagged with the avatar and which of its
    /// two lanes produced it. Chat and work for the same avatar run together.
    LaneEvent {
        avatar: String,
        kind: crate::room::lane::LaneKind,
        event: LaneEvent,
    },
    /// Coordinator effects from a chief tool call, for the actor to apply.
    Effects(Vec<Effect>),
    /// A clerk flag headed for the Chief's inbox.
    Flag(InboxItem),
    /// A lane called `message_user`.
    MessageUser { from: String, text: String },
    /// A client attached and wants the current picture now, not at the next
    /// tick. Emits one status and does nothing else.
    Resync,
    /// Stop the room.
    Shutdown,
}

/// What the room shows the user, drained by the UI or a headless driver.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomEvent {
    /// The Chief spoke to the user.
    Chief(String),
    /// A lane's visible narration (chat lanes only).
    LaneChat { avatar: String, text: String },
    /// A dispatch, result or finish line that every view should show.
    Visible { author: String, text: String },
    /// A piece of a lane's turn: thinking, a tool call, a tool result. The text
    /// itself still arrives as `Chief` or `LaneChat`.
    Part { author: String, lane: String, part: LanePart },
    /// A fresh status snapshot (digest, swim lanes, gate counts).
    Status(StatusSnapshot),
    /// The room is closing.
    Closed,
}

/// One structured piece of a lane's turn, in the order the lane produced it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LanePart {
    Reasoning { text: String },
    /// Text arriving live; the whole reply follows as `Chief` or `LaneChat`.
    TextDelta { text: String },
    /// Thinking arriving live; the whole of it follows as `Reasoning`.
    ReasoningDelta { text: String },
    ToolCall { id: String, name: String, args: String },
    ToolResult { id: String, ok: bool, result: String },
}

/// One lane in the status table: who, what they are doing, and how it is going.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaneInfo {
    pub avatar: String,
    pub display: String,
    pub role: String,
    /// `chat` or `work`; empty for an avatar with no lane running.
    pub lane: String,
    /// `running`, `idle`, `paused`, `submitted`, `done` or `failed`.
    pub phase: String,
    pub task: String,
    pub started_ms: u64,
    pub last_event_ms: u64,
    pub steps: usize,
    pub last_tool: String,
    pub error: String,
}

/// One thing that happened, for the timeline.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EventInfo {
    pub at_ms: u64,
    pub avatar: String,
    pub marker: String,
    pub label: String,
}

impl LanePart {
    /// A live piece that a late client does not need replayed.
    pub fn is_delta(&self) -> bool {
        matches!(self, LanePart::TextDelta { .. } | LanePart::ReasoningDelta { .. })
    }
}

/// A point-in-time picture of the room for the status panes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusSnapshot {
    /// The work-digest text (per-lane current state).
    pub digest: String,
    /// The swim-lane text (per-lane activity history).
    pub timeline: String,
    /// Open / verified / returned / reports.
    pub counts: crate::room::trajectory::gate::Counts,
    /// Avatars whose lane is currently paused.
    pub paused: Vec<String>,
    /// Shelf keys, one per line.
    pub shelf: String,
    /// Trajectory outline.
    pub trajectory: String,
    /// Every avatar's lanes as rows, for the status table.
    pub lanes: Vec<LaneInfo>,
    /// Recent milestones, newest first.
    pub events: Vec<EventInfo>,
    /// The room's clock when this was taken, so a client can show elapsed times.
    pub now_ms: u64,
}
