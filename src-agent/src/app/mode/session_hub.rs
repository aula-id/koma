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
#[derive(Clone)]
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
    /// The session's UUID (the on-disk dir name / socket key), used by the
    /// client-side swapper to address the chosen session-daemon directly. The
    /// synthetic `NewSession` row carries `None`; a real session carries
    /// `Some(uuid)`.
    // Populated now (daemon builder + client `build_local_hub`) but not yet READ —
    // the swapper that consumes it lands next commit. Allow until then.
    #[allow(dead_code)]
    pub session_id: Option<String>,
}

/// One row in the HISTORY pane: an on-disk session not currently live.
#[derive(Clone)]
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
///
/// `Clone` is derived so the client-side swapper can snapshot a hub into a throwaway
/// shadow `AppState` for the existing pure-from-snapshot renderer each frame.
#[derive(Clone)]
pub struct SessionHub {
    /// LIVE in-memory sessions, in `AppStateRest::sessions` order.
    pub cooking: Vec<CookingEntry>,
    /// On-disk sessions not currently live (history = on-disk minus cooking).
    pub history: Vec<HistoryEntry>,
    /// Which pane has keyboard focus.
    pub focus: HubPane,
    /// Cursor within `cooking` (used when `focus == Cooking`).
    pub cooking_selected: usize,
    /// Cursor within `history_filtered` (used when `focus == History`) — i.e. an
    /// index into the FILTERED view, not into `history` directly. Resolve the real
    /// entry via `history[history_filtered[history_selected]]`.
    pub history_selected: usize,
    /// Live search query over the history pane (printable keys typed while the
    /// History pane is focused). Empty = show all.
    pub history_query: String,
    /// Indices into `history` that match `history_query` (identity when empty).
    /// `history_selected` indexes into THIS, not `history`.
    pub history_filtered: Vec<usize>,
    /// When set, a kill is awaiting confirmation: the value is the position in
    /// `cooking` of the session to act on. While Some, the hub shows a confirm
    /// bar and the input handler only accepts confirm/cancel.
    pub pending_kill: Option<usize>,
}

impl SessionHub {
    /// Toggle focus between the two panes. The non-focused pane keeps its cursor.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            HubPane::Cooking => HubPane::History,
            HubPane::History => HubPane::Cooking,
        };
    }

    /// Move the focused pane's cursor up one row (clamps at 0). The History pane
    /// scrolls over the FILTERED view (`history_filtered`).
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

    /// Move the focused pane's cursor down one row (clamps at the last entry). The
    /// History pane scrolls over the FILTERED view (`history_filtered`).
    pub fn move_down(&mut self) {
        match self.focus {
            HubPane::Cooking => {
                if self.cooking_selected + 1 < self.cooking.len() {
                    self.cooking_selected += 1;
                }
            }
            HubPane::History => {
                if self.history_selected + 1 < self.history_filtered.len() {
                    self.history_selected += 1;
                }
            }
        }
    }

    /// Rebuild `history_filtered` from the current `history_query`: identity when
    /// the query is empty, else the indices whose `history[i].name` contains the
    /// query case-insensitively. Clamps `history_selected` into the new filtered
    /// range (0 when empty) so the cursor never dangles past the visible rows.
    pub fn refilter_history(&mut self) {
        if self.history_query.is_empty() {
            self.history_filtered = (0..self.history.len()).collect();
        } else {
            let q = self.history_query.to_lowercase();
            self.history_filtered = self
                .history
                .iter()
                .enumerate()
                .filter(|(_, e)| e.name.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.history_selected = self
            .history_selected
            .min(self.history_filtered.len().saturating_sub(1));
    }

    /// The highlighted COOKING entry, or `None` if that pane is empty. Read on
    /// Enter while the cooking pane is focused to resolve the target `sessions`
    /// index for the foreground switch.
    pub fn selected_cooking(&self) -> Option<&CookingEntry> {
        self.cooking.get(self.cooking_selected)
    }

    /// The REAL index into `history` of the highlighted history row (the filtered
    /// cursor resolved back to its underlying `history` position), or `None` when
    /// the filtered view is empty. Carried out on Enter so the runtime opens the
    /// row the user actually sees regardless of the active filter.
    pub fn selected_history_real_idx(&self) -> Option<usize> {
        self.history_filtered.get(self.history_selected).copied()
    }
}
