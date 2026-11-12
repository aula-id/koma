//! The work digest: a compact picture of what the lanes are doing, folded from
//! their events and prepended to each chat lane's turn.
//!
//! This is the other half of the chat/work split. A chat lane answers the user
//! without reading the work lanes' chatter; instead it reads this digest, so it
//! can say how far along the work is without interrupting it.

use std::collections::BTreeMap;

use super::event::LaneEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// `group_task_complete` passed; waiting for the lane to end.
    Submitted,
    /// The lane ended cleanly.
    Done,
    Failed,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Running => "running",
            Phase::Submitted => "submitted",
            Phase::Done => "done",
            Phase::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaneStatus {
    pub phase: Phase,
    pub steps: usize,
    pub last_tool: Option<String>,
    /// Shelf keys this lane wrote, in first-seen order, deduplicated.
    pub published: Vec<String>,
    pub error: Option<String>,
}

impl Default for LaneStatus {
    fn default() -> Self {
        Self { phase: Phase::Running, steps: 0, last_tool: None, published: Vec::new(), error: None }
    }
}

/// Per-lane status, keyed by lane id (the avatar). Rendered in key order so the
/// digest text is deterministic.
#[derive(Debug, Default)]
pub struct Digest {
    lanes: BTreeMap<String, LaneStatus>,
}

impl Digest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, avatar: &str) -> Option<&LaneStatus> {
        self.lanes.get(avatar)
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// Drop a lane once it has been retired (e.g. superseded or reaped).
    pub fn forget(&mut self, avatar: &str) {
        self.lanes.remove(avatar);
    }

    /// Fold one event from `avatar`'s lane into the picture.
    pub fn observe(&mut self, avatar: &str, event: &LaneEvent) {
        let lane = self.lanes.entry(avatar.to_string()).or_default();
        match event {
            LaneEvent::Step(n) => {
                lane.steps = lane.steps.max(n + 1);
                if lane.phase == Phase::Submitted {
                    // a lane may keep publishing after it submits; stay submitted
                } else {
                    lane.phase = Phase::Running;
                }
            }
            LaneEvent::ToolStarted { name } => {
                lane.last_tool = Some(name.clone());
            }
            LaneEvent::ToolDone { .. } => {}
            LaneEvent::Published { key, .. } => {
                if !lane.published.iter().any(|k| k == key) {
                    lane.published.push(key.clone());
                }
            }
            LaneEvent::Chat(_) => {}
            LaneEvent::Submitted => {
                lane.phase = Phase::Submitted;
            }
            LaneEvent::Finished { .. } => {
                if lane.phase != Phase::Failed {
                    lane.phase = Phase::Done;
                }
            }
            LaneEvent::Failed { error } => {
                lane.phase = Phase::Failed;
                lane.error = Some(error.clone());
            }
        }
    }

    /// A compact, line-per-lane summary to prepend to a chat-lane turn. Empty
    /// string when no lane has run.
    pub fn render(&self) -> String {
        if self.lanes.is_empty() {
            return String::new();
        }
        let mut out = String::from("Work lanes:\n");
        for (avatar, lane) in &self.lanes {
            out.push_str(&format!("  {avatar}: {} (step {})", lane.phase.label(), lane.steps));
            if lane.phase == Phase::Failed {
                if let Some(error) = &lane.error {
                    out.push_str(&format!(" — {}", first_line(error)));
                }
            } else if let Some(tool) = &lane.last_tool {
                out.push_str(&format!(" — last: {tool}"));
            }
            if !lane.published.is_empty() {
                out.push_str(&format!("; published: {}", lane.published.join(", ")));
            }
            out.push('\n');
        }
        out
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}
