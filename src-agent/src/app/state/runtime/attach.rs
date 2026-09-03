//! Staged-attachment reconciliation + input-history recall methods, split out
//! of [`super::SessionRuntime`] for file size. Same `impl SessionRuntime` type
//! as `mod.rs`; no behaviour change from the pre-split single-file layout.

use super::SessionRuntime;
use crate::dto::chat::AttachmentKind;

/// One marker span in composer text: byte range + kind + N.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerSpan {
    pub kind: AttachmentKind,
    pub n: usize,
    /// Inclusive start byte index into the source text.
    pub start: usize,
    /// Exclusive end byte index (past the closing `]`).
    pub end: usize,
}

/// Find every `[Image #N]` / `[Pasted Text #N]` span in `text` (byte offsets).
pub fn find_marker_spans(text: &str) -> Vec<MarkerSpan> {
    let mut out = Vec::new();
    collect_prefix_spans(text, "[Image #", AttachmentKind::Image, &mut out);
    collect_prefix_spans(text, "[Pasted Text #", AttachmentKind::PastedText, &mut out);
    out.sort_by_key(|s| s.start);
    out
}

fn collect_prefix_spans(
    text: &str,
    prefix: &str,
    kind: AttachmentKind,
    out: &mut Vec<MarkerSpan>,
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
        let Ok(n) = digits.parse::<usize>() else {
            continue;
        };
        let end = i + prefix.len() + digits.len() + 1;
        out.push(MarkerSpan {
            kind,
            n,
            start: i,
            end,
        });
    }
}

/// Char-index of the caret → nearest marker span.
///
/// Preference: span containing the caret; else the closest span by distance
/// to midpoint. `None` if there are no markers.
pub fn nearest_marker_span(text: &str, caret_chars: usize) -> Option<MarkerSpan> {
    let spans = find_marker_spans(text);
    if spans.is_empty() {
        return None;
    }
    let caret_byte = text
        .char_indices()
        .nth(caret_chars)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    if let Some(hit) = spans
        .iter()
        .find(|s| caret_byte >= s.start && caret_byte < s.end)
    {
        return Some(*hit);
    }
    let caret_c = caret_chars as isize;
    spans.into_iter().min_by_key(|s| {
        let start_c = text[..s.start].chars().count() as isize;
        let end_c = text[..s.end].chars().count() as isize;
        let mid = (start_c + end_c) / 2;
        (caret_c - mid).unsigned_abs()
    })
}

/// Collapse machine paste fences in `text` back to `[Pasted Text #N]` markers
/// for composer restore (rewind). Body bytes stay on disk via the attachment.
pub fn collapse_paste_fences_to_markers(text: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // No backref: end tag's n is captured separately and we trust the open n.
        crate::re_util::static_re(
            r#"(?s)<<<pasted_text n=(\d+) path="[^"]*">>>.*?<<<end_pasted_text n=\d+>>>"#,
        )
    });
    re.replace_all(text, |caps: &regex::Captures| {
        let n = &caps[1];
        format!("[Pasted Text #{n}]")
    })
    .into_owned()
}

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
        find_marker_spans(text)
            .into_iter()
            .map(|s| (s.kind, s.n))
            .collect()
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
    ///
    /// Clears `pending_attachments`: history-up only restores text, not chips
    /// (avoids stale image/paste cards from the live composer).
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
        self.pending_attachments.clear();
    }

    /// Recall the next (newer) sent user message; past the newest, restore the
    /// stashed live input and leave recall mode.
    ///
    /// Clears `pending_attachments` on each step (same rationale as
    /// [`Self::history_prev`]). Restoring the live stash also clears — the stash
    /// path never stored attachments.
    pub fn history_next(&mut self, users: &[String]) {
        match self.hist_idx {
            Some(i) if i + 1 < users.len() => {
                self.hist_idx = Some(i + 1);
                self.input = users[i + 1].clone();
                self.cursor = self.char_len();
                self.pending_attachments.clear();
            }
            Some(_) => {
                self.hist_idx = None;
                self.input = std::mem::take(&mut self.input_stash);
                self.cursor = self.char_len();
                self.pending_attachments.clear();
            }
            None => {}
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

    #[test]
    fn nearest_marker_prefers_span_under_caret() {
        let text = "aa [Pasted Text #1] bb [Image #2] cc";
        // Caret inside paste marker.
        let paste_start = text.find("[Pasted Text #1]").unwrap();
        let caret = text[..paste_start + 5].chars().count();
        let hit = nearest_marker_span(text, caret).unwrap();
        assert_eq!(hit.kind, AttachmentKind::PastedText);
        assert_eq!(hit.n, 1);
    }

    #[test]
    fn collapse_fences_to_markers() {
        let fenced = "hi\n<<<pasted_text n=3 path=\"pastes/03-paste.txt\">>>\nbody here\n<<<end_pasted_text n=3>>>\nbye";
        let got = collapse_paste_fences_to_markers(fenced);
        assert_eq!(got, "hi\n[Pasted Text #3]\nbye");
    }
}
