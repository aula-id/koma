//! Key handler for the unified session hub (`/resume`, `Mode::SessionHub`).
//!
//! Tab toggles the focused pane (cooking <-> history); Up/Down move the selection
//! within the focused pane; Enter acts on the focused pane's selection; Esc
//! closes back to Chat without touching any session state (Ctrl+C is inert).
//!
//! - Enter on a COOKING row -> [`Action::LiveSwitch`] carrying that session's
//!   `sessions` index (REUSES the `/swap` foreground-switch path verbatim).
//! - Enter on a HISTORY row -> [`Action::HubOpenHistory`] carrying that row's REAL
//!   index into `history` (resolved through the live filter); the runtime resolves
//!   it to a path and loads it into a new tab.
//! - Ctrl+X on a COOKING real-session row -> arm a kill confirm (`pending_kill`);
//!   while armed the hub only accepts confirm (Enter / y / Ctrl+X) or cancel
//!   (Esc / n). Confirm emits [`Action::HubKillConfirm`].
//! - On the HISTORY pane, printable keys feed a live search (`history_query`);
//!   Backspace deletes from it. On the COOKING pane, `n`/`N` stays the `/new`
//!   shortcut (no search there).
//!
//! The hub carries its own state (a snapshot of live + on-disk sessions), so
//! `_rest` is unused — mirroring [`super::handle_rewind`].

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use crate::app::mode::{HubPane, SessionHub, SessionKind};
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Convenience: the `/new` (swap) slash action — the cooking pane's `n`/`N`
/// shortcut and the synthetic "[+ new session]" row both resolve to it.
fn new_session_action() -> Action {
    Action::Slash(crate::controller::command::Command::New(
        crate::controller::command::NewMode::Swap,
    ))
}

/// Handle a key press inside the session hub.
///
/// Order matters (see the module docs): Ctrl+C is inert; a pending kill confirm
/// short-circuits to confirm/cancel; Ctrl+X arms a kill on a cooking real-session
/// row; then Esc/Tab/arrows/Enter; then Backspace + printable keys feed the
/// history search (cooking-pane `n`/`N` stays the `/new` shortcut).
pub fn handle_session_hub(
    hub: &mut SessionHub,
    _rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    // 1. Ctrl+C is fully inert (koma disables it); Esc still closes the hub back to Chat.
    if is_ctrl(&key, 'c') {
        return Action::None;
    }

    // 2. CONFIRM MODE: while a kill is armed, accept ONLY confirm or cancel and
    //    return immediately — nothing else runs while confirming.
    if hub.pending_kill.is_some() {
        // Confirm: Enter, y/Y, or another Ctrl+X.
        if matches!(key.code, KeyCode::Enter)
            || matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
            || is_ctrl(&key, 'x')
        {
            return Action::HubKillConfirm;
        }
        // Cancel: Esc or n/N. Any other key is swallowed (stay armed).
        if matches!(key.code, KeyCode::Esc)
            || matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N'))
        {
            hub.pending_kill = None;
        }
        return Action::None;
    }

    // 2b. CONFIRM MODE for HISTORY-pane physical delete: while a delete is armed,
    //     accept ONLY confirm (Ctrl+X or Enter) or cancel (Esc / n / N). Mutually
    //     exclusive with pending_kill (different panes).
    if hub.pending_delete.is_some() {
        // Confirm: another Ctrl+X (or Enter, mirroring the kill convention).
        if is_ctrl(&key, 'x') || matches!(key.code, KeyCode::Enter) {
            return Action::HubDeleteConfirm;
        }
        // Cancel: Esc or n/N. Any other key clears the arm and falls through.
        if matches!(key.code, KeyCode::Esc)
            || matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N'))
        {
            hub.pending_delete = None;
            return Action::None;
        }
        // Any other key clears the arm and falls through to normal handling.
        hub.pending_delete = None;
        // Fall through.
    }

    // 3. Ctrl+X: arm the appropriate action based on the focused pane.
    //    - COOKING pane: arm a live-session kill (only on REAL session rows).
    //    - HISTORY pane: arm a physical delete (only when a row is selected).
    //    The two are mutually exclusive by focus; arming one also clears the other.
    if is_ctrl(&key, 'x') {
        match hub.focus {
            HubPane::Cooking => {
                if let Some(entry) = hub.selected_cooking() {
                    if entry.kind == SessionKind::Session {
                        hub.pending_kill = entry.session_id.clone();
                        hub.pending_delete = None;
                    }
                }
            }
            HubPane::History => {
                if let Some(real) = hub.selected_history_real_idx() {
                    if let Some(e) = hub.history.get(real) {
                        if let Some(uuid) = e.path.file_name().and_then(|n| n.to_str()) {
                            hub.pending_delete = Some(uuid.to_string());
                            hub.pending_kill = None;
                        }
                    }
                }
            }
        }
        return Action::None;
    }

    match key.code {
        // 4. Esc → close back to Chat unchanged.
        KeyCode::Esc => Action::CloseSessionHub,
        // 5. Tab / Shift+Tab → toggle the focused pane.
        KeyCode::Tab | KeyCode::BackTab => {
            hub.toggle_focus();
            Action::None
        }
        // 6. Up / Down → move the focused pane's cursor.
        KeyCode::Up => {
            hub.move_up();
            Action::None
        }
        KeyCode::Down => {
            hub.move_down();
            Action::None
        }
        // 7. Enter → act on the focused pane's selection. History opens the REAL
        //    filtered index so the row the user sees is the one that loads.
        KeyCode::Enter => match hub.focus {
            HubPane::Cooking => match hub.selected_cooking() {
                Some(entry) if entry.kind == SessionKind::NewSession => new_session_action(),
                Some(entry) => Action::LiveSwitch(entry.idx),
                None => Action::CloseSessionHub,
            },
            HubPane::History => match hub.selected_history_real_idx() {
                // Carry the REAL `history` index; the runtime reads the path back
                // out and loads it (non-destructively) into a new tab.
                Some(real) => Action::HubOpenHistory(real),
                None => Action::CloseSessionHub,
            },
        },
        // 8. Backspace → delete from the history search (History pane only).
        KeyCode::Backspace => {
            if matches!(hub.focus, HubPane::History) {
                hub.history_query.pop();
                hub.refilter_history();
            }
            Action::None
        }
        // 9. Printable key (NOT a Ctrl chord — those are handled above):
        //    - Cooking pane: n/N is the `/new` shortcut; everything else is inert.
        //    - History pane: feed the live search query.
        KeyCode::Char(c) => match hub.focus {
            HubPane::Cooking => {
                if c == 'n' || c == 'N' {
                    new_session_action()
                } else {
                    Action::None
                }
            }
            HubPane::History => {
                hub.history_query.push(c);
                hub.refilter_history();
                Action::None
            }
        },
        // 10. Anything else → ignored.
        _ => Action::None,
    }
}
