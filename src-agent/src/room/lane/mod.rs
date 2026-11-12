//! The lane layer: how avatars actually run.
//!
//! Every avatar has two loops. A **chat lane** takes short turns, answers the
//! user and the Chief, and never blocks on long work. A **work lane** runs the
//! long execution for one assignment. The actor (landing next) owns both and
//! connects them through three pure pieces built here:
//!
//! - [`inbox`] — the Chief's priority inbox, so an unread user message always
//!   leads and a coincident wake rides in the same turn.
//! - [`control`] — pause, resume, stop and redirect a lane.
//! - [`digest`] — a compact picture of the work lanes that each chat lane reads
//!   at turn start, so it can report progress without reading their chatter.
//!
//! [`event::LaneEvent`] is the vocabulary the lanes emit into all three.

pub mod control;
pub mod digest;
pub mod event;
pub mod inbox;
pub mod runner;
pub mod timeline;
pub mod transcript;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use control::{Action as ControlAction, Command as ControlCommand, ControlState};
#[allow(unused_imports)]
pub use digest::{Digest, LaneStatus, Phase};
#[allow(unused_imports)]
pub use crate::room::registry::LaneKind;
#[allow(unused_imports)]
pub use event::LaneEvent;
#[allow(unused_imports)]
pub use inbox::{Inbox, Item as InboxItem, Priority};
#[allow(unused_imports)]
pub use timeline::{Milestone, Timeline};
#[allow(unused_imports)]
pub use transcript::{lane_dir, Transcript};
#[allow(unused_imports)]
pub use runner::{
    run_lane, LaneConfig, LaneInput, ModelClient, ModelReply, ModelRequest, ToolRouter,
    ToolRun, ToolSpec, Usage,
};
