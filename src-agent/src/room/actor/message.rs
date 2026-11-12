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
    /// A fresh status snapshot (digest, swim lanes, gate counts).
    Status(StatusSnapshot),
    /// The room is closing.
    Closed,
}

/// A point-in-time picture of the room for the status panes.
#[derive(Debug, Clone, PartialEq)]
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
}
