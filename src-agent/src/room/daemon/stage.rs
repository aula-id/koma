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
    expand_paste_markers, ingest_image_bytes, ingest_image_from_path, ingest_image_from_raw_bytes, ingest_paste_text,
    should_collapse_paste, sniff_image_mime, PASTE_SOFT_MAX_BYTES,
};
use crate::room::lane::UserImage;
use crate::room::wire::SdkAttachment;

/// At most this many attachments travel with one composed message.
pub const COMPOSE_MAX_ITEMS: usize = 16;

/// At most this many bytes of attachments travel with one composed message.
pub const COMPOSE_MAX_BYTES: usize = 32 * 1024 * 1024;

/// A composed message: the text for the actor, the images that travel with it,
/// and what each attachment became, in order.
pub type Composed = (String, Vec<UserImage>, Vec<String>);

/// A refused composition: a short code and the reason, for the client.
pub type Refusal = (String, String);

/// The message text with each insert on a line of its own after it: what the
/// user's line reads like before the pasted text is expanded.
pub fn with_inserts(text: &str, inserts: &[String]) -> String {
    let mut out = text.to_string();
    for insert in inserts {
        out.push('\n');
        out.push_str(insert);
    }
    out
}

fn refusal(code: &str, error: impl Into<String>) -> Refusal {
    (code.to_string(), error.into())
}

/// An attachment that passed every check and only needs to be stored.
enum Checked {
    Image { name: String, bytes: Vec<u8> },
    Chip(String),
    Inline(String),
}

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

    /// Compose a whole message with its attachments, with the same ingest and
    /// markers a paste uses. Each attachment's insert goes on a line of its own
    /// after `text` ([`with_inserts`]). Every attachment is checked before the
    /// first one is stored, and the items live on a list of this message
    /// alone: what [`Staging::paste_text`] staged stays staged.
    pub fn compose(&self, text: &str, attachments: &[SdkAttachment]) -> Result<Composed, Refusal> {
        if text.trim().is_empty() {
            return Err(refusal("empty_text", "the message is empty"));
        }
        if attachments.len() > COMPOSE_MAX_ITEMS {
            return Err(refusal("too_many_attachments", format!("at most {COMPOSE_MAX_ITEMS} attachments")));
        }
        let mut total = 0usize;
        let mut checked = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            let item = match attachment {
                SdkAttachment::ImageFile { path } => check_image(Path::new(path), COMPOSE_MAX_BYTES - total)?,
                SdkAttachment::PastedText { text } => {
                    let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
                    if cleaned.len() > PASTE_SOFT_MAX_BYTES {
                        return Err(refusal("too_large", format!("paste too large (max {PASTE_SOFT_MAX_BYTES} bytes)")));
                    }
                    if should_collapse_paste(&cleaned) {
                        Checked::Chip(cleaned)
                    } else {
                        Checked::Inline(cleaned)
                    }
                }
            };
            total += match &item {
                Checked::Image { bytes, .. } => bytes.len(),
                Checked::Chip(text) | Checked::Inline(text) => text.len(),
            };
            if total > COMPOSE_MAX_BYTES {
                return Err(refusal("too_large", format!("attachments exceed {COMPOSE_MAX_BYTES} bytes")));
            }
            checked.push(item);
        }
        let mut local: Vec<Attachment> = Vec::new();
        let mut inserts = Vec::with_capacity(checked.len());
        for item in checked {
            let stored = match item {
                Checked::Image { name, bytes } => ingest_image_bytes(&self.dir.join("images"), &name, &bytes)
                    .map_err(|e| refusal("ingest_failed", format!("could not attach that image: {e}")))?,
                Checked::Chip(text) => ingest_paste_text(&self.dir.join("pastes"), &text)
                    .map_err(|e| refusal("ingest_failed", format!("could not store the pasted text: {e}")))?,
                Checked::Inline(text) => {
                    inserts.push(text);
                    continue;
                }
            };
            let (attachment, marker) = stored;
            local.push(attachment);
            inserts.push(marker);
        }
        let raw = with_inserts(text, &inserts);
        let images = local
            .iter()
            .filter(|a| a.is_image())
            .map(|a| UserImage { n: a.marker_n, path: self.dir.join(&a.rel_path) })
            .collect();
        Ok((expand_paste_markers(&raw, &local, &self.dir), images, inserts))
    }
}

/// Read an image attachment, refusing a relative path, anything that is not a
/// regular file, a file over `room` bytes, and a file that is not an image.
fn check_image(path: &Path, room: usize) -> Result<Checked, Refusal> {
    use std::io::Read as _;
    if !path.is_absolute() {
        return Err(refusal("not_absolute", "the image path must be absolute"));
    }
    // The type is checked before the open, and the open does not block, so a FIFO or
    // device at the path is refused instead of stalling in open(2). The check on the
    // opened file covers a path swapped between the two.
    let not_regular = || refusal("unreadable", "could not attach that image: not a regular file");
    let meta = std::fs::metadata(path).map_err(|e| refusal("unreadable", format!("could not attach that image: {e}")))?;
    if !meta.is_file() {
        return Err(not_regular());
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK has no effect on reads of a regular file.
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let file = opts.open(path).map_err(|e| refusal("unreadable", format!("could not attach that image: {e}")))?;
    let is_file = file.metadata().map(|m| m.is_file()).unwrap_or(false);
    if !is_file {
        return Err(refusal("unreadable", "could not attach that image: not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(room as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| refusal("unreadable", format!("could not attach that image: {e}")))?;
    if bytes.len() > room {
        return Err(refusal("too_large", format!("attachments exceed {COMPOSE_MAX_BYTES} bytes")));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".to_string());
    if sniff_image_mime(Path::new(&name), &bytes).is_none() {
        return Err(refusal("not_an_image", "could not attach that image: not a recognised image"));
    }
    Ok(Checked::Image { name, bytes })
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

    // The shared path-paste helper reads a backslash as an escape, so a Windows path is not recognised.
    #[cfg(unix)]
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

    fn image(text: &str) -> SdkAttachment {
        SdkAttachment::ImageFile { path: text.into() }
    }

    fn pasted(text: &str) -> SdkAttachment {
        SdkAttachment::PastedText { text: text.into() }
    }

    #[test]
    fn a_composed_message_without_attachments_is_the_text_itself() {
        let (stage, _) = staging("compose-plain");
        assert_eq!(stage.compose("scan the login", &[]), Ok(("scan the login".into(), Vec::new(), Vec::new())));
        assert_eq!(stage.compose("  ", &[]).map_err(|(code, _)| code), Err("empty_text".to_string()));
    }

    #[test]
    fn composed_pastes_go_inline_or_become_an_expanded_chip() {
        let (stage, _) = staging("compose-paste");
        let long = "row\n".repeat(20);
        let (text, images, inserts) = stage.compose("look", &[pasted("pwd"), pasted(&long)]).expect("composed");
        assert_eq!(inserts, vec!["pwd".to_string(), "[Pasted Text #1]".to_string()]);
        assert!(images.is_empty());
        assert!(text.starts_with("look\npwd\n<<<pasted_text n=1") && text.contains(&long), "{text}");
    }

    #[test]
    fn a_composed_image_travels_with_its_marker() {
        let (stage, dir) = staging("compose-image");
        let file = dir.join("shot.png");
        std::fs::write(&file, PNG).unwrap();
        let (text, images, inserts) = stage.compose("what is this?", &[image(&file.to_string_lossy())]).expect("composed");
        assert_eq!(inserts, vec!["[Image #1]".to_string()]);
        assert_eq!(text, "what is this?\n[Image #1]");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].n, 1);
        assert!(images[0].path.starts_with(dir.join("images")) && images[0].path.is_file());
    }

    #[test]
    fn a_refused_attachment_stores_nothing_and_says_why() {
        let (stage, dir) = staging("compose-refused");
        let missing = dir.join("missing.png");
        let not_image = dir.join("notes.txt");
        std::fs::write(&not_image, b"plain words").unwrap();
        let code = |attachments: &[SdkAttachment]| stage.compose("hi", attachments).map(|_| ()).map_err(|(code, _)| code);
        assert_eq!(code(&[image("relative.png")]), Err("not_absolute".to_string()));
        assert_eq!(code(&[image(&missing.to_string_lossy())]), Err("unreadable".to_string()));
        assert_eq!(code(&[image(&dir.to_string_lossy())]), Err("unreadable".to_string()));
        assert_eq!(code(&[pasted(&"row\n".repeat(20)), image(&not_image.to_string_lossy())]), Err("not_an_image".to_string()));
        assert_eq!(code(&vec![pasted("x"); COMPOSE_MAX_ITEMS + 1]), Err("too_many_attachments".to_string()));
        assert_eq!(code(&[pasted(&"x".repeat(PASTE_SOFT_MAX_BYTES + 1))]), Err("too_large".to_string()));
        // Every check runs before the first item is stored.
        assert!(!dir.join("pastes").exists() && !dir.join("images").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_image_path_is_refused_without_blocking() {
        let (stage, dir) = staging("compose-fifo");
        let fifo = dir.join("pipe.png");
        let c_path = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `c_path` is a valid NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let path = fifo.to_string_lossy().into_owned();
        std::thread::spawn(move || {
            let result = stage.compose("hi", &[image(&path)]).map(|_| ()).map_err(|(code, _)| code);
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("compose returned");
        assert_eq!(result, Err("unreadable".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn composing_leaves_what_a_paste_staged() {
        let (stage, _) = staging("compose-shared");
        let long = "line\n".repeat(30);
        let Pasted::Insert(marker) = stage.paste_text(&long) else { panic!("refused") };
        assert_eq!(marker, "[Pasted Text #1]");
        let (text, _, inserts) = stage.compose("mine [Pasted Text #1]", &[pasted(&"other\n".repeat(5))]).expect("composed");
        assert_eq!(inserts, vec!["[Pasted Text #2]".to_string()], "the numbers do not collide");
        assert!(text.starts_with("mine [Pasted Text #1]\n<<<pasted_text n=2"), "a marker from the shared list stays as it is: {text}");
        assert!(stage.compose("bad", &[image("relative.png")]).is_err());
        let (expanded, _) = stage.take(&format!("send {marker}"));
        assert!(expanded.contains("<<<pasted_text n=1") && expanded.contains(&long), "the paste is still staged");
    }
}
