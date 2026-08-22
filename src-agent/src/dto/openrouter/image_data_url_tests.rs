#![allow(clippy::unwrap_used, clippy::expect_used)]
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
