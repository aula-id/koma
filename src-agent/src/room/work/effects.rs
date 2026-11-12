//! What a coordinator tool call asks the rest of the room to do.
//!
//! The coordinator only decides and records. Starting lanes, steering them and
//! posting messages are the lane runner's job, so each call returns these as
//! data and the runner applies them after the transaction committed.

use super::model::Assignment;

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Start a work lane for this assignment. `goal` is the registered goal text.
    StartLane { assignment: Assignment, goal: String },
    /// Inject a steering message into the avatar's running lane (or hold it for
    /// the next turn if the lane is idle).
    Steer { avatar: String, text: String },
    /// A message every chat view should show. `nonce` makes posting idempotent.
    Visible { author: String, text: String, nonce: String },
}
