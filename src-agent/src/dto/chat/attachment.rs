//! [`Attachment`] — a file attached to a chat message (image or pasted text).
//!
//! An attachment is a small record that LINKS a message to an on-disk file; the
//! bytes themselves are NEVER stored in the message or `messages.json`.
//! They live under `<session>/images/NN-name.ext` or `<session>/pastes/NN-paste.txt`
//! (see [`crate::model::store::session_images_dir`] /
//! [`crate::model::store::session_pastes_dir`] + the ingest core in
//! [`crate::model::attachment`]). The record carries the kind, the relative path,
//! the sniffed mime type, and the marker number `N` that ties it to the literal
//! `[Image #N]` or `[Pasted Text #N]` token in the message text.
//!
//! Image base64 data-URLs the model receives are DERIVED from the on-disk file at
//! send time (see `to_wire_with_images`). Pasted text is expanded into machine
//! fences in the message content at submit (body still lives on disk for
//! rewind/edit). Resume re-reads from disk and the link survives across runs.

use serde::{Deserialize, Serialize};

/// What kind of staged/persisted attachment this record is.
///
/// Defaults to [`AttachmentKind::Image`] so older `messages.json` rows without a
/// `kind` field keep deserializing as images.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// Image under `images/`; drives multimodal wire parts.
    #[default]
    Image,
    /// Collapsed pasted text under `pastes/`; body expanded into fences on send.
    PastedText,
}

/// One file attached to a [`super::ChatMessage`].
///
/// Persisted in `messages.json` (and the sqlite msglog row's content stays the
/// plain text / fenced body, attachments ride the JSON message). The bytes are
/// on disk under the session's `images/` or `pastes/` dir; this record is the
/// durable link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    /// Discriminator: image vs pasted text. Older JSON without this field
    /// deserializes as [`AttachmentKind::Image`].
    #[serde(default)]
    pub kind: AttachmentKind,
    /// The marker number `N` that matches the literal `[Image #N]` or
    /// `[Pasted Text #N]` token in the owning message's `content`. Monotonic
    /// **per kind** (separate `.seq` counters under `images/` and `pastes/`).
    pub marker_n: usize,
    /// Path RELATIVE to the session directory: `images/NN-name.ext` or
    /// `pastes/NN-paste.txt`. Resolved against `<session>/` at send/edit time.
    pub rel_path: String,
    /// Sniffed MIME type (e.g. `image/png`, `text/plain`), used for image
    /// data-URL prefixes and GUI chip kind derivation.
    pub mime: String,
}

impl Attachment {
    /// The original on-disk basename (`NN-name.ext`) — the trailing path segment
    /// of `rel_path`. Shown in warn cards + the model-visible strip warning.
    pub fn file_name(&self) -> &str {
        self.rel_path
            .rsplit('/')
            .next()
            .unwrap_or(self.rel_path.as_str())
    }

    /// Whether this attachment is an image (multimodal wire / GUI image chip).
    pub fn is_image(&self) -> bool {
        matches!(self.kind, AttachmentKind::Image)
    }

    /// Whether this attachment is collapsed pasted text.
    pub fn is_pasted_text(&self) -> bool {
        matches!(self.kind, AttachmentKind::PastedText)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_json_without_kind_deserializes_as_image() {
        let json = r#"{
            "marker_n": 2,
            "rel_path": "images/02-shot.png",
            "mime": "image/png"
        }"#;
        let att: Attachment = serde_json::from_str(json).expect("deserialize");
        assert_eq!(att.kind, AttachmentKind::Image);
        assert_eq!(att.marker_n, 2);
        assert_eq!(att.rel_path, "images/02-shot.png");
        assert_eq!(att.mime, "image/png");
        assert!(att.is_image());
        assert!(!att.is_pasted_text());
    }

    #[test]
    fn pasted_text_kind_round_trips() {
        let att = Attachment {
            kind: AttachmentKind::PastedText,
            marker_n: 1,
            rel_path: "pastes/01-paste.txt".to_string(),
            mime: "text/plain".to_string(),
        };
        let json = serde_json::to_string(&att).expect("serialize");
        let back: Attachment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, att);
        assert!(back.is_pasted_text());
    }
}
