//! Key handler for the `/security` daemon control panel (`Mode::Security`).
//!
//! A read-only control panel — no sub-modes and no editing, so the dispatch is
//! simple: escape/quit, cursor navigation, and daemon lifecycle keys.
//!
//! Key map:
//! - `Esc`      → `Action::CloseSecurity` (return to Chat)
//! - `Ctrl+C`   → `Action::Quit`
//! - `Up`       → move cursor up in the ACTIVE pane (tools or deps)
//! - `Down`     → move cursor down in the ACTIVE pane (tools or deps)
//! - `h`/`H`    → toggle the body pane (tools ⇄ dependencies); mode-state only
//! - `i`/`I`    → (deps pane only) `Action::SecurityInstall` the selected dependency
//! - `r`        → `Action::SecurityRestart`
//! - `Enter`/`Space` → `Action::SecurityToggleTool` (toggle the SELECTED row: the daemon
//!   checkbox starts/stops the daemon; the YOLO checkbox arms/disarms Layer-1 YOLO (only
//!   when the daemon is running); a tool row flips that tool's active state)
//! - `d`        → `Action::SecurityToggleDomain` (toggle every tool in the selected tool's domain)
//!
//! Daemon start/stop/toggle no longer have dedicated keys (s/x/t removed) — the daemon is
//! now the top `[x] Daemon running` checkbox, toggled with Space like any other row.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::SecurityState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/security` control panel.
pub fn handle_security(s: &mut SecurityState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Keep the cursor clamp honest: the daemon may have started/stopped since the
    // panel opened (the view re-reads status live, but this mode-state copy didn't),
    // so refresh from the live manager before handling navigation.
    if let Some(m) = rest.sec_manager.as_ref() {
        s.status = m.status();
    }
    // Self-heal install-health: it is normally seeded once on panel open, but the daemon
    // starts ASYNCHRONOUSLY — if the panel opened before the daemon was ready (or the
    // daemon was started from inside the panel), the open-time probe never ran and no
    // other path re-probes. Kick off a NON-BLOCKING probe here once the daemon is up.
    // Gated on `is_empty()` (health() returns the full manifest, never an empty list, so a
    // delivered result sticks) and on `sec_health_rx.is_none()` inside the helper, so the
    // probe fires exactly once. NO blocking `health()` runs on the input path any more —
    // `service_global` drains the receiver and clears `health_fetching`.
    // Short-circuit `&&`: the side-effecting kick-off only fires when the panel actually
    // needs a probe (empty health, daemon up) — and the helper itself no-ops when one is
    // already in flight, so this stays exactly-once.
    if s.install_health.is_empty()
        && s.status.running
        && crate::app::runtime::commands::security::kick_off_health_probe(rest)
    {
        s.health_fetching = true;
    }
    // Keep the mode-state's inactive mirror in step with the authoritative set on
    // `rest` (the action handlers mutate `rest.sec_inactive` then refresh, but a
    // re-entry into the panel reads this on each key).
    s.inactive = rest.sec_inactive.clone();
    // Same for the YOLO arm flag — mirror the authoritative `rest.yolo_armed` so the
    // checkbox row reflects the live state on every key.
    s.yolo_armed = rest.yolo_armed;
    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::CloseSecurity,
        KeyCode::Up => {
            s.move_up();
            Action::None
        }
        KeyCode::Down => {
            s.move_down();
            Action::None
        }
        KeyCode::Enter | KeyCode::Char(' ') => Action::SecurityToggleTool,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::SecurityToggleDomain,
        // Toggle the body pane (tools ⇄ dependencies). Pure mode-state mutation, like
        // the cursor moves — no runtime round-trip needed.
        KeyCode::Char('h') | KeyCode::Char('H') => {
            s.toggle_health_view();
            Action::None
        }
        // Install/repair the selected dependency — ONLY in the dependency pane, and only
        // when a row is actually selected. `health_selected` indexes `health_items()`
        // (render order), so resolve through it to find the underlying entry.
        KeyCode::Char('i') | KeyCode::Char('I') => {
            if s.health_view {
                match s
                    .health_items()
                    .get(s.health_selected)
                    .and_then(|&idx| s.install_health.get(idx))
                {
                    Some(e) => Action::SecurityInstall(e.key.clone()),
                    None => Action::None,
                }
            } else {
                Action::None
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => Action::SecurityRestart,
        _ => Action::None,
    }
}
