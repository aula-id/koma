//! Client-side session-hub builder for the daemon swapper.
//!
//! The thin attach client connects to ONE session-daemon's socket. To let the
//! user switch among LIVE session-daemons on `--resume` / `/resume`, the client
//! needs its OWN [`SessionHub`] sourced from cross-daemon DISCOVERY rather than
//! from a single daemon's `AppStateRest::sessions` (which only knows its own one
//! session). [`build_local_hub`] mirrors the SHAPE of the daemon-side
//! [`crate::app::runtime::commands::new_session::build_session_hub`] but draws
//! its COOKING rows from [`super::super::manage::list_live_sessions`] and keys
//! each row by the session UUID (the swapper's addressing key) instead of a Vec
//! index.
//!
//! The consumer (the `/resume` swap that reconnects the client to the chosen
//! daemon) lands in the NEXT commit, so this builder is currently unused.

use crate::app::mode::{CookingEntry, HistoryEntry, HubPane, SessionHub, SessionKind};
use crate::model::store;

/// Build a CLIENT-side [`SessionHub`] from cross-daemon discovery.
///
/// COOKING = a synthetic "[+ new session]" row, then one row per LIVE
/// session-daemon from [`super::super::manage::list_live_sessions`], each keyed
/// by its session UUID (`session_id`) — the swapper addresses the chosen daemon
/// by this id, not by a Vec index (so `idx` is left as the sentinel `usize::MAX`,
/// matching the daemon builder and unused client-side). The row tagged
/// `is_foreground` is the session the client is CURRENTLY attached to
/// (`current_session_id`), if it is among the live set.
///
/// HISTORY = the on-disk sessions from [`store::list_sessions`] MINUS any whose
/// UUID is currently live (dedup by id: a live session shows ONLY in cooking,
/// mirroring the daemon builder's path-dedup intent). A `list_sessions` failure
/// yields an empty history pane rather than a surfaced error.
///
/// Mirrors the daemon builder's defaults exactly: focus on the cooking pane,
/// cursors at 0, empty history query, identity history filter, no pending kill.
#[allow(dead_code)] // consumed by the `/resume` swap in the next commit
pub(crate) fn build_local_hub(current_session_id: Option<&str>) -> SessionHub {
    // Discover the live session-daemons once: this drives the COOKING rows AND
    // the live-id set used to dedup HISTORY below.
    let live = super::super::manage::list_live_sessions();

    // The set of LIVE session UUIDs, used to hide already-live sessions from the
    // HISTORY pane. `SessionStatus::session_id` and `SessionMeta::id` are the SAME
    // UUID namespace (both the on-disk session dir name / socket key), so a string
    // set dedups them directly.
    let live_ids: std::collections::HashSet<String> =
        live.iter().map(|s| s.session_id.clone()).collect();

    // COOKING pane: a synthetic "[+ new session]" row first, then one row per live
    // session-daemon. `idx` is the sentinel (unused client-side); `session_id` is
    // the real addressing key.
    let mut cooking: Vec<CookingEntry> = Vec::with_capacity(live.len() + 1);
    cooking.push(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
        session_id: None,
    });
    for status in live {
        // Compute the foreground flag BEFORE moving the id/name out of `status`.
        let is_foreground = current_session_id == Some(status.session_id.as_str());
        cooking.push(CookingEntry {
            idx: usize::MAX,
            kind: SessionKind::Session,
            name: status.name,
            working: status.working,
            is_foreground,
            session_id: Some(status.session_id),
        });
    }

    // HISTORY pane: on-disk sessions MINUS the live ones (dedup by UUID). A listing
    // failure shouldn't block the hub — show an empty history pane.
    let history: Vec<HistoryEntry> = match store::list_sessions() {
        Ok(metas) => metas
            .into_iter()
            .filter(|m| !live_ids.contains(&m.id))
            .map(|m| HistoryEntry {
                path: m.path,
                name: m.name,
                last_active: m.modified,
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    // History starts fully visible: identity filter, empty query.
    let history_filtered: Vec<usize> = (0..history.len()).collect();

    SessionHub {
        cooking,
        history,
        focus: HubPane::Cooking,
        cooking_selected: 0,
        history_selected: 0,
        history_query: String::new(),
        history_filtered,
        pending_kill: None,
    }
}
