//! Staged-attachment reconciliation + input-history recall methods, split out
//! of [`super::SessionRuntime`] for file size. Same `impl SessionRuntime` type
//! as `mod.rs`; no behaviour change from the pre-split single-file layout.

use super::SessionRuntime;

impl SessionRuntime {
    /// Re-sync `pending_attachments` to the `[Image #N]` markers still present in
    /// `input`: drop any staged attachment whose marker number no longer appears
    /// in the composer text. Called from the char-removal paths (`backspace` /
    /// `delete_forward`) so deleting an `[Image #N]` token live-drops its
    /// attachment card.
    ///
    /// Deliberately NOT called from insert paths (`push_char` / `insert_marker` /
    /// the attach helpers): an image's [`Attachment`] record is pushed a beat
    /// AFTER its marker is inserted, so reconciling on insert could drop a
    /// just-staged image before its record lands.
    pub fn reconcile_attachments(&mut self) {
        if self.pending_attachments.is_empty() {
            return; // nothing staged → nothing to drop (skips the scan on every keystroke)
        }
        let present = Self::marker_numbers(&self.input);
        self.pending_attachments
            .retain(|a| present.contains(&a.marker_n));
    }

    /// Collect the set of `N` values from every literal `[Image #N]` token in
    /// `text` — the exact format produced by `model::attachment`
    /// (`format!("[Image #{n}]")`). A run of ASCII digits sitting between the
    /// `[Image #` prefix and a closing `]` counts; a broken / half-typed marker
    /// matches nothing, so its attachment reconciles away.
    fn marker_numbers(text: &str) -> std::collections::HashSet<usize> {
        const PREFIX: &str = "[Image #";
        let mut out = std::collections::HashSet::new();
        for (i, _) in text.match_indices(PREFIX) {
            let after_prefix = &text[i + PREFIX.len()..];
            let digits: String = after_prefix
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.is_empty() || !after_prefix[digits.len()..].starts_with(']') {
                continue;
            }
            if let Ok(n) = digits.parse::<usize>() {
                out.insert(n);
            }
        }
        out
    }

    /// Move the staged composer attachments out for the message being submitted,
    /// leaving `pending_attachments` empty. Called at submit, paired with
    /// `take_input()`, so the markers and their attachment records travel
    /// together onto the user message.
    ///
    /// `submitted_text` is the text being sent (the composer `input` has already
    /// been emptied by `take_input` at this point, so we reconcile against the
    /// SUBMITTED text, not `self.input`). This is the send-time backstop: an
    /// attachment whose `[Image #N]` marker did not survive into the sent message
    /// — a marker broken by mid-token typing, or dropped by a history-recall
    /// replace — is discarded here so a marker-less image can never ship.
    pub fn take_attachments(&mut self, submitted_text: &str) -> Vec<crate::dto::chat::Attachment> {
        let present = Self::marker_numbers(submitted_text);
        self.pending_attachments
            .retain(|a| present.contains(&a.marker_n));
        std::mem::take(&mut self.pending_attachments)
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
