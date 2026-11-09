//! Lane changes requested by the model through `sec_lane`.
//!
//! A tool call cannot reach `&mut AppState`, and a `ToolCtx` resolves its lane once
//! per turn. So the tool records the request here, keyed by session id:
//!
//! - the gate reads it at once ([`override_for`]), so the rest of the turn runs
//!   under the new lane;
//! - the event loop persists it through the same `apply_lane` path the `/security`
//!   panel uses ([`drain_unapplied`]);
//! - the next turn's context reads the saved setting and drops the entry
//!   ([`clear_applied`]).

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::app::sec::lane::Lane;

struct Entry {
    lane: Option<Lane>,
    applied: bool,
}

static BOARD: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());

fn board() -> std::sync::MutexGuard<'static, BTreeMap<String, Entry>> {
    BOARD.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Record the model's choice for `session_id`. `None` clears the lane.
pub fn request(session_id: &str, lane: Option<Lane>) {
    board().insert(
        session_id.to_string(),
        Entry {
            lane,
            applied: false,
        },
    );
}

/// The lane the model picked this turn: `Some(choice)` overrides the context's
/// lane, `None` means no request is pending.
pub fn override_for(session_id: &str) -> Option<Option<Lane>> {
    board().get(session_id).map(|entry| entry.lane)
}

/// Requests the loop has not persisted yet. Marks them applied.
pub fn drain_unapplied() -> Vec<(String, Option<Lane>)> {
    board()
        .iter_mut()
        .filter(|(_, entry)| !entry.applied)
        .map(|(id, entry)| {
            entry.applied = true;
            (id.clone(), entry.lane)
        })
        .collect()
}

/// A new context reads the persisted lane; drop an entry that is already saved.
pub fn clear_applied(session_id: &str) {
    let mut b = board();
    if b.get(session_id).is_some_and(|entry| entry.applied) {
        b.remove(session_id);
    }
}

/// Parse the model's argument. `none` clears.
pub fn parse(value: &str) -> Result<Option<Lane>, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "" => Ok(None),
        "research" => Ok(Some(Lane::Research)),
        "red" => Ok(Some(Lane::Red)),
        "blue" => Ok(Some(Lane::Blue)),
        other => Err(format!(
            "unknown lane '{other}': use research, red, blue or none"
        )),
    }
}

/// What the lane allows, for the tool result.
pub fn summary(lane: Option<Lane>) -> &'static str {
    match lane {
        None => "lane cleared: sec_ tools other than sec_catalog and sec_lane are refused until a lane is set",
        Some(Lane::Research) => "research: local analysis only (no network, no target probes, no listeners); may propose patches",
        Some(Lane::Red) => "red: may reach the network, probe in-scope targets and start listeners; may not propose patches",
        Some(Lane::Blue) => "blue: may reach the network and probe in-scope targets; may propose patches; no listeners",
    }
}

#[cfg(test)]
#[path = "lane_switch_test.rs"]
mod tests;
