//! Composer wrappers on [`super::AppStateRest`].
//!
//! The composer state (`input`, `cursor`, `pending_attachments`, `hist_idx`,
//! `input_stash`) now lives on the foreground [`SessionRuntime`], so the actual
//! caret/history editing methods are defined on that type. The wrappers here
//! delegate to `fg_mut()` and additionally reset the GLOBAL `/` palette
//! selection (`palette_sel`, which stays on `AppStateRest`) for the edits that
//! used to clear it — keeping the observable behaviour byte-identical to when
//! these methods lived directly on `AppStateRest`.

use super::rest::AppStateRest;

impl AppStateRest {
    /// Insert `c` at the caret (foreground composer) and reset the `/` palette
    /// selection. See [`super::SessionRuntime::push_char`].
    pub fn push_char(&mut self, c: char) {
        self.fg_mut().push_char(c);
        self.palette_sel = 0;
    }

    /// Delete the char BEFORE the caret and reset the `/` palette selection.
    /// See [`super::SessionRuntime::backspace`].
    pub fn backspace(&mut self) {
        self.fg_mut().backspace();
        self.palette_sel = 0;
    }

    /// Forward-delete the char AT the caret and reset the `/` palette selection.
    /// See [`super::SessionRuntime::delete_forward`].
    pub fn delete_forward(&mut self) {
        self.fg_mut().delete_forward();
        self.palette_sel = 0;
    }

    /// Move the caret one char left (no-op at the start).
    pub fn cursor_left(&mut self) {
        self.fg_mut().cursor_left();
    }

    /// Move the caret one char right (capped at the input length).
    pub fn cursor_right(&mut self) {
        self.fg_mut().cursor_right();
    }

    /// Jump the caret to the start of the input.
    pub fn cursor_home(&mut self) {
        self.fg_mut().cursor_home();
    }

    /// Jump the caret to the end of the input. Also called after any bulk replace
    /// (history recall, command/file completion) so the caret never dangles past
    /// the new (possibly shorter) text.
    pub fn cursor_end(&mut self) {
        self.fg_mut().cursor_end();
    }

    /// Move the caret up one visual line within a multi-line input. Returns
    /// `true` when the caret moved (so the caller can suppress history recall).
    pub fn cursor_up(&mut self) -> bool {
        self.fg_mut().cursor_up()
    }

    /// Move the caret down one visual line within a multi-line input. Returns
    /// `true` when the caret moved.
    pub fn cursor_down(&mut self) -> bool {
        self.fg_mut().cursor_down()
    }

    /// Take the input buffer and reset the `/` palette selection. See
    /// [`super::SessionRuntime::take_input`].
    pub fn take_input(&mut self) -> String {
        let taken = self.fg_mut().take_input();
        self.palette_sel = 0;
        taken
    }

    /// Insert the literal marker string `s` at the caret and reset the `/`
    /// palette selection. See [`super::SessionRuntime::insert_marker`].
    pub fn insert_marker(&mut self, s: &str) {
        self.fg_mut().insert_marker(s);
        self.palette_sel = 0;
    }

    /// Move the staged composer attachments out for the message being submitted.
    /// See [`super::SessionRuntime::take_attachments`].
    pub fn take_attachments(&mut self) -> Vec<crate::dto::chat::Attachment> {
        self.fg_mut().take_attachments()
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
                self.fg_mut().pending_attachments.push(att);
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
                self.fg_mut().pending_attachments.push(att);
                true
            }
            Err(_) => false,
        }
    }

    /// Recall the previous (older) sent user message into the input. `users` is
    /// the session's user messages oldest-first.
    pub fn history_prev(&mut self, users: &[String]) {
        self.fg_mut().history_prev(users);
    }

    /// Recall the next (newer) sent user message; past the newest, restore the
    /// stashed live input and leave recall mode.
    pub fn history_next(&mut self, users: &[String]) {
        self.fg_mut().history_next(users);
    }
}
