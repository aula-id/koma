//! The swim-lane view: who did what, in what order.
//!
//! The digest shows each lane's current state and the shelf shows what was
//! proved; neither keeps a history. This records a milestone per lane as the
//! events arrive, so the Secure UI can show the room's activity over time, one
//! row per avatar.
//!
//! It lives in the actor's memory (the shelf and the work tables are the
//! durable truth), so a restart starts the history fresh.

use std::collections::BTreeMap;

use super::event::LaneEvent;

/// One thing a lane did, in shape-carrying glyphs so the row reads without
/// colour: `◆` opened, `○` published, `✔` submitted, `✕` failed, `⚑` flagged.
#[derive(Debug, Clone, PartialEq)]
pub struct Milestone {
    pub at_ms: u64,
    pub marker: char,
    pub label: String,
}

/// Activity history, one row per avatar in first-seen order.
#[derive(Debug, Default)]
pub struct Timeline {
    lanes: BTreeMap<String, Vec<Milestone>>,
    order: Vec<String>,
    start_ms: Option<u64>,
}

impl Timeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn push(&mut self, avatar: &str, at_ms: u64, marker: char, label: String) {
        if !self.lanes.contains_key(avatar) {
            self.order.push(avatar.to_string());
        }
        self.lanes.entry(avatar.to_string()).or_default().push(Milestone { at_ms, marker, label });
        self.start_ms = Some(self.start_ms.map_or(at_ms, |s| s.min(at_ms)));
    }

    /// Open a lane's row with what it is doing (its kind, "chat" or "clerk").
    pub fn open(&mut self, avatar: &str, label: &str, at_ms: u64) {
        self.push(avatar, at_ms, '◆', label.to_string());
    }

    /// Fold a lane event into a milestone. Routine events (steps, tool starts,
    /// chat chunks, a plain finish) add nothing to the activity trail.
    pub fn record(&mut self, avatar: &str, event: &LaneEvent, at_ms: u64) {
        match event {
            LaneEvent::Published { key, .. } => self.push(avatar, at_ms, '○', last_segment(key)),
            LaneEvent::Submitted => self.push(avatar, at_ms, '✔', "submit".to_string()),
            LaneEvent::Failed { error } => self.push(avatar, at_ms, '✕', first_clause(error)),
            LaneEvent::Flag { note, .. } => self.push(avatar, at_ms, '⚑', first_clause(note)),
            _ => {}
        }
    }

    /// The most recent milestones across every lane, newest first, as
    /// `(at_ms, avatar, marker, label)`.
    pub fn events(&self, limit: usize) -> Vec<(u64, String, char, String)> {
        let mut all: Vec<(u64, String, char, String)> = self
            .lanes
            .iter()
            .flat_map(|(avatar, marks)| marks.iter().map(move |m| (m.at_ms, avatar.clone(), m.marker, m.label.clone())))
            .collect();
        all.sort_by(|a, b| b.0.cmp(&a.0));
        all.truncate(limit);
        all
    }

    /// Drop a lane's row (e.g. a superseded assignment).
    pub fn forget(&mut self, avatar: &str) {
        if self.lanes.remove(avatar).is_some() {
            self.order.retain(|a| a != avatar);
        }
    }

    /// Render the swim lanes, each row truncated to `width`. Milestones read
    /// left to right in the order they happened. Empty string when nothing has
    /// happened.
    pub fn render(&self, width: usize) -> String {
        if self.order.is_empty() {
            return String::new();
        }
        let name_width = self.order.iter().map(|a| a.chars().count()).max().unwrap_or(0);
        let body_width = width.saturating_sub(name_width + 2).max(8);
        let mut out = String::new();
        for avatar in &self.order {
            let trail: Vec<String> = self.lanes[avatar].iter().map(|m| format!("{} {}", m.marker, m.label)).collect();
            let body = clip(&trail.join("  ──  "), body_width);
            out.push_str(&format!("{avatar:<name_width$}  {body}\n"));
        }
        out
    }
}

fn last_segment(key: &str) -> String {
    key.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(key).to_string()
}

/// The first sentence or line of a message, for a compact milestone label.
fn first_clause(s: &str) -> String {
    let head = s.split(['.', '\n', ';']).next().unwrap_or(s).trim();
    clip(if head.is_empty() { s.trim() } else { head }, 40)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}
