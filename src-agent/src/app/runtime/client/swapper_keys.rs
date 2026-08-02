//! Swapper key-handling: applying one keystroke to the client-side session
//! hub, and the Ctrl+X arm/confirm lanes (kill a live session / delete a
//! history entry). Split out of [`super::swapper`] for file size; `run_swapper`
//! there keeps calling [`handle_swapper_key`] (bumped to `pub(super)` — every
//! other function here is only called internally within this module, so stays
//! private; no other behaviour change).

use std::sync::mpsc;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::{HubPane, SessionHub, SessionKind};
use crate::controller::input::is_ctrl;
use crate::ipc::proto::SessionStatus;

use super::super::manage;
use super::swapper::SwapperOutcome;

/// Apply one key to the swapper's `hub`, returning `Some(outcome)` if it resolves the
/// swapper (Enter/Esc/Ctrl+C) or `None` if it was a navigation/edit key (state mutated
/// in place, keep looping).
///
/// Mirrors [`crate::controller::input::handle_session_hub`]'s key → hub-state mutations
/// so the swapper feels identical to today's `/resume` picker, with two deliberate
/// differences for the client-side, cross-daemon context:
///   - Enter resolves to a SESSION UUID ([`SwapperOutcome::Pick`]) instead of a daemon-
///     side `Action` (LiveSwitch by Vec index / HubOpenHistory by `history` index): a
///     cooking `[+ new session]` row mints a fresh UUID, a real cooking row uses its
///     `session_id`, and a history row derives its UUID from the on-disk path's final
///     component (the session dir name == its id);
///   - `Ctrl+X` is a two-step KILL on the focused live session (arm → confirm), reaping
///     that session's daemon so it drops out of COOKING and reappears in HISTORY — see the
///     `Ctrl+X` arm below.
///
/// `snap_tx` is the discovery-snapshot channel the background probe feeds; the off-thread
/// KILL worker ([`handle_ctrl_x_cooking_nuke`]) ships its post-kill sweep back through it so
/// the main loop refreshes the session list (and the `is_foreground` flag) WITHOUT the input
/// thread ever blocking on the multi-second kill.
pub(super) fn handle_swapper_key(
    hub: &mut SessionHub,
    key: &KeyEvent,
    snap_tx: &mpsc::Sender<Vec<SessionStatus>>,
) -> Option<SwapperOutcome> {
    // --- Ctrl+X: two-step kill (arm → confirm) on the focused live session ---
    // koma's kill convention (matches /bash, the sub-agent abort, the daemon-side hub).
    // Checked FIRST because `is_ctrl` inspects modifiers, and because a confirming second
    // Ctrl+X must NOT be treated as a disarm below.
    if is_ctrl(key, 'x') {
        return handle_ctrl_x_nuke(hub, snap_tx);
    }

    // --- disarm: any key OTHER than a confirming Ctrl+X cancels a pending kill ---
    // The kill fires only on Ctrl+X immediately followed by Ctrl+X on the SAME row; moving
    // the selection or pressing anything else disarms. We clear here, then process the key
    // normally below — EXCEPT `Esc`, which is swallowed (it only disarms; a SECOND Esc then
    // exits the swapper as usual, per the spec).
    if hub.pending_kill.is_some() {
        hub.pending_kill = None;
        if matches!(key.code, KeyCode::Esc) && !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        // Fall through: the key is handled normally this same press.
    }

    // Same disarm rule for a pending HISTORY-pane delete: any key other than a
    // confirming Ctrl+X (intercepted above) cancels the armed delete.
    if hub.pending_delete.is_some() {
        hub.pending_delete = None;
        if matches!(key.code, KeyCode::Esc) && !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
    }

    // Ctrl+C is fully inert now (koma disables it): it no longer cancels the swapper.
    // Esc is the sole cancel gesture (handled in the match below).

    match key.code {
        // Esc → cancel back (reconnect to the previous session, or exit on a cold start).
        KeyCode::Esc => Some(SwapperOutcome::Cancel),

        // Tab / Shift+Tab → toggle the focused pane (cursor of the other pane preserved).
        KeyCode::Tab | KeyCode::BackTab => {
            hub.toggle_focus();
            None
        }

        // Up / Down → move the focused pane's cursor (History scrolls the FILTERED view).
        KeyCode::Up => {
            hub.move_up();
            None
        }
        KeyCode::Down => {
            hub.move_down();
            None
        }

        // Enter → resolve the focused selection to a target session UUID.
        KeyCode::Enter => resolve_enter(hub),

        // Backspace → delete from the history search (History pane only), then refilter.
        KeyCode::Backspace => {
            if matches!(hub.focus, HubPane::History) {
                hub.history_query.pop();
                hub.refilter_history();
            }
            None
        }

        // Printable key (NOT a Ctrl chord — those are intercepted before reaching the
        // search). The daemon handler interleaves its Ctrl+C / Ctrl+X chord guards ahead
        // of the Char arm; here we guard inline so a Ctrl+X (deferred kill) / any Ctrl
        // combo never leaks a literal char into the history query.
        //   - History pane: feed the live search query (push + refilter).
        //   - Cooking pane: inert. (The daemon handler treats n/N as the `/new` shortcut;
        //     here the `[+ new session]` row is Enter-selectable, so no shortcut is needed
        //     and stray letters on the cooking pane do nothing.)
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if matches!(hub.focus, HubPane::History) {
                hub.history_query.push(c);
                hub.refilter_history();
            }
            None
        }

        // Any other key — including a Ctrl chord that fell through the guards above (Ctrl+X
        // is handled at the top; Ctrl+C is fully inert now): ignore.
        _ => None,
    }
}

fn handle_ctrl_x_nuke(
    hub: &mut SessionHub,
    snap_tx: &mpsc::Sender<Vec<SessionStatus>>,
) -> Option<SwapperOutcome> {
    match hub.focus {
        HubPane::Cooking => handle_ctrl_x_cooking_nuke(hub, snap_tx),
        HubPane::History => {
            handle_ctrl_x_history_delete(hub);
            None
        }
    }
}

/// Handle a `Ctrl+X` press on the COOKING pane: the two-step session KILL
/// (arm → confirm) that reaps the focused live session's daemon.
///
/// Guard: acts ONLY when the highlighted row is a REAL live session
/// (`SessionKind::Session` carrying `Some(session_id)`). On the synthetic
/// `[+ new session]` row or a row with no id, it returns `None` and arms nothing.
///
/// Two-step (mirrors the daemon-side hub's arm→confirm):
///   - not yet armed on THIS row → arm: set `pending_kill` to the focused cooking
///     index and stay in the picker (the confirm bar renders from `pending_kill`);
///   - already armed AND still aimed at the same session UUID → CONFIRM = kill: clear the
///     arm and reap that session's daemon OFF this thread. The escalating
///     [`manage::kill_session_daemon`] (a SILENT graceful→SIGTERM→SIGKILL kill, alt-screen
///     safe — unlike `stop_session_daemon`, which prints) BLOCKS until the daemon is actually
///     dead, and the follow-up [`manage::list_live_sessions`] sweep round-trips every live
///     socket — so BOTH run on a spawned worker that ships the fresh snapshot back through
///     `snap_tx` (the SAME channel the periodic probe feeds). The main loop drains it and
///     merges via `apply_snapshot` (in the sibling `swapper` module), so the killed row
///     simply disappears on the next snapshot push while the picker keeps repainting /
///     navigating throughout — never freezing for the multi-second kill (mirrors this
///     module's off-thread probe design; a wedged daemon is still force-killed, just
///     asynchronously).
///
/// Always returns `None` — a kill never resolves the swapper; the user keeps picking.
fn handle_ctrl_x_cooking_nuke(
    hub: &mut SessionHub,
    snap_tx: &mpsc::Sender<Vec<SessionStatus>>,
) -> Option<SwapperOutcome> {
    // Resolve the focused row + its session id; bail (arm nothing) on the synthetic
    // new-session row or any row without an id.
    let target_id = match hub.selected_cooking() {
        Some(entry) if entry.kind == SessionKind::Session => entry.session_id.clone()?,
        _ => return None,
    };

    // Is a kill already armed AND still aimed at this session? (Selection could
    // have moved, or the background probe could have shifted the list — then this
    // Ctrl+X re-arms on the new row instead of confirming.)
    let armed_here = hub.pending_kill.as_deref() == Some(target_id.as_str());

    if !armed_here {
        // First press → ARM. The confirm bar renders from `pending_kill`.
        hub.pending_kill = Some(target_id);
        return None;
    }

    // Second press on the same row → CONFIRM = KILL.
    // Clear the arm now (the confirm bar drops on this repaint), then reap the daemon OFF
    // this thread. The escalating kill (graceful QuitDaemon → SIGTERM → SIGKILL, alt-screen
    // safe) BLOCKS up to the grace budget (KILL_GRACE + two SIGNAL_GRACE windows), and the
    // follow-up list_live_sessions() sweep round-trips every live socket on top — running
    // EITHER inline would freeze the single input/render thread (no repaint / Esc / nav) for
    // seconds, contradicting this module's off-thread probe design. So the worker kills,
    // re-sweeps discovery ITSELF, and ships the result through the SAME snapshot channel the
    // periodic probe feeds; the main loop drains it and merges via `apply_snapshot`
    // (preserving cursor/focus/query + the is_foreground flag). The killed row disappears on
    // that next snapshot push — the loop keeps repainting throughout. There is no existing
    // per-row "dying" state to set, so the row simply vanishes on the refresh (acceptable).
    hub.pending_kill = None;
    let snap_tx = snap_tx.clone();
    std::thread::spawn(move || {
        manage::kill_session_daemon(&target_id); // blocks until dead (or the budget is spent)
        let _ = snap_tx.send(manage::list_live_sessions());
    });

    None
}

/// Handle a `Ctrl+X` press on the HISTORY pane: two-step PHYSICAL delete of the
/// on-disk session (arm → confirm). First press arms `pending_delete` on the
/// focused row's session UUID; a second press on the SAME row deletes the
/// session directory + its registry row via [`crate::model::store::delete_session`],
/// drops it from the in-memory `history` list, and refilters so it vanishes
/// immediately (the background probe only refreshes COOKING — history is
/// client-owned here). Irreversible. No-op on an empty filtered view.
fn handle_ctrl_x_history_delete(hub: &mut SessionHub) {
    let real = match hub.selected_history_real_idx() {
        Some(r) => r,
        None => return,
    };
    let (path, uuid) = match hub.history.get(real) {
        Some(e) => match e.path.file_name().and_then(|n| n.to_str()) {
            Some(id) => (e.path.clone(), id.to_string()),
            None => return,
        },
        None => return,
    };
    // Not yet armed on THIS row → arm and wait for the confirming second press.
    if hub.pending_delete.as_deref() != Some(uuid.as_str()) {
        hub.pending_delete = Some(uuid);
        return;
    }
    // Second press on the same row → CONFIRM = physical delete (disk + registry).
    let _ = crate::model::store::delete_session(&path);
    hub.pending_delete = None;
    if real < hub.history.len() {
        hub.history.remove(real);
    }
    hub.refilter_history();
}

/// Resolve an Enter press in the swapper to a target session UUID (or `None` to stay in
/// the picker when the focused pane has nothing actionable). Pure read of `hub`.
fn resolve_enter(hub: &SessionHub) -> Option<SwapperOutcome> {
    match hub.focus {
        HubPane::Cooking => match hub.selected_cooking() {
            // "[+ new session]" → mint a brand-new session UUID; `client_run` spawns its
            // daemon (create branch) and attaches.
            Some(entry) if entry.kind == SessionKind::NewSession => {
                Some(SwapperOutcome::Pick(uuid::Uuid::new_v4().to_string()))
            }
            // A real live session → attach to its already-running daemon by its id. A
            // real row always carries `Some(id)`; the `?`-less guard degrades a (never-
            // expected) `None` to staying in the picker rather than picking a blank id.
            Some(entry) => entry.session_id.clone().map(SwapperOutcome::Pick),
            // Empty cooking pane (can't happen — the synthetic row is always present) →
            // stay in the picker.
            None => None,
        },
        HubPane::History => match hub.selected_history_real_idx() {
            // A history row → derive the session UUID from the on-disk dir name (the path's
            // final component IS the session id), then `client_run` create-or-LOADs it.
            // A path with no final component (shouldn't happen for a real session dir) →
            // stay in the picker rather than pick a bogus id.
            Some(real) => hub
                .history
                .get(real)
                .and_then(|h| h.path.file_name())
                .and_then(|n| n.to_str())
                .map(|id| SwapperOutcome::Pick(id.to_string())),
            // Empty filtered history (e.g. a search that matched nothing) → stay.
            None => None,
        },
    }
}
