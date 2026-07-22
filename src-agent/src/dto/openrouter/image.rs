//! Attachment data-URL derivation + oversized-image downscaling for the
//! OpenRouter wire builder. Split out of [`super::request`] for file size; no
//! behaviour change — [`data_url_for`] is re-exported from `request` so its
//! existing `crate::dto::openrouter::request::data_url_for` call sites
//! (the Codex Responses transport) keep resolving unchanged.

use std::path::{Path, PathBuf};

/// Read an attachment's on-disk bytes and build its `data:<mime>;base64,<…>` URL,
/// or `None` when the file can't be read. `rel_path` (`images/NN-name.ext`) is
/// resolved against the session dir — the bytes are NEVER taken from the message.
///
/// This heals attachments that were ingested with a wrong stored `mime` (e.g.
/// from an older koma build, or any path that didn't run the magic-byte sniff):
/// the bytes are re-sniffed here at send time via `infer::get`, and its verdict
/// wins over `att.mime` whenever it identifies a concrete image type. `att.mime`
/// is only used when `infer` has no opinion on the bytes.
///
/// Oversized images are downscaled before they're base64-encoded — see
/// [`maybe_downscale`]. Both the chat-completions parts path (this function)
/// and the Codex Responses `input_image` path go through here, so the mime in
/// the emitted data-URL always matches the bytes actually sent: the
/// infer-derived mime for a passthrough, or `image/jpeg`/`image/png` for a
/// downscaled rendition.
///
/// `pub(crate)` so the Codex Responses transport (`service::openrouter::codex`)
/// reuses the exact same data-URL derivation for its `input_image` parts.
pub(crate) fn data_url_for(
    session_dir: &Path,
    att: &crate::dto::chat::Attachment,
) -> Option<String> {
    use base64::Engine;
    let path = session_dir.join(&att.rel_path);
    let bytes = std::fs::read(&path).ok()?;
    let mime = infer::get(&bytes)
        .filter(|kind| kind.mime_type().starts_with("image/"))
        .map(|kind| kind.mime_type().to_string())
        .unwrap_or_else(|| att.mime.clone());
    let (bytes, mime) = maybe_downscale(&path, bytes, &mime);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}

/// Longest edge (px) an image may have before [`maybe_downscale`] resizes it.
const MAX_IMAGE_EDGE: u32 = 2048;
/// Raw byte size an image may have before [`maybe_downscale`] resizes it (4 MiB).
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Downscale an oversized image for the wire, or pass it through untouched.
///
/// The envelope: an image whose raw byte length is `<= MAX_IMAGE_BYTES` AND
/// whose decoded longest edge is `<= MAX_IMAGE_EDGE` sends exactly as today —
/// this is the common case, and it's checked cheaply (only the header is read,
/// via [`image::ImageReader::into_dimensions`], never a full pixel decode) so
/// small images never touch the resize/encode path at all.
///
/// An image outside the envelope is: decoded, resized so the longest edge is
/// `MAX_IMAGE_EDGE` (aspect-ratio preserved, `CatmullRom` filter), then
/// re-encoded — JPEG quality 85 when the source has no alpha channel, PNG when
/// it does (an animated GIF source decodes to its first frame only, the
/// `image` crate's default for the non-animation decode path, and re-encodes
/// like any other still image). The rendition is cached at
/// `<images_dir>/.scaled/<stem>.<jpg|png>` (see [`read_cached_rendition`] /
/// [`write_cached_rendition`]) so repeat sends of the same attachment skip the
/// decode/resize/encode work entirely.
///
/// Graceful passthrough: ANY failure along this path — header unreadable,
/// decode failure (e.g. a format outside the compiled-in codec set), encode
/// failure, or a "downscaled" result that's somehow larger than the original —
/// falls back to the original bytes/mime, unchanged. An oversized original
/// that still 400s upstream is no worse than the pre-existing behaviour; this
/// function never makes sending an image less likely to succeed than before.
fn maybe_downscale(path: &Path, bytes: Vec<u8>, mime: &str) -> (Vec<u8>, String) {
    let within_bytes = bytes.len() <= MAX_IMAGE_BYTES;
    let within_edge = if within_bytes {
        image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .map(|(w, h)| w.max(h) <= MAX_IMAGE_EDGE)
            // Header unreadable (unsupported/odd format, or not really an
            // image at all) — treat as "within envelope" so we pass it
            // through untouched: the resize path can't handle it either, and
            // this keeps behaviour identical to before this feature existed.
            .unwrap_or(true)
    } else {
        false
    };
    if within_bytes && within_edge {
        return (bytes, mime.to_string());
    }

    // Oversized: a fresh cached rendition (newer than the source file) skips
    // the decode/resize/encode work entirely.
    if let Some(cached) = read_cached_rendition(path) {
        return cached;
    }

    match downscale_and_encode(&bytes) {
        Some((scaled, scaled_mime)) if scaled.len() < bytes.len() => {
            write_cached_rendition(path, &scaled, scaled_mime);
            (scaled, scaled_mime.to_string())
        }
        // Decode/encode failed, or the "downscaled" rendition is somehow
        // larger than the original (rare) — passthrough.
        _ => (bytes, mime.to_string()),
    }
}

/// Decode `bytes`, resize so the longest edge is [`MAX_IMAGE_EDGE`], and
/// re-encode: JPEG (quality 85) when the source has no alpha channel, PNG when
/// it does. `None` on any decode/resize/encode failure.
fn downscale_and_encode(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    use image::imageops::FilterType;
    use image::ImageEncoder;

    let img = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let has_alpha = img.color().has_alpha();
    // `resize` fits the image within an `edge x edge` bounding box, preserving
    // aspect ratio — exactly what we want for "longest edge <= MAX_IMAGE_EDGE".
    let resized = img.resize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE, FilterType::CatmullRom);

    let mut out = Vec::new();
    if has_alpha {
        let rgba = resized.to_rgba8();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
        Some((out, "image/png"))
    } else {
        let rgb = resized.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .ok()?;
        Some((out, "image/jpeg"))
    }
}

/// The cache directory for downscaled renditions, sibling to the source file
/// itself: `<images_dir>/.scaled/` (the source lives at `<images_dir>/NN-name.ext`,
/// so `path.parent()` IS the images dir).
fn scaled_cache_dir(path: &Path) -> Option<PathBuf> {
    Some(path.parent()?.join(".scaled"))
}

/// Look up a cached rendition for `path` (the on-disk source image), returning
/// its bytes + mime only when a cache file exists AND its mtime is `>=` the
/// source's mtime. Any I/O or mtime error is treated as a cache miss — per the
/// graceful-passthrough contract, a stale/unreadable cache is never fatal, it
/// just means [`maybe_downscale`] regenerates the rendition instead.
fn read_cached_rendition(path: &Path) -> Option<(Vec<u8>, String)> {
    let cache_dir = scaled_cache_dir(path)?;
    let stem = path.file_stem()?.to_str()?;
    for (ext, mime) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        let candidate = cache_dir.join(format!("{stem}.{ext}"));
        let Ok(cached_meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        let is_fresh = (|| -> Option<bool> {
            let cached_mtime = cached_meta.modified().ok()?;
            let source_mtime = std::fs::metadata(path).ok()?.modified().ok()?;
            Some(cached_mtime >= source_mtime)
        })()
        .unwrap_or(false); // any mtime error => stale, regenerate
        if is_fresh {
            if let Ok(bytes) = std::fs::read(&candidate) {
                return Some((bytes, mime.to_string()));
            }
        }
    }
    None
}

/// Write a downscaled rendition to `<images_dir>/.scaled/<stem>.<jpg|png>`,
/// creating the `.scaled` dir if needed. Best-effort: any I/O failure is
/// silently ignored — the caller already has the bytes in hand to send, so a
/// failed cache write only costs a re-encode on the next send, never the send
/// itself. The source file at `path` is never modified.
fn write_cached_rendition(path: &Path, bytes: &[u8], mime: &str) {
    let Some(cache_dir) = scaled_cache_dir(path) else {
        return;
    };
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    if std::fs::create_dir_all(&cache_dir).is_err() {
        return;
    }
    let ext = if mime == "image/png" { "png" } else { "jpg" };
    let _ = std::fs::write(cache_dir.join(format!("{stem}.{ext}")), bytes);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod data_url_tests {
    use super::*;

    /// Unique-per-test scratch dir under the OS temp dir, mirroring the
    /// manual pid+nanos temp-dir convention used elsewhere in this crate's
    /// tests (no `tempfile` dev-dependency exists). Caller is responsible for
    /// `std::fs::remove_dir_all` cleanup.
    fn test_tmp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "koma_data_url_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Encode a synthetic `w x h` RGB (no alpha) PNG in memory — used as both
    /// the "small, passthrough" and "oversized, downscale" test fixture.
    ///
    /// Pixels are deterministic pseudo-random noise (a small LCG), not a
    /// smooth gradient: a gradient PNG-compresses so well that a re-encoded
    /// JPEG of the same pixels comes back LARGER than the tiny original —
    /// which correctly triggers `maybe_downscale`'s "keep the original"
    /// safety net (item 2 in the feature spec) and would make the oversized
    /// test fixture assert against the wrong rendition. Noise behaves like a
    /// real photo for compression purposes, so the JPEG rendition is reliably
    /// smaller than the PNG original once oversized.
    fn encode_test_png(w: u32, h: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let mut seed: u32 = 12345;
        let mut next_byte = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) as u8
        };
        let mut raw = vec![0u8; (w * h * 3) as usize];
        for b in raw.iter_mut() {
            *b = next_byte();
        }
        let img = image::RgbImage::from_raw(w, h, raw).expect("raw buf sized for w*h*3");
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .expect("encode test png");
        buf
    }

    /// (a) An image within the envelope (small dimensions, small bytes) must
    /// send byte-identical to a manual data-URL encode — zero behaviour change,
    /// no re-encode.
    #[test]
    fn small_image_passes_through_byte_identical() {
        let dir = test_tmp_dir("small");
        let images_dir = dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let bytes = encode_test_png(4, 4);
        std::fs::write(images_dir.join("01-small.png"), &bytes).unwrap();
        let att = crate::dto::chat::Attachment {
            marker_n: 1,
            rel_path: "images/01-small.png".to_string(),
            mime: "image/png".to_string(),
        };

        let got = data_url_for(&dir, &att).expect("data url");

        use base64::Engine;
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        );
        assert_eq!(got, expected);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (b) An image exceeding the longest-edge bound must come back downscaled
    /// to `<= MAX_IMAGE_EDGE` on its longest edge, re-encoded (no alpha source
    /// -> JPEG) with the mime in the data-URL matching the rendition actually
    /// sent.
    #[test]
    fn oversized_image_is_downscaled() {
        let dir = test_tmp_dir("oversized");
        let images_dir = dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let bytes = encode_test_png(3000, 100);
        std::fs::write(images_dir.join("01-big.png"), &bytes).unwrap();
        let att = crate::dto::chat::Attachment {
            marker_n: 1,
            rel_path: "images/01-big.png".to_string(),
            mime: "image/png".to_string(),
        };

        let got = data_url_for(&dir, &att).expect("data url");

        assert!(
            got.starts_with("data:image/jpeg;base64,"),
            "expected a JPEG rendition (no-alpha source), got prefix of: {}",
            &got[..got.len().min(40)]
        );
        let b64 = got.strip_prefix("data:image/jpeg;base64,").unwrap();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&decoded))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert!(
            w.max(h) <= MAX_IMAGE_EDGE,
            "longest edge {} exceeds cap {}",
            w.max(h),
            MAX_IMAGE_EDGE
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (c) A second send of the same oversized attachment reuses the cached
    /// rendition written by the first send (rather than silently failing or
    /// drifting) — same data-URL both times, and the cache file exists on disk
    /// at the documented path.
    #[test]
    fn cache_file_created_and_reused() {
        let dir = test_tmp_dir("cache");
        let images_dir = dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let bytes = encode_test_png(3000, 100);
        std::fs::write(images_dir.join("01-cache.png"), &bytes).unwrap();
        let att = crate::dto::chat::Attachment {
            marker_n: 1,
            rel_path: "images/01-cache.png".to_string(),
            mime: "image/png".to_string(),
        };

        let first = data_url_for(&dir, &att).expect("first call");

        let cache_path = images_dir.join(".scaled").join("01-cache.jpg");
        assert!(
            cache_path.exists(),
            "expected cached rendition at {:?}",
            cache_path
        );

        let second = data_url_for(&dir, &att).expect("second call");
        assert_eq!(
            first, second,
            "cached rendition must reproduce the same data URL"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
