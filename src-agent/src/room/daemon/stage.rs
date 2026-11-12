//! What the user pastes into the composer before the message is sent.
//!
//! The composer is the client's, but the files are the daemon's: it reads the
//! clipboard, copies an image the user pasted a path to, and keeps a long paste
//! on disk, exactly as the coding chat does (same ingest core, same markers). A
//! pasted item goes back to the client as the marker to put in the draft
//! (`[Image #1]`, `[Pasted Text #2]`). When the message is sent, the markers in
//! it are resolved: pasted text is expanded in place and the images travel with
//! the message to the lane that reads it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::dto::chat::Attachment;
use crate::model::attachment::{
    expand_paste_markers, ingest_image_from_path, ingest_image_from_raw_bytes, ingest_paste_text,
    should_collapse_paste, PASTE_SOFT_MAX_BYTES,
};
use crate::room::lane::UserImage;

/// What a paste turned into.
#[derive(Debug, Clone, PartialEq)]
pub enum Pasted {
    /// Text to put into the draft at the caret: the text itself, or a marker.
    Insert(String),
    /// Nothing was added; the reason is shown to the user.
    Refused(String),
}

pub struct Staging {
    dir: PathBuf,
    pending: Mutex<Vec<Attachment>>,
}

impl Staging {
    /// Stage under `dir` (`images/` and `pastes/` live inside it).
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf(), pending: Mutex::new(Vec::new()) }
    }

    /// The session directory this room lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Attachment>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A text paste, routed like the chat's: a path to an image becomes the
    /// image, long or multi-line text becomes a chip, anything else stays inline.
    pub fn paste_text(&self, text: &str) -> Pasted {
        if let Some(path) = crate::controller::input::image_path_paste(text) {
            return match ingest_image_from_path(&self.dir.join("images"), Path::new(&path)) {
                Ok((attachment, marker)) => {
                    self.lock().push(attachment);
                    Pasted::Insert(marker)
                }
                Err(e) => Pasted::Refused(format!("could not attach that image: {e}")),
            };
        }
        let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
        if !should_collapse_paste(&cleaned) {
            return Pasted::Insert(cleaned);
        }
        match ingest_paste_text(&self.dir.join("pastes"), &cleaned) {
            Ok((attachment, marker)) => {
                self.lock().push(attachment);
                Pasted::Insert(marker)
            }
            Err(_) => Pasted::Refused(format!("paste too large (max {PASTE_SOFT_MAX_BYTES} bytes)")),
        }
    }

    /// An image from the clipboard (PNG bytes).
    pub fn paste_image(&self, bytes: &[u8]) -> Pasted {
        if !infer::get(bytes).is_some_and(|kind| kind.mime_type().starts_with("image/")) {
            return Pasted::Refused("clipboard image: the clipboard does not hold an image".into());
        }
        match ingest_image_from_raw_bytes(&self.dir.join("images"), bytes, "image/png", "pasted.png") {
            Ok((attachment, marker)) => {
                self.lock().push(attachment);
                Pasted::Insert(marker)
            }
            Err(e) => Pasted::Refused(format!("clipboard image: {e}")),
        }
    }

    /// Resolve a message that is being sent: expand the pasted-text markers and
    /// collect the images whose `[Image #n]` marker is still in the text. Staging
    /// is emptied; an item whose marker the user deleted is dropped.
    pub fn take(&self, text: &str) -> (String, Vec<UserImage>) {
        let pending = std::mem::take(&mut *self.lock());
        if pending.is_empty() {
            return (text.to_string(), Vec::new());
        }
        let images = pending
            .iter()
            .filter(|a| a.is_image() && text.contains(&format!("[Image #{}]", a.marker_n)))
            .map(|a| UserImage { n: a.marker_n, path: self.dir.join(&a.rel_path) })
            .collect();
        (expand_paste_markers(text, &pending, &self.dir), images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0,
        0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn staging(name: &str) -> (Staging, PathBuf) {
        let dir = std::env::temp_dir().join(format!("koma-stage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Staging::new(&dir), dir)
    }

    #[test]
    fn a_short_paste_stays_inline_and_a_long_one_becomes_a_chip() {
        let (stage, _) = staging("text");
        assert_eq!(stage.paste_text("pwd"), Pasted::Insert("pwd".into()));
        let long = "line\n".repeat(30);
        let Pasted::Insert(marker) = stage.paste_text(&long) else { panic!("refused") };
        assert_eq!(marker, "[Pasted Text #1]");
        let (expanded, images) = stage.take(&format!("look at {marker}"));
        assert!(expanded.contains("<<<pasted_text n=1") && expanded.contains(&long));
        assert!(images.is_empty());
    }

    #[test]
    fn a_pasted_image_travels_with_the_message_that_still_names_it() {
        let (stage, dir) = staging("image");
        let Pasted::Insert(marker) = stage.paste_image(PNG) else { panic!("refused") };
        assert_eq!(marker, "[Image #1]");
        let (text, images) = stage.take(&format!("what is {marker}?"));
        assert_eq!(text, "what is [Image #1]?");
        assert_eq!(images.len(), 1);
        assert!(images[0].path.starts_with(&dir) && images[0].path.is_file());
        // Staging is spent after a send.
        assert!(stage.take("again [Image #1]").1.is_empty());
    }

    #[test]
    fn a_deleted_marker_drops_its_image_and_a_path_paste_attaches_the_file() {
        let (stage, dir) = staging("path");
        stage.paste_image(PNG);
        assert!(stage.take("no marker here").1.is_empty());
        let file = dir.join("shot.png");
        std::fs::write(&file, PNG).unwrap();
        let Pasted::Insert(marker) = stage.paste_text(&file.to_string_lossy()) else { panic!("refused") };
        assert!(marker.starts_with("[Image #"));
        assert!(matches!(stage.paste_image(b"not an image at all"), Pasted::Refused(_)));
    }
}
