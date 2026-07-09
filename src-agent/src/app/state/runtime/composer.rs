//! Composer caret/text editing methods, split out of [`super::SessionRuntime`]
//! for file size. Same `impl SessionRuntime` type as `mod.rs`; no behaviour
//! change from the pre-split single-file layout.
//!
//! The caret `cursor` is a CHAR index into `input`; `byte_at` maps it to the
//! byte offset `String::insert`/`remove` need, so non-ASCII input can never
//! panic on a non-char-boundary.

use super::SessionRuntime;

impl SessionRuntime {
    /// Char count of the current input (the caret's upper bound).
    ///
    /// `pub(super)` (rather than private) so the sibling `attach` module's
    /// history-recall methods can reuse it too.
    pub(super) fn char_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset of char index `idx` (== input length when `idx >= char_len`).
    fn byte_at(&self, idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    /// Insert `c` at the caret and advance it (mid-text editing supported).
    ///
    /// The `palette_sel = 0` / `hist_idx = None` reset for the `/` palette is the
    /// caller's job ([`super::AppStateRest::push_char`] resets the GLOBAL
    /// `palette_sel` after delegating here); this clears only the per-session
    /// `hist_idx`.
    pub fn push_char(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.hist_idx = None;
    }

    /// Delete the char BEFORE the caret and retreat it; no-op at the start.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_at(self.cursor);
        self.input.remove(at);
        self.hist_idx = None;
        // A backspace may have deleted (part of) an `[Image #N]` marker; drop any
        // staged attachment whose marker is now gone so the card can't outlive it.
        self.reconcile_attachments();
    }

    /// Delete the char AT the caret (forward delete, the Delete key); no-op at the
    /// end of the input. Mirrors [`Self::backspace`] but does not move the caret.
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.char_len() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.input.remove(at);
        self.hist_idx = None;
        // Mirror `backspace`: a forward-delete may have removed (part of) an
        // `[Image #N]` marker, so re-sync the staged attachments to the text.
        self.reconcile_attachments();
    }

    /// Move the caret one char left (no-op at the start).
    pub fn cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the caret one char right (capped at the input length).
    pub fn cursor_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_len());
    }

    /// Jump the caret to the start of the input.
    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Jump the caret to the end of the input. Also called after any bulk replace
    /// (history recall, command/file completion) so the caret never dangles past
    /// the new (possibly shorter) text.
    pub fn cursor_end(&mut self) {
        self.cursor = self.char_len();
    }

    /// Move the caret up one visual line within a multi-line input.
    ///
    /// Returns `true` when the caret moved (so the caller can suppress history
    /// recall), or `false` when the caret is already on the first line (single-
    /// line input always returns `false`, preserving the existing history-recall
    /// behaviour).
    pub fn cursor_up(&mut self) -> bool {
        // Walk chars up to cursor to compute (line, col) in char units.
        let mut line: usize = 0;
        let mut col: usize = 0;
        for ch in self.input.chars().take(self.cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if line == 0 {
            return false; // already on the first line → let caller do history
        }
        // Collect char lengths per line (split on '\n').
        let line_lens: Vec<usize> = self.input.split('\n').map(|l| l.chars().count()).collect();
        let target_line = line - 1;
        let target_col = col.min(line_lens[target_line]);
        // Convert (target_line, target_col) back to a flat char index.
        self.cursor = line_lens[..target_line].iter().sum::<usize>()
            + target_line  // one '\n' per consumed line break
            + target_col;
        true
    }

    /// Move the caret down one visual line within a multi-line input.
    ///
    /// Returns `true` when the caret moved, `false` when already on the last
    /// line (single-line input always returns `false`).
    pub fn cursor_down(&mut self) -> bool {
        let line_lens: Vec<usize> = self.input.split('\n').map(|l| l.chars().count()).collect();
        let last_line = line_lens.len() - 1;
        // Walk chars up to cursor to compute (line, col).
        let mut line: usize = 0;
        let mut col: usize = 0;
        for ch in self.input.chars().take(self.cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if line == last_line {
            return false; // already on the last line → let caller do history
        }
        let target_line = line + 1;
        let target_col = col.min(line_lens[target_line]);
        self.cursor = line_lens[..target_line].iter().sum::<usize>()
            + target_line  // one '\n' per consumed line break
            + target_col;
        true
    }

    /// Take the input buffer, resetting the caret + per-session history index.
    /// The GLOBAL `palette_sel` reset is the caller's job (see
    /// [`super::AppStateRest::take_input`]).
    pub fn take_input(&mut self) -> String {
        self.hist_idx = None;
        self.cursor = 0;
        std::mem::take(&mut self.input)
    }

    /// Clear the composer in place (idle double-Esc with text present): empty the
    /// input, park the caret at 0, drop any staged attachments so a marker can't
    /// outlive its image, and leave history recall (`hist_idx = None`). The GLOBAL
    /// `palette_sel` reset, if any, is the caller's job.
    pub fn clear_composer(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.pending_attachments.clear();
        self.hist_idx = None;
    }

    /// Insert the literal marker string `s` (e.g. `"[Image #3]"`) at the caret,
    /// advancing it past the inserted run. Mirrors [`Self::push_char`]'s caret /
    /// history discipline so a bulk marker insert behaves like typing; the GLOBAL
    /// `palette_sel` reset is the caller's job (see
    /// [`super::AppStateRest::insert_marker`]).
    pub fn insert_marker(&mut self, s: &str) {
        let at = self.byte_at(self.cursor);
        self.input.insert_str(at, s);
        self.cursor += s.chars().count();
        self.hist_idx = None;
    }
}
