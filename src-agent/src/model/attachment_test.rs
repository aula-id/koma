#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::dto::chat::AttachmentKind;

/// Minimal-but-valid JPEG magic bytes (SOI + APP0 marker) — enough for
/// `infer` to recognise the content as `image/jpeg` regardless of the
/// file's extension.
const JPEG_MAGIC: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
];

fn tmp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "koma_attachment_test_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn sniff_image_mime_trusts_magic_bytes_over_mislabeled_extension() {
    // A file named `.png` but containing real JPEG bytes must sniff as
    // `image/jpeg` (the magic bytes), NOT `image/png` (the extension) —
    // otherwise the stored mime would contradict what upstream actually
    // receives, producing a 400 "Multimodal data is corrupted" error.
    let path = Path::new("photo.png");
    let mime = sniff_image_mime(path, JPEG_MAGIC);
    assert_eq!(mime.as_deref(), Some("image/jpeg"));
}

#[test]
fn sniff_image_mime_falls_back_to_extension_when_infer_has_no_opinion() {
    // Bytes `infer` can't identify (too short / no known magic) but which
    // still carry a recognised image extension keep using the extension
    // fallback, unchanged from prior behavior.
    let path = Path::new("photo.png");
    let mime = sniff_image_mime(path, b"not a real image body");
    assert_eq!(mime.as_deref(), Some("image/png"));
}

#[test]
fn ingest_image_from_raw_bytes_trusts_magic_bytes_over_wrong_host_mime() {
    // The host/clipboard tool claims `image/png` via `--type`, but the
    // actual bytes are JPEG — the stored attachment mime must reflect the
    // real bytes, not the (wrong) host-supplied type.
    let dir = tmp_dir("raw_mime");
    let (att, _marker) =
        ingest_image_from_raw_bytes(&dir, JPEG_MAGIC, "image/png", "pasted.png").unwrap();
    assert_eq!(att.mime, "image/jpeg");
    assert_eq!(att.kind, AttachmentKind::Image);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn should_collapse_paste_char_threshold() {
    let under: String = "a".repeat(149);
    let at: String = "a".repeat(150);
    assert!(!should_collapse_paste(&under));
    assert!(should_collapse_paste(&at));
}

#[test]
fn should_collapse_paste_line_threshold() {
    assert!(!should_collapse_paste("single line"));
    assert!(should_collapse_paste("line one\nline two"));
    // A trailing newline still counts as multi-line paste (≥2 lines).
    assert!(should_collapse_paste("one line with eol\n"));
}

#[test]
fn ingest_paste_text_writes_file_and_marker() {
    let dir = tmp_dir("paste");
    let body = "hello paste\nsecond line";
    let (att, marker) = ingest_paste_text(&dir, body).unwrap();
    assert_eq!(att.kind, AttachmentKind::PastedText);
    assert_eq!(att.marker_n, 1);
    assert_eq!(att.rel_path, "pastes/01-paste.txt");
    assert_eq!(att.mime, "text/plain");
    assert_eq!(marker, "[Pasted Text #1]");
    let on_disk = std::fs::read_to_string(dir.join("01-paste.txt")).unwrap();
    assert_eq!(on_disk, body);
    // Second paste bumps seq independently of images.
    let (att2, marker2) = ingest_paste_text(&dir, "again").unwrap();
    assert_eq!(att2.marker_n, 2);
    assert_eq!(marker2, "[Pasted Text #2]");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ingest_paste_text_rejects_over_soft_cap() {
    let dir = tmp_dir("paste_cap");
    let huge = "x".repeat(PASTE_SOFT_MAX_BYTES + 1);
    let err = ingest_paste_text(&dir, &huge).unwrap_err();
    assert!(err.to_string().contains("soft cap"));
    std::fs::remove_dir_all(&dir).ok();
}
