//! State for the `/quit` confirm overlay (`Mode::QuitConfirm`).
//!
//! Shown ALWAYS when the user asks to quit (the `/quit` command or the quit
//! keybind), regardless of whether any session has work in flight. The user may
//! want to KEEP idle sessions on disk (detach) so they reappear in the session
//! hub's history pane on the next launch — so we always ask.
//!
//! The overlay has two phases:
//!
//! - **`Choice`** — awaiting the user's decision. Shows the question, three
//!   navigable buttons (`[quit]`, `[detach]`, `[cancel]`), and a focused-button
//!   description. Navigation (Left/Right, Tab/Shift+Tab), activation (Enter),
//!   and direct shortcuts (k/d/Esc) are handled in
//!   [`crate::controller::input::handle_quit_confirm`] (local TUI) /
//!   `client::input::handle_quit_confirm_key` (attached client).
//!
//! - **`Exiting`** — the user activated quit or detach. Shows a braille spinner
//!   with "Exiting…" and a description. All key/click input is suppressed to
//!   prevent duplicate quit/kill requests. The spinner is time-based (wall
//!   clock) so even a single frame of the exit state shows a meaningful glyph.
//!   The event loop breaks (standalone) or waits for socket disconnect (client)
//!   while this phase is visible.
//!
//! The same three choices are also CLICKABLE: the draw fn records each button's
//! on-screen [`Rect`] into [`QuitConfirmState::button_rects`] (interior
//! mutability, since the draw takes `&self`), and the event loop hit-tests a
//! left-click against them to dispatch the same actions the keys do.

use std::cell::Cell;

use ratatui::layout::Rect;

/// Phase of the quit-confirm overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitConfirmPhase {
    /// Awaiting the user's choice — the normal overlay with question + buttons.
    Choice,
    /// Exit in progress — the user activated quit or detach. Show a braille
    /// spinner and suppress all input until the process exits or the daemon
    /// socket disconnects (client mode).
    Exiting,
}

/// State for the quit-confirm overlay.
///
/// `working` is the number of sessions with work in flight at open time.
/// `total` is the total number of sessions. Both are display-only: the header
/// text adapts based on whether any work is in flight. The three choices form a
/// navigable horizontal button row: `selected` is the focused button (also
/// driven by clicks), and each choice is still bound to a direct key shortcut
/// (k / d / Esc).
///
/// `phase` tracks the overlay lifecycle: `Choice` (awaiting input) or
/// `Exiting` (spinner feedback, input suppressed). Not projected via IPC — the
/// client constructs its own `Exiting` state locally.
pub struct QuitConfirmState {
    /// Count of live sessions with work in flight at open time. Display only.
    pub working: usize,
    /// Total number of sessions at open time. Display only.
    pub total: usize,
    /// Index of the currently focused button, in fixed order:
    /// `0` = close window (quit), `1` = minimize (detach), `2` = cancel. Moved by
    /// Left/Right + Tab/Shift+Tab (and a click sets it to the hit button);
    /// Enter activates it. Initialized to `2` (cancel) so an immediate Enter
    /// lands on the SAFE choice and can't accidentally close a window's session.
    pub selected: usize,
    /// On-screen hit-boxes for the three clickable buttons, in fixed order:
    /// `[0]` = close window (quit) (k), `[1]` = minimize (d), `[2]` = cancel (esc).
    /// Written by the `&self` draw via interior mutability each frame and read by
    /// the event loop on a left-click. The buttons are laid out as horizontal
    /// segments on one row, so each rect is a chip-width band. All-zero
    /// (`Rect::ZERO`) until the first paint, so a click before the overlay has
    /// rendered simply hits nothing. NOT part of the IPC snapshot (the projection
    /// copies only `working`/`total`), so no serde.
    pub button_rects: Cell<[Rect; 3]>,
    /// Current phase: `Choice` (awaiting user decision) or `Exiting` (braille
    /// spinner feedback, input suppressed). Starts as `Choice`; transitions to
    /// `Exiting` when the user activates quit or detach.
    pub phase: QuitConfirmPhase,
}

impl QuitConfirmState {
    /// Build the overlay state from the busy and total session counts.
    ///
    /// Focus starts on `2` (cancel) — the safe default — and the click hit-boxes
    /// start empty (`Rect::ZERO`); the first paint fills them. Phase starts as
    /// `Choice` (the normal navigable overlay).
    pub fn new(working: usize, total: usize) -> Self {
        Self {
            working,
            total,
            selected: 2,
            button_rects: Cell::new([Rect::ZERO; 3]),
            phase: QuitConfirmPhase::Choice,
        }
    }

    /// Build an EXITING-phase overlay for immediate exit feedback.
    ///
    /// Used by the client's render loop to show a braille spinner while waiting
    /// for the daemon to disconnect, and by the standalone event loop after
    /// the user activates quit/detach. `working`/`total` are display-only
    /// (carried for header text if needed); `selected` is unused (buttons are
    /// hidden).
    pub fn exiting() -> Self {
        Self {
            working: 0,
            total: 0,
            selected: 2,
            button_rects: Cell::new([Rect::ZERO; 3]),
            phase: QuitConfirmPhase::Exiting,
        }
    }

    /// Whether the overlay is in the exiting phase (input suppressed).
    pub fn is_exiting(&self) -> bool {
        self.phase == QuitConfirmPhase::Exiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_starts_in_choice_phase() {
        let s = QuitConfirmState::new(2, 5);
        assert_eq!(s.phase, QuitConfirmPhase::Choice);
        assert!(!s.is_exiting());
        assert_eq!(s.working, 2);
        assert_eq!(s.total, 5);
        assert_eq!(s.selected, 2); // cancel is safe default
    }

    #[test]
    fn exiting_state_has_exiting_phase() {
        let s = QuitConfirmState::exiting();
        assert_eq!(s.phase, QuitConfirmPhase::Exiting);
        assert!(s.is_exiting());
    }

    #[test]
    fn phase_transition_to_exiting() {
        let mut s = QuitConfirmState::new(1, 3);
        assert_eq!(s.phase, QuitConfirmPhase::Choice);
        s.phase = QuitConfirmPhase::Exiting;
        assert!(s.is_exiting());
    }
}
