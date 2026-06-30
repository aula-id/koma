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
//!   [d] detach & quit    — leave conversations persisted on disk (resumable
//!                          later), then exit — in-flight work still stops when
//!                          the process exits;
//!   [esc] cancel         — back to Chat, no quit.
//!
//! The same three choices are also CLICKABLE: the draw fn records each option
//! row's on-screen [`Rect`] into [`QuitConfirmState::button_rects`] (interior
//! mutability, since the draw takes `&self`), and the event loop hit-tests a
//! left-click against them to dispatch the same actions the keys do.

use std::cell::Cell;

use ratatui::layout::Rect;

/// State for the quit-confirm overlay.
///
/// `working` is the number of sessions with work in flight at open time.
/// `total` is the total number of sessions. Both are display-only: the header
/// text adapts based on whether any work is in flight. No selection cursor: the
/// three choices are bound to distinct keys (k / d / Esc) AND to clicks on their
/// rows.
pub struct QuitConfirmState {
    /// Count of live sessions with work in flight at open time. Display only.
    pub working: usize,
    /// Total number of sessions at open time. Display only.
    pub total: usize,
    /// On-screen hit-boxes for the three clickable option rows, in fixed order:
    /// `[0]` = kill (k), `[1]` = detach (d), `[2]` = cancel (esc). Written by the
    /// `&self` draw via interior mutability each frame and read by the event loop
    /// on a left-click. All-zero (`Rect::ZERO`) until the first paint, so a click
    /// before the overlay has rendered simply hits nothing. NOT part of the IPC
    /// snapshot (the projection copies only `working`/`total`), so no serde.
    pub button_rects: Cell<[Rect; 3]>,
}

impl QuitConfirmState {
    /// Build the overlay state from the busy and total session counts.
    ///
    /// The click hit-boxes start empty (`Rect::ZERO`); the first paint fills them.
    pub fn new(working: usize, total: usize) -> Self {
        Self {
            working,
            total,
            button_rects: Cell::new([Rect::ZERO; 3]),
        }
    }
}
