//! [`HelpState`] — the working state for the full-screen, searchable `/help`
//! reference + launcher (Help mode).
//!
//! Data-driven: `all` is built once from the two registries in
//! [`crate::controller::command`] — the [`COMMANDS`](crate::controller::command::COMMANDS)
//! table (each becomes a [`HelpKind::Command`] entry) and the
//! [`KEYBINDINGS`](crate::controller::command::KEYBINDINGS) table (each becomes a
//! [`HelpKind::Keybinding`] entry). The live `query` filters them with a
//! case-insensitive substring match, mirroring [`crate::app::mode::PickerState`].
//! Keystroke handling lives in [`crate::controller::input::help`]; rendering in
//! [`crate::view::help`].

use crate::controller::command::{COMMANDS, KEYBINDINGS};

/// Whether a help entry is a slash COMMAND (launchable with Enter) or a
/// KEYBINDING (reference only — Enter is a no-op).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpKind {
    /// A `/slash` command from the [`COMMANDS`] table. Enter runs it.
    Command,
    /// A keyboard shortcut from the [`KEYBINDINGS`] table. Reference only.
    Keybinding,
}

/// A single row in the help reference: a command or a keybinding.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    /// What this entry is (drives Enter behaviour + the row's accent tag).
    pub kind: HelpKind,
    /// The key column — the `/cmd` name (Command) or the chord (Keybinding).
    /// For a Command this is the leading-slash name verbatim from [`COMMANDS`],
    /// so it can be fed straight to [`crate::controller::command::parse`].
    pub key: String,
    /// The one-line description shown next to the key.
    pub desc: String,
}

/// Working state for the in-app `/help` reference + launcher.
///
/// `all` holds every command + keybinding (built once in [`HelpState::new`]);
/// `filtered_idx` is a subset of indices into `all` that match `query`.
/// `selected` is an index into `filtered_idx` (not into `all`), exactly like
/// [`crate::app::mode::PickerState`].
#[derive(Debug, Clone)]
pub struct HelpState {
    /// The user's live search string (updated on every keypress).
    pub query: String,
    /// Every help entry, unfiltered, in display order (commands then keys).
    pub all: Vec<HelpEntry>,
    /// Indices into `all` of entries that match the current `query`.
    /// Empty query → everything included (same order as `all`).
    pub filtered_idx: Vec<usize>,
    /// Cursor position within `filtered_idx` (not within `all`).
    pub selected: usize,
    /// The compiled-in koma version ([`crate::model::store::current_version`]),
    /// shown in the "Updating koma" block. Filled when `/help` opens; empty under
    /// [`HelpState::new`]/[`Default`] (which has no app-state access) until the
    /// command handler threads it in.
    pub current_version: String,
    /// `Some((latest_version, message))` iff a newer koma version is available
    /// (per [`crate::app::version::is_newer`]); `None` when up-to-date or no check
    /// has succeeded. Drives the "available [latest]" hint in the update block.
    pub update: Option<(String, Option<String>)>,
}

impl HelpState {
    /// Build the help state by aggregating both registries, then run the first
    /// filter pass (which with an empty query just includes everything).
    ///
    /// Order is COMMANDS first, then KEYBINDINGS, so the flat list reads
    /// commands-first; the view may still label the two groups.
    pub fn new() -> Self {
        let mut all: Vec<HelpEntry> = Vec::with_capacity(COMMANDS.len() + KEYBINDINGS.len());
        for (name, desc) in COMMANDS {
            all.push(HelpEntry {
                kind: HelpKind::Command,
                key: (*name).to_string(),
                desc: (*desc).to_string(),
            });
        }
        for (key, desc) in KEYBINDINGS {
            all.push(HelpEntry {
                kind: HelpKind::Keybinding,
                key: (*key).to_string(),
                desc: (*desc).to_string(),
            });
        }

        let mut s = Self {
            query: String::new(),
            all,
            filtered_idx: vec![],
            selected: 0,
            // Version fields default empty/None here (no app-state access); the
            // `/help` command handler fills them via `with_version` from `rest`.
            current_version: String::new(),
            update: None,
        };
        s.refilter();
        s
    }

    /// Populate the "Updating koma" version fields from app state, returning `self`
    /// so the `/help` command handler can build-then-fill in one expression.
    ///
    /// `current_version` is the compiled-in koma version; `update` is set only when
    /// `latest` is strictly newer than it (per [`crate::app::version::is_newer`]).
    pub fn with_version(
        mut self,
        current_version: String,
        update: Option<(String, Option<String>)>,
    ) -> Self {
        self.current_version = current_version;
        self.update = update;
        self
    }

    /// Rebuild `filtered_idx` from `all` using the current `query`.
    ///
    /// Matching is case-insensitive substring search on both the key and the
    /// description. After filtering, `selected` is clamped to the last valid
    /// index so it never points outside the filtered list (mirrors
    /// [`crate::app::mode::PickerState::refilter`]).
    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered_idx = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                q.is_empty()
                    || e.key.to_lowercase().contains(&q)
                    || e.desc.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp `selected` so it remains valid after the list shrinks.
        if self.selected >= self.filtered_idx.len() {
            self.selected = self.filtered_idx.len().saturating_sub(1);
        }
    }

    /// Move the cursor up one row (clamps at 0).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the cursor down one row (clamps at the last filtered entry).
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered_idx.len() {
            self.selected += 1;
        }
    }

    /// Return a reference to the currently highlighted entry, or `None` when
    /// the filtered list is empty.
    pub fn selected_entry(&self) -> Option<&HelpEntry> {
        self.filtered_idx
            .get(self.selected)
            .and_then(|&i| self.all.get(i))
    }
}

impl Default for HelpState {
    fn default() -> Self {
        Self::new()
    }
}
