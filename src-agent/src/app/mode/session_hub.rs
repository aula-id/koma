//! State for the unified session hub (`/resume`, `Mode::SessionHub`).
//!
//! The hub merges the old `/swap` live-session picker and the `/resume` disk
//! picker into ONE two-pane overlay. Because the daemon owns the locks, an
//! explicit lock/unlock UI is meaningless — what actually matters is LIVE vs not:
//!
//! - **COOKING** pane — the currently-LIVE in-memory sessions (one row per
//!   [`crate::app::state::SessionRuntime`] in `AppStateRest::sessions`). This is
//!   exactly what `/swap` used to show. Enter switches the foreground to the
//!   chosen session (no abort, no lock churn).
//! - **HISTORY** pane — the on-disk sessions from [`crate::model::store::list_sessions`]
//!   MINUS any whose path is already live (dedup: a live session shows ONLY in
//!   cooking). Enter loads the chosen session into a NEW appended tab.
//!
//! The hub is a fixed snapshot built when `/resume` opens. One pane is focused at
//! a time; Tab toggles focus, Up/Down move the selection within the focused pane
//! (each pane scrolls independently inside its own half), Enter acts on the
//! focused pane's selection, Esc closes back to Chat. Selection/scroll state lives
//! here; keystroke handling lives in [`crate::controller::input::handle_session_hub`].

use std::path::PathBuf;
use std::time::SystemTime;

/// Which of the two panes currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubPane {
    /// The LIVE in-memory sessions (top half). Enter switches foreground.
    Cooking,
    /// The on-disk sessions (bottom half). Enter loads into a new tab.
    History,
}

/// The kind of a cooking entry — either a real live session or a synthetic
/// "[+ new session]" action row pinned at the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// A real live session (has a `sessions` index).
    Session,
    /// Synthetic "[+ new session]" row — Enter triggers `/new`.
    NewSession,
}

/// One row in the COOKING pane: a snapshot of a single live session, or a
/// synthetic action row.
pub struct CookingEntry {
    /// Index of this session in `AppStateRest::sessions`. Carried out on Enter so
    /// the foreground switch sets `foreground = idx` directly. Stable for the
    /// hub's lifetime — `sessions` is only ever appended to / has its foreground
    /// changed, never reordered or removed.
    ///
    /// For `NewSession` entries this is `usize::MAX` (never a valid index).
    pub idx: usize,
    /// The kind of entry (real session vs synthetic new-session row).
    pub kind: SessionKind,
    /// Display name of the session (its [`crate::model::session::Session::name`]),
    /// or a placeholder when the slot has no session yet.
    pub name: String,
    /// Whether the session had work in flight when the snapshot was taken
    /// (`SessionRuntime::is_working()`), shown as a ● working / ○ ready marker.
    /// Ignored for `NewSession` entries.
    pub working: bool,
    /// Whether this is the current foreground session, tagged `(current)`.
    /// Ignored for `NewSession` entries.
    pub is_foreground: bool,
}

/// One row in the HISTORY pane: an on-disk session not currently live.
pub struct HistoryEntry {
    /// The session's on-disk directory path — the canonical identity used to load
    /// it (and to re-check its lock) when Enter opens it.
    pub path: PathBuf,
    /// Display name of the session.
    pub name: String,
    /// Last-active time (the registry `updated_at`), shown as a relative age.
    pub last_active: SystemTime,
}

/// State for the two-pane session hub.
///
/// `cooking` is the snapshot of live sessions (in `sessions` order); `history` is
/// the on-disk sessions minus the live ones. `focus` is the active pane; each
/// pane carries its OWN cursor so switching focus preserves where you were.
/// Neither list is searchable — both are short navigable lists.
pub struct SessionHub {
    /// LIVE in-memory sessions, in `AppStateRest::sessions` order.
    pub cooking: Vec<CookingEntry>,
    /// On-disk sessions not currently live (history = on-disk minus cooking).
    pub history: Vec<HistoryEntry>,
    /// Which pane has keyboard focus.
    pub focus: HubPane,
    /// Cursor within `cooking` (used when `focus == Cooking`).
    pub cooking_selected: usize,
    /// Cursor within `history` (used when `focus == History`).
    pub history_selected: usize,
}

impl SessionHub {
    /// Toggle focus between the two panes. The non-focused pane keeps its cursor.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            HubPane::Cooking => HubPane::History,
            HubPane::History => HubPane::Cooking,
        };
    }

    /// Move the focused pane's cursor up one row (clamps at 0).
    pub fn move_up(&mut self) {
        match self.focus {
            HubPane::Cooking => {
                self.cooking_selected = self.cooking_selected.saturating_sub(1);
            }
            HubPane::History => {
                self.history_selected = self.history_selected.saturating_sub(1);
            }
        }
    }

    /// Move the focused pane's cursor down one row (clamps at the last entry).
    pub fn move_down(&mut self) {
        match self.focus {
            HubPane::Cooking => {
                if self.cooking_selected + 1 < self.cooking.len() {
                    self.cooking_selected += 1;
                }
            }
            HubPane::History => {
                if self.history_selected + 1 < self.history.len() {
                    self.history_selected += 1;
                }
            }
        }
    }

    /// The highlighted COOKING entry, or `None` if that pane is empty. Read on
    /// Enter while the cooking pane is focused to resolve the target `sessions`
    /// index for the foreground switch.
    pub fn selected_cooking(&self) -> Option<&CookingEntry> {
        self.cooking.get(self.cooking_selected)
    }

    /// The highlighted HISTORY entry, or `None` if that pane is empty. Read on
    /// Enter while the history pane is focused to resolve the path to load.
    pub fn selected_history(&self) -> Option<&HistoryEntry> {
        self.history.get(self.history_selected)
    }
}
