#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// Minimal-but-valid JPEG magic bytes (SOI + APP0 marker) — enough for
/// `infer` to recognise the content as `image/jpeg` regardless of the
/// file's extension.
const JPEG_MAGIC: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
];

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
    let dir = std::env::temp_dir().join(format!(
        "koma_attachment_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (att, _marker) =
        ingest_image_from_raw_bytes(&dir, JPEG_MAGIC, "image/png", "pasted.png").unwrap();
    assert_eq!(att.mime, "image/jpeg");
    std::fs::remove_dir_all(&dir).ok();
}
