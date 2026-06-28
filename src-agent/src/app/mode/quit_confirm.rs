//! State for the `/quit` confirm overlay (`Mode::QuitConfirm`).
//!
//! Shown ALWAYS when the user asks to quit (the `/quit` command or the quit
//! keybind), regardless of whether any session has work in flight. The user may
//! want to KEEP idle sessions on disk (detach) so they reappear in the session
//! hub's history pane on the next launch — so we always ask.
//!
//! It is a fixed snapshot built when the confirm opens: `working` is the count
//! of busy sessions and `total` is the total session count at that moment
//! (purely for the header text — sessions changing state while the overlay is up
//! just mean a slightly stale count, which is harmless). The overlay offers
//! three choices, handled in [`crate::controller::input::handle_quit_confirm`]:
//!   [k] kill all & quit — abort every session's in-flight stream, then exit;
//!   [d] detach & quit    — leave conversations persisted on disk, then exit
//!                          (Phase 1: they still die with the process — true
//!                          headless detach arrives with the daemon);
//!   [esc] cancel         — back to Chat, no quit.

/// State for the quit-confirm overlay.
///
/// `working` is the number of sessions with work in flight at open time.
/// `total` is the total number of sessions. Both are display-only: the header
/// text adapts based on whether any work is in flight. No selection cursor: the
/// three choices are bound to distinct keys (k / d / Esc).
pub struct QuitConfirmState {
    /// Count of live sessions with work in flight at open time. Display only.
    pub working: usize,
    /// Total number of sessions at open time. Display only.
    pub total: usize,
}

impl QuitConfirmState {
    /// Build the overlay state from the busy and total session counts.
    pub fn new(working: usize, total: usize) -> Self {
        Self { working, total }
    }
}
