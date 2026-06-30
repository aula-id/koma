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
//! three choices, rendered as a navigable horizontal button row and handled in
//! [`crate::controller::input::handle_quit_confirm`]:
//!   [quit & kill] — abort every session's in-flight stream, then exit;
//!   [minimize]    — leave conversations persisted on disk (resumable later),
//!                   then exit — in-flight work still stops when the process exits;
//!   [cancel]      — back to Chat, no quit.
//!
//! Focus moves across the row with Left/Right (and Tab/Shift+Tab); Enter
//! activates the focused button. The legacy `k` / `d` / `Esc` shortcuts still
//! fire their action directly. The focused button index lives in
//! [`QuitConfirmState::selected`].
//!
//! The same three choices are also CLICKABLE: the draw fn records each button's
//! on-screen [`Rect`] into [`QuitConfirmState::button_rects`] (interior
//! mutability, since the draw takes `&self`), and the event loop hit-tests a
//! left-click against them to dispatch the same actions the keys do.

use std::cell::Cell;

use ratatui::layout::Rect;

/// State for the quit-confirm overlay.
///
/// `working` is the number of sessions with work in flight at open time.
/// `total` is the total number of sessions. Both are display-only: the header
/// text adapts based on whether any work is in flight. The three choices form a
/// navigable horizontal button row: `selected` is the focused button (also
/// driven by clicks), and each choice is still bound to a direct key shortcut
/// (k / d / Esc).
pub struct QuitConfirmState {
    /// Count of live sessions with work in flight at open time. Display only.
    pub working: usize,
    /// Total number of sessions at open time. Display only.
    pub total: usize,
    /// Index of the currently focused button, in fixed order:
    /// `0` = quit & kill, `1` = minimize (detach), `2` = cancel. Moved by
    /// Left/Right + Tab/Shift+Tab (and a click sets it to the hit button);
    /// Enter activates it. Initialized to `2` (cancel) so an immediate Enter
    /// lands on the SAFE choice and can't accidentally kill sessions.
    pub selected: usize,
    /// On-screen hit-boxes for the three clickable buttons, in fixed order:
    /// `[0]` = quit & kill (k), `[1]` = minimize (d), `[2]` = cancel (esc).
    /// Written by the `&self` draw via interior mutability each frame and read by
    /// the event loop on a left-click. The buttons are laid out as horizontal
    /// segments on one row, so each rect is a chip-width band. All-zero
    /// (`Rect::ZERO`) until the first paint, so a click before the overlay has
    /// rendered simply hits nothing. NOT part of the IPC snapshot (the projection
    /// copies only `working`/`total`), so no serde.
    pub button_rects: Cell<[Rect; 3]>,
}

impl QuitConfirmState {
    /// Build the overlay state from the busy and total session counts.
    ///
    /// Focus starts on `2` (cancel) — the safe default — and the click hit-boxes
    /// start empty (`Rect::ZERO`); the first paint fills them.
    pub fn new(working: usize, total: usize) -> Self {
        Self {
            working,
            total,
            selected: 2,
            button_rects: Cell::new([Rect::ZERO; 3]),
        }
    }
}
