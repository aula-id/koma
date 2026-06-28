//! Input-editing and history methods on [`super::AppStateRest`].
//!
//! The caret (`cursor`) is a CHAR index into `input`. `byte_at` maps a char
//! index to the byte offset `String::insert`/`remove` need, so non-ASCII input
//! can never panic on a non-char-boundary. `char_len` is the count edits clamp
//! against.

use super::rest::AppStateRest;

impl AppStateRest {
    /// Char count of the current input (the caret's upper bound).
    fn char_len(&self) -> usize {
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
    pub fn push_char(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.palette_sel = 0;
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
        self.palette_sel = 0;
        self.hist_idx = None;
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

    pub fn take_input(&mut self) -> String {
        self.palette_sel = 0;
        self.hist_idx = None;
        self.cursor = 0;
        std::mem::take(&mut self.input)
    }

    /// Insert the literal marker string `s` (e.g. `"[Image #3]"`) at the caret,
    /// advancing it past the inserted run. Mirrors [`Self::push_char`]'s caret /
    /// palette / history discipline so a bulk marker insert behaves like typing.
    pub fn insert_marker(&mut self, s: &str) {
        let at = self.byte_at(self.cursor);
        self.input.insert_str(at, s);
        self.cursor += s.chars().count();
        self.palette_sel = 0;
        self.hist_idx = None;
    }

    /// Move the staged composer attachments out for the message being submitted,
    /// leaving `pending_attachments` empty. Called at submit, paired with
    /// `take_input()`, so the markers and their attachment records travel
    /// together onto the user message.
    pub fn take_attachments(&mut self) -> Vec<crate::dto::chat::Attachment> {
        std::mem::take(&mut self.pending_attachments)
    }

    /// Ingest the image file at `path` into the active session's `images/` dir,
    /// stage the produced [`Attachment`](crate::dto::chat::Attachment), and insert
    /// its `[Image #N]` marker at the caret. Returns `true` on success.
    ///
    /// Returns `false` (composer untouched) when there is no active session or the
    /// ingest fails (missing file / not a recognised image / write error), so the
    /// caller can fall back to inserting the raw pasted text. The session's images
    /// dir is `<session.path>/images/`.
    pub fn try_attach_image_path(&mut self, path: &str) -> bool {
        let Some(images_dir) = self.fg().session.as_ref().map(|s| s.images_dir()) else {
            return false;
        };
        match crate::model::attachment::ingest_image_from_path(
            &images_dir,
            std::path::Path::new(path),
        ) {
            Ok((att, marker)) => {
                self.insert_marker(&marker);
                self.pending_attachments.push(att);
                true
            }
            Err(_) => false,
        }
    }

    /// Ingest raw image `bytes` (already read from the clipboard) into the active
    /// session's `images/` dir with the given `mime` and `basename`, stage the
    /// produced [`Attachment`](crate::dto::chat::Attachment), and insert its
    /// `[Image #N]` marker at the caret. Returns `true` on success.
    ///
    /// Returns `false` (composer untouched) when there is no active session or the
    /// ingest fails (not a recognised image / write error). The caller should
    /// toast any failure independently.
    pub fn try_attach_image_bytes(&mut self, bytes: Vec<u8>, mime: &str, basename: &str) -> bool {
        let Some(images_dir) = self.fg().session.as_ref().map(|s| s.images_dir()) else {
            return false;
        };
        match crate::model::attachment::ingest_image_from_raw_bytes(
            &images_dir,
            &bytes,
            mime,
            basename,
        ) {
            Ok((att, marker)) => {
                self.insert_marker(&marker);
                self.pending_attachments.push(att);
                true
            }
            Err(_) => false,
        }
    }

    /// Recall the previous (older) sent user message into the input. `users` is
    /// the session's user messages oldest-first.
    pub fn history_prev(&mut self, users: &[String]) {
        if users.is_empty() {
            return;
        }
        let next = match self.hist_idx {
            None => {
                self.input_stash = self.input.clone();
                users.len() - 1
            }
            Some(0) => return, // already at the oldest
            Some(i) => i - 1,
        };
        self.hist_idx = Some(next);
        self.input = users[next].clone();
        self.cursor = self.char_len();
    }

    /// Recall the next (newer) sent user message; past the newest, restore the
    /// stashed live input and leave recall mode.
    pub fn history_next(&mut self, users: &[String]) {
        match self.hist_idx {
            Some(i) if i + 1 < users.len() => {
                self.hist_idx = Some(i + 1);
                self.input = users[i + 1].clone();
                self.cursor = self.char_len();
            }
            Some(_) => {
                self.hist_idx = None;
                self.input = std::mem::take(&mut self.input_stash);
                self.cursor = self.char_len();
            }
            None => {}
        }
    }
}
