//! Quit-flow action handlers: the working-aware quit chokepoint and the
//! kill-all / detach / cancel choices from the [`Mode::QuitConfirm`] overlay.
//!
//! The single entry point for ANY quit request (the `/quit` command and the quit
//! keybind both route here) is [`request_quit`]:
//!   - ALWAYS opens the confirm overlay so the user picks kill-all vs detach vs
//!     cancel — even when nothing is working, the user may want to detach idle
//!     sessions so they persist on disk and reappear in the session hub's history
//!     pane on the next launch.
//!   - Only exception: zero sessions (should never happen normally); in that
//!     case quit immediately.
//!
//! All on-disk lock teardown happens on the NATURAL exit path (after `run_loop`
//! returns, in [`crate::app::runtime::run`]), which now releases EVERY session's
//! lock — so neither handler here touches locks directly.

use crate::app::mode::{Mode, QuitConfirmState};
use crate::app::state::AppState;

/// Quit chokepoint shared by the `/quit` command and the quit keybind.
///
/// Always opens the [`Mode::QuitConfirm`] overlay so the user must choose
/// kill-all, detach, or cancel — even when nothing is working, the user may
/// want to detach idle sessions so they persist on disk and reappear in the
/// session hub's history pane on the next launch. The overlay header adapts its
/// wording to the working-vs-idle state.
///
/// Two exceptions quit IMMEDIATELY (no overlay):
///   - zero sessions (normally impossible);
///   - a landing / unconfigured screen ([`Mode::Onboard`], [`Mode::OnboardProvider`],
///     or a first-run [`Mode::KeyInput`]). These are setup-or-quit ONLY: routing them
///     through QuitConfirm would let its cancel path ([`handle_quit_cancel`]) drop a
///     brand-new user into an unconfigured Chat, so Esc/'q' here is a clean exit.
///
/// [`SessionRuntime::is_working`]: crate::app::state::SessionRuntime::is_working
pub(in crate::app::runtime) fn request_quit(state: &mut AppState) {
    let total = state.rest.sessions.len();
    // Zero sessions: nothing to keep or kill — just quit immediately.
    if total == 0 {
        state.rest.should_quit = true;
        return;
    }
    // Landing / unconfigured screens are setup-or-quit ONLY: a quit request here must
    // exit cleanly rather than open QuitConfirm (whose cancel returns to Chat — a dead,
    // unconfigured Chat for a first-run user). Covers the first-run chooser, the guided
    // provider wizard, and the first-run credentials wizard at one chokepoint.
    let on_landing = match state.mode() {
        Mode::Onboard(_) | Mode::OnboardProvider(_) => true,
        Mode::KeyInput(form) => form.first_run,
        _ => false,
    };
    if on_landing {
        state.rest.should_quit = true;
        return;
    }
    let working = state
        .rest
        .sessions
        .iter()
        .filter(|s| s.is_working())
        .count();
    // Always ask: the overlay header adapts to whether work is in flight.
    *state.mode_mut() = Mode::QuitConfirm(Box::new(QuitConfirmState::new(working, total)));
    // Re-enable mouse capture temporarily so the overlay's left-click buttons
    // work — but only when mouse capture is effectively ON (touch terminal or
    // explicit `on`). When capture is OFF (desktop default), QuitConfirm is
    // keyboard-only (already works) and we avoid a spurious enable/disable cycle.
    let capture_on = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.mouse_capture.resolved())
        .unwrap_or(false);
    if capture_on {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::EnableMouseCapture
        );
    }
}

/// Handle `Action::QuitKillAll`: abort EVERY session's in-flight stream, then
/// transition to the exit-feedback phase. Mirrors
/// [`crate::app::runtime::stream::abort_current`] but across ALL sessions (that
/// helper only touches the foreground): for each session it aborts the task
/// handle, drops the active receiver (so late events vanish), and clears the
/// `waiting` flag. Also tears down any in-flight compaction animation (those
/// fields are global, not per-session). Locks are released by the natural exit
/// path.
///
/// Instead of setting `should_quit` directly, this transitions the
/// `QuitConfirmState` to the `Exiting` phase so the view can render a braille
/// spinner exit screen. The event loop sees the Exiting phase and sets
/// `should_quit` after drawing, ensuring the user gets visible feedback before
/// synchronous shutdown begins.
pub(super) fn handle_quit_kill_all(state: &mut AppState) {
    for s in &mut state.rest.sessions {
        if let Some(h) = s.current_task.take() {
            h.abort();
        }
        s.active_rx = None;
        s.waiting = false;
        // Tear down EACH session's in-flight compaction animation / deferred apply so a
        // kill mid-compact leaves no bookkeeping dangling. Per-session now (C4) — clear
        // it on every session as part of the same kill-all sweep.
        s.compact_anim_start = None;
        s.compact_apply_at = None;
        s.compact_pending = None;
    }
    // Keep the dialog visible and put the spinner inside the quit chip.
    if let crate::app::mode::Mode::QuitConfirm(s) = state.mode_mut() {
        s.selected = 0;
        s.phase = crate::app::mode::QuitConfirmPhase::Exiting;
    }
}

/// Handle `Action::QuitDetach`: detach & quit. Transition to the exit-feedback
/// phase so the view renders a braille spinner while the process exits. The
/// session's conversation stays persisted on disk and is resumable later.
/// Locks are released by the natural exit path.
///
/// Phase 1 caveat: there is no daemon yet, so the in-flight work still dies when
/// the process exits — "detach" here means "leave it resumable", not "keep it
/// cooking headless". The overlay copy says so explicitly.
pub(super) fn handle_quit_detach(state: &mut AppState) {
    // Keep the dialog visible and put the spinner inside the detach chip.
    if let crate::app::mode::Mode::QuitConfirm(s) = state.mode_mut() {
        s.selected = 1;
        s.phase = crate::app::mode::QuitConfirmPhase::Exiting;
    }
}

/// Handle `Action::QuitCancel`: dismiss the overlay and return to Chat
/// unchanged. Nothing is aborted; the app keeps running.
pub(super) fn handle_quit_cancel(state: &mut AppState) {
    *state.mode_mut() = Mode::Chat;
    // Restore the mouse-capture state to what the session setting dictates —
    // the QuitConfirm open may have toggled it; cancel re-applies the
    // session's current mode (ON for touch terminals, OFF for desktop).
    let mc = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.mouse_capture)
        .unwrap_or_default();
    crate::app::runtime::actions::apply_mouse_capture(mc);
}
