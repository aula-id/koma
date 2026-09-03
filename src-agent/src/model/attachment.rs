//! Attachment ingest core: copy a file into the session's `images/` or
//! `pastes/` dir and return the [`Attachment`] record + composer marker token.
//!
//! ONE ingest core, MANY callers. Every path that wants to attach an image or
//! collapsed paste routes through here so the on-disk layout, the monotonic
//! marker numbering, and the mime sniff stay identical regardless of entry
//! point:
//! - path-paste (the user pastes a text path to an image file),
//! - the `@`-picker image branch,
//! - the send-time `@`-scan backstop,
//! - clipboard bitmaps via raw bytes,
//! - large/multi-line text pastes → `pastes/NN-paste.txt`.
//!
//! Image layout: `<images_dir>/NN-basename.ext` (marker `[Image #N]`).
//! Paste layout: `<pastes_dir>/NN-paste.txt` (marker `[Pasted Text #N]`).
//! Each dir has its own `.seq` counter so N spaces are independent.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::dto::chat::{Attachment, AttachmentKind};

/// The image extensions koma recognises for attachment (lowercased, no dot).
/// Used by the extension-first mime sniff AND by the paste/`@` callers to decide
/// whether a path is an image before routing it through ingest.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Whether `path`'s extension marks it as one of the [`IMAGE_EXTS`] koma ingests.
/// Pure string check — does NOT touch the filesystem (no existence test).
pub fn has_image_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|e| IMAGE_EXTS.contains(&e.as_str()))
        .unwrap_or(false)
}

/// Soft upper bound for a single collapsed paste body (2 MiB). Larger pastes
/// are rejected at ingest so a runaway clipboard dump cannot fill the session
/// dir or blow the model context in one shot.
pub const PASTE_SOFT_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Collapse threshold: pasted text becomes a `[Pasted Text #N]` chip when it
/// has **≥150 characters** OR **≥2 lines** (a trailing newline alone does not
/// count as a second line — `"a\n"` is still one line of content).
pub const PASTE_COLLAPSE_MIN_CHARS: usize = 150;

/// Whether `s` should collapse to a paste chip instead of staying inline in
/// the composer. Pure predicate — does not touch disk.
///
/// Collapse when the paste is **≥150 characters** OR contains at least one
/// newline (product lock: ≥2 lines, including a single line with trailing EOL).
pub fn should_collapse_paste(s: &str) -> bool {
    s.chars().count() >= PASTE_COLLAPSE_MIN_CHARS || s.contains('\n')
}

/// Atomically increment and return the next sequence number for `dir`.
///
/// The counter is persisted in `dir/.seq` (a plain text file holding the
/// last-used integer). On each call: read the current value (0 if absent), add 1,
/// write back, and return the new value. This is single-writer (the TUI event loop
/// is single-threaded), so a simple read-modify-write on the `.seq` file is safe
/// and collision-free even when several attachments are ingested in a single submit.
///
/// The `.seq` file lives inside the attachment dir so it is cleaned up
/// automatically when the session directory is removed — no separate teardown needed.
fn next_attach_seq(dir: &Path) -> usize {
    let seq_path = dir.join(".seq");
    let current: usize = std::fs::read_to_string(&seq_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let next = current + 1;
    // Best-effort write; a failure (e.g. read-only fs) just means the counter
    // won't persist across this call, producing a duplicate NN — the same risk
    // as the old read_dir approach, and equally rare.
    let _ = std::fs::write(&seq_path, next.to_string());
    next
}

/// Image-dir sequence helper (kept as a thin alias for call-site clarity).
fn next_image_seq(images_dir: &Path) -> usize {
    next_attach_seq(images_dir)
}

/// Pastes-dir sequence helper.
fn next_paste_seq(pastes_dir: &Path) -> usize {
    next_attach_seq(pastes_dir)
}

/// Sniff a mime type from `bytes`' magic numbers first (via the `infer` crate),
/// falling back to the path's extension only when `infer` has no conclusive
/// opinion. Returns `None` when the file is not a recognised image — this
/// rejection behavior is unchanged from the extension-first version.
///
/// The magic-byte type is AUTHORITATIVE: a `.png`-named file containing JPEG
/// bytes comes back as `image/jpeg`, so the mime we store always matches what
/// we actually send upstream (fixes 400s from mislabelled extensions). The
/// extension is only consulted (a) to gate that this path is even a candidate
/// image at all, and (b) as the mime fallback when `infer` can't identify the
/// bytes at all (returns `None`) — some small/edge images aren't covered by
/// `infer`, so we trust the extension exactly as before in that case.
fn sniff_image_mime(path: &Path, bytes: &[u8]) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    // Magic-byte check: if `infer` recognises the bytes, it wins outright —
    // both for the reject gate (must be an image/*) and for the mime string
    // itself (so a mislabelled extension doesn't lie about the real bytes).
    if let Some(kind) = infer::get(bytes) {
        if !kind.mime_type().starts_with("image/") {
            return None;
        }
        return Some(kind.mime_type().to_string());
    }
    // `infer` had no opinion (some small/edge images aren't covered) — trust
    // the extension, same as before.
    let subtype = if ext == "jpg" { "jpeg" } else { ext.as_str() };
    Some(format!("image/{subtype}"))
}

/// Build the destination filename `NN-basename.ext` for the `nn`-th attachment,
/// preserving `src`'s original basename + extension. A source with no usable
/// file name falls back to `image` (keeping any extension).
fn dest_name(nn: usize, src: &Path) -> String {
    let base = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "image".to_string());
    format!("{nn:02}-{base}")
}

/// Ingest raw image `bytes` (already in memory) into `images_dir` under
/// `basename`, returning the [`Attachment`] + its `[Image #N]` marker token.
///
/// This is the BYTES form of the ingest core (used by the clipboard-bitmap path
/// in a later slice); the path form ([`ingest_image_from_path`]) reads the file
/// then delegates here. Steps:
/// 1. lazily create `images_dir`,
/// 2. compute the next monotonic `NN` via [`next_image_seq`] (`.seq` counter file),
/// 3. sniff the mime (extension + magic bytes); reject non-images,
/// 4. write `NN-basename.ext`,
/// 5. return `(Attachment { marker_n, rel_path: "images/NN-…", mime }, "[Image #N]")`.
pub fn ingest_image_bytes(
    images_dir: &Path,
    basename: &str,
    bytes: &[u8],
) -> Result<(Attachment, String)> {
    std::fs::create_dir_all(images_dir)?;
    let nn = next_image_seq(images_dir);
    let name_path = Path::new(basename);
    let mime = sniff_image_mime(name_path, bytes)
        .ok_or_else(|| anyhow!("not a recognised image: {basename}"))?;
    let dest = dest_name(nn, name_path);
    let dest_path = images_dir.join(&dest);
    std::fs::write(&dest_path, bytes)?;
    let rel_path = format!("images/{dest}");
    let marker = format!("[Image #{nn}]");
    Ok((
        Attachment {
            kind: AttachmentKind::Image,
            marker_n: nn,
            rel_path,
            mime,
        },
        marker,
    ))
}

/// Ingest raw image `bytes` with an EXPLICIT `mime` string (e.g. `"image/png"`)
/// and `basename` (e.g. `"pasted.png"`) into `images_dir`.
///
/// This is the clipboard-bitmap entry point: the caller supplies a `--type` mime
/// from the clipboard tool, but the magic bytes are AUTHORITATIVE — when `infer`
/// identifies a concrete image type, that wins over the host-supplied `mime`
/// (fixes a mislabelled `--type` producing a mime that contradicts the actual
/// bytes upstream). The host-supplied `mime` (falling back to the basename
/// extension) is used only when `infer` can't identify the bytes at all. The
/// magic-byte check also still gates rejection of non-image clipboard data.
/// On success returns `(Attachment, "[Image #N]")`.
pub fn ingest_image_from_raw_bytes(
    images_dir: &Path,
    bytes: &[u8],
    mime: &str,
    basename: &str,
) -> Result<(Attachment, String)> {
    let inferred = infer::get(bytes);
    if let Some(kind) = &inferred {
        if !kind.mime_type().starts_with("image/") {
            return Err(anyhow!("clipboard data does not appear to be an image"));
        }
    }
    let effective_mime =
        if let Some(kind) = inferred.filter(|k| k.mime_type().starts_with("image/")) {
            // Magic bytes identified a concrete image type — trust that over the
            // host-supplied `--type`, which may be stale or simply wrong.
            kind.mime_type().to_string()
        } else if mime.starts_with("image/") {
            mime.to_string()
        } else {
            // Try to derive from basename extension.
            let ext = Path::new(basename)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_else(|| "png".to_string());
            let sub = if ext == "jpg" {
                "jpeg".to_string()
            } else {
                ext
            };
            format!("image/{sub}")
        };
    std::fs::create_dir_all(images_dir)?;
    let nn = next_image_seq(images_dir);
    let name_path = Path::new(basename);
    let dest = dest_name(nn, name_path);
    let dest_path = images_dir.join(&dest);
    std::fs::write(&dest_path, bytes)?;
    let rel_path = format!("images/{dest}");
    let marker = format!("[Image #{nn}]");
    Ok((
        Attachment {
            kind: AttachmentKind::Image,
            marker_n: nn,
            rel_path,
            mime: effective_mime,
        },
        marker,
    ))
}

/// Ingest pasted `text` into `pastes_dir` as `NN-paste.txt`, returning the
/// [`Attachment`] + its `[Pasted Text #N]` marker token.
///
/// Disk is the source of truth for the body (same contract as images). The
/// caller is expected to have already decided collapse via
/// [`should_collapse_paste`]. Rejects bodies larger than
/// [`PASTE_SOFT_MAX_BYTES`].
pub fn ingest_paste_text(pastes_dir: &Path, text: &str) -> Result<(Attachment, String)> {
    if text.len() > PASTE_SOFT_MAX_BYTES {
        return Err(anyhow!(
            "pasted text exceeds {} byte soft cap",
            PASTE_SOFT_MAX_BYTES
        ));
    }
    std::fs::create_dir_all(pastes_dir)?;
    let nn = next_paste_seq(pastes_dir);
    let dest = format!("{nn:02}-paste.txt");
    let dest_path = pastes_dir.join(&dest);
    std::fs::write(&dest_path, text)?;
    let rel_path = format!("pastes/{dest}");
    let marker = format!("[Pasted Text #{nn}]");
    Ok((
        Attachment {
            kind: AttachmentKind::PastedText,
            marker_n: nn,
            rel_path,
            mime: "text/plain".to_string(),
        },
        marker,
    ))
}

/// Ingest the image file at `src_path` into `images_dir`, returning the
/// [`Attachment`] + its `[Image #N]` marker token.
///
/// The PATH entry point of the ingest core (path-paste, `@`-picker, send-time
/// `@`-scan). Reads the file off disk, then delegates to [`ingest_image_bytes`]
/// for the copy + sniff + numbering. Errors (missing file, non-image, write
/// failure) propagate so the caller can toast and leave the composer untouched.
pub fn ingest_image_from_path(images_dir: &Path, src_path: &Path) -> Result<(Attachment, String)> {
    let bytes = std::fs::read(src_path)?;
    let basename = src_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    ingest_image_bytes(images_dir, &basename, &bytes)
}

/// Scan `text` for `@<path>` tokens that resolve to existing image files on
/// disk, ingest each one into `images_dir`, rewrite the `@path` token to its
/// `[Image #N]` marker in the returned text, and collect the produced
/// [`Attachment`] records.
///
/// This is the SEND-TIME `@`-scan backstop (Slice 3). It fires on every submit
/// and catches hand-typed `@path/to/image.png` tokens that bypassed the
/// interactive picker. Dedup is automatic — the interactive picker already
/// rewrote its `@path` to `[Image #N]`, so no `@path` for those remains.
///
/// Only `@`-prefixed tokens are considered (NEVER bare filenames in prose).
/// A token is a run of non-whitespace characters. Tokens that do not have
/// an image extension, or whose resolved path does not exist, are left
/// unchanged (silently skipped — not an error).
///
/// `workdir` is the session's working directory; relative paths in `@tokens`
/// are resolved against it.
pub fn scan_at_image_tokens(
    text: &str,
    images_dir: &Path,
    workdir: &Path,
) -> (String, Vec<Attachment>) {
    // Collect (start_byte, end_byte) for each non-whitespace token.
    let mut tokens: Vec<(usize, usize)> = Vec::new();
    let mut tok_start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = tok_start.take() {
                tokens.push((s, i));
            }
        } else if tok_start.is_none() {
            tok_start = Some(i);
        }
    }
    if let Some(s) = tok_start {
        tokens.push((s, text.len()));
    }

    let mut result = String::with_capacity(text.len());
    let mut attachments: Vec<Attachment> = Vec::new();
    let mut cursor = 0usize; // byte position we've flushed up to

    for (start, end) in tokens {
        let token = &text[start..end];
        if let Some(path_str) = token.strip_prefix('@') {
            if has_image_extension(path_str) {
                let src = if Path::new(path_str).is_absolute() {
                    std::path::PathBuf::from(path_str)
                } else {
                    workdir.join(path_str)
                };
                if src.exists() {
                    match ingest_image_from_path(images_dir, &src) {
                        Ok((att, marker)) => {
                            // Copy text before this token, then the replacement marker.
                            result.push_str(&text[cursor..start]);
                            result.push_str(&marker);
                            cursor = end;
                            attachments.push(att);
                            continue;
                        }
                        Err(_) => {
                            // Ingest failed: leave the @token verbatim.
                        }
                    }
                }
            }
        }
        // Default: copy everything up to and including this token.
        result.push_str(&text[cursor..end]);
        cursor = end;
    }
    // Flush any trailing whitespace after the last token.
    if cursor < text.len() {
        result.push_str(&text[cursor..]);
    }

    (result, attachments)
}

/// List all `.png` files in a project's `.screenshoot/` directory, returning
/// their basenames sorted alphabetically. Returns an empty vec when the
/// directory doesn't exist or contains no PNGs.
pub fn list_screenshoot_pngs(workspace: &Path) -> Vec<String> {
    let dir = workspace.join(".screenshoot");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "png")
                .unwrap_or(false)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Resolve a `.screenshoot/` filename to an absolute path inside the project's
/// `.screenshoot/` directory. Returns `None` if the resolved path doesn't
/// exist or isn't a file.
pub fn resolve_screenshoot_path(workspace: &Path, name: &str) -> Option<PathBuf> {
    let p = workspace.join(".screenshoot").join(name);
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "attachment_test.rs"]
mod tests;
