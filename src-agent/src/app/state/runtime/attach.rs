//! Staged-attachment reconciliation + input-history recall methods, split out
//! of [`super::SessionRuntime`] for file size. Same `impl SessionRuntime` type
//! as `mod.rs`; no behaviour change from the pre-split single-file layout.

use super::SessionRuntime;
use crate::dto::chat::AttachmentKind;

impl SessionRuntime {
    /// Re-sync `pending_attachments` to the `[Image #N]` / `[Pasted Text #N]`
    /// markers still present in `input`: drop any staged attachment whose
    /// `(kind, marker_n)` no longer appears in the composer text. Called from
    /// the char-removal paths (`backspace` / `delete_forward`) so deleting a
    /// marker live-drops its attachment card.
    ///
    /// Deliberately NOT called from insert paths (`push_char` / `insert_marker` /
    /// the attach helpers): an attachment's record is pushed a beat AFTER its
    /// marker is inserted, so reconciling on insert could drop a just-staged
    /// item before its record lands.
    pub fn reconcile_attachments(&mut self) {
        if self.pending_attachments.is_empty() {
            return; // nothing staged → nothing to drop (skips the scan on every keystroke)
        }
        let present = Self::marker_keys(&self.input);
        self.pending_attachments
            .retain(|a| present.contains(&(a.kind, a.marker_n)));
    }

    /// Collect `(kind, N)` for every literal `[Image #N]` and `[Pasted Text #N]`
    /// token in `text`. Kind-aware so image #1 and paste #1 (independent seq
    /// spaces) do not collide during reconcile/take.
    fn marker_keys(text: &str) -> std::collections::HashSet<(AttachmentKind, usize)> {
        let mut out = std::collections::HashSet::new();
        scan_markers(text, "[Image #", AttachmentKind::Image, &mut out);
        scan_markers(text, "[Pasted Text #", AttachmentKind::PastedText, &mut out);
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
    /// attachment whose marker did not survive into the sent message — a marker
    /// broken by mid-token typing, or dropped by a history-recall replace — is
    /// discarded here so a marker-less attachment can never ship.
    pub fn take_attachments(&mut self, submitted_text: &str) -> Vec<crate::dto::chat::Attachment> {
        let present = Self::marker_keys(submitted_text);
        self.pending_attachments
            .retain(|a| present.contains(&(a.kind, a.marker_n)));
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

/// Scan `text` for `PREFIX{digits}]` tokens and insert `(kind, n)` into `out`.
fn scan_markers(
    text: &str,
    prefix: &str,
    kind: AttachmentKind,
    out: &mut std::collections::HashSet<(AttachmentKind, usize)>,
) {
    for (i, _) in text.match_indices(prefix) {
        let after_prefix = &text[i + prefix.len()..];
        let digits: String = after_prefix
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() || !after_prefix[digits.len()..].starts_with(']') {
            continue;
        }
        if let Ok(n) = digits.parse::<usize>() {
            out.insert((kind, n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::chat::{Attachment, AttachmentKind};

    #[test]
    fn marker_keys_finds_image_and_paste() {
        let keys = SessionRuntime::marker_keys(
            "see [Image #2] and [Pasted Text #1] plus [Image #2] again",
        );
        assert!(keys.contains(&(AttachmentKind::Image, 2)));
        assert!(keys.contains(&(AttachmentKind::PastedText, 1)));
        assert!(!keys.contains(&(AttachmentKind::Image, 1)));
    }

    #[test]
    fn take_attachments_is_kind_aware() {
        let mut rt = SessionRuntime::new();
        rt.pending_attachments = vec![
            Attachment {
                kind: AttachmentKind::Image,
                marker_n: 1,
                rel_path: "images/01-a.png".into(),
                mime: "image/png".into(),
            },
            Attachment {
                kind: AttachmentKind::PastedText,
                marker_n: 1,
                rel_path: "pastes/01-paste.txt".into(),
                mime: "text/plain".into(),
            },
        ];
        // Only the paste marker survives → image #1 must drop even though n=1 matches paste.
        let taken = rt.take_attachments("body [Pasted Text #1]");
        assert_eq!(taken.len(), 1);
        assert!(taken[0].is_pasted_text());
        assert!(rt.pending_attachments.is_empty());
    }
}
