//! What a lane emits as it runs. The actor tags each event with the lane it
//! came from, folds it into the [`Digest`](super::digest::Digest), and turns
//! the ones the Chief must see into inbox items.

/// One progress event from a chat or work lane.
#[derive(Debug, Clone, PartialEq)]
pub enum LaneEvent {
    /// About to make the n-th model call (0-based).
    Step(usize),
    /// A tool call is starting.
    ToolStarted { name: String },
    /// A tool call finished; `ok` is false on an error or blocked result.
    ToolDone { name: String, ok: bool },
    /// This lane wrote a shelf key at this revision (from a `put_*` call).
    Published { key: String, revision: i64 },
    /// A chunk of chat text headed for the user (chat lanes only).
    Chat(String),
    /// The lane submitted its assignment (`group_task_complete` passed).
    Submitted,
    /// The Clerk raised a flag to the Chief (`clerk_flag`). Only the clerk lane
    /// produces this; the actor turns it into a Chief inbox item.
    Flag { severity: String, note: String },
    /// The lane ended; `report` is its final text.
    Finished { report: String },
    /// The lane failed fatally; `error` is the cause.
    Failed { error: String },
}

impl LaneEvent {
    /// A shelf key this event published, if any.
    pub fn published_key(&self) -> Option<&str> {
        match self {
            LaneEvent::Published { key, .. } => Some(key),
            _ => None,
        }
    }
}
