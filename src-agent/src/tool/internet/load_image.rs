//! `load_image` tool — securely load an existing workspace, session-scratch,
//! or this session's `images/` attachment file.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::ToolCtx;

/// Bound model-facing image loads before allocating/reading the whole file.
pub(crate) const MAX_LOAD_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Load an image from an explicitly permitted local path.
pub struct LoadImage;

impl super::Tool for LoadImage {
    fn name(&self) -> &'static str {
        "load_image"
    }

    fn description(&self) -> &'static str {
        "Load an existing image file from a configured workspace, this session's \
         exact scratch directory, or this session's images/ attachment directory \
         into the next model message for visual inspection. Use after message_find \
         or compact when a past [Image #N] is no longer in live context."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Image path. Relative paths resolve in configured workspaces; `images/NN-…` resolves against this session directory; absolute paths must be inside a workspace, this session's scratch directory, or this session's images/ directory."
                },
                "image_n": {
                    "type": "integer",
                    "description": "Optional marker number N from [Image #N]. Resolves to the matching file under this session's images/ (NN-*). Prefer when message_find or a compact inventory listed the marker without a full path."
                }
            },
            "additionalProperties": false
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let (path, _) = read_validated_image_from_args(ctx, args)?;
        Ok(format!("load_image: validated {}", path.display()))
    }
}

/// Path string from args (non-empty when present).
#[allow(dead_code)] // retained for callers/tests that only need path extraction
pub(crate) fn image_arg(args: &Value) -> Result<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required non-empty string argument 'path'"))
}

fn image_n_arg(args: &Value) -> Option<usize> {
    args.get("image_n").and_then(|v| {
        v.as_u64()
            .map(|n| n as usize)
            .or_else(|| v.as_i64().filter(|&n| n > 0).map(|n| n as usize))
    })
}

/// Resolve path or `image_n`, then validate + read. Used by the tool and the
/// stream interceptor.
pub(crate) fn read_validated_image_from_args(
    ctx: &ToolCtx,
    args: &Value,
) -> Result<(PathBuf, Vec<u8>)> {
    let requested = match (image_n_arg(args), args.get("path").and_then(Value::as_str)) {
        (Some(n), _) => {
            let path = resolve_session_image_n(ctx, n)?;
            return read_validated_image(ctx, &path.to_string_lossy());
        }
        (None, Some(p)) if !p.is_empty() => p,
        _ => bail!("provide non-empty 'path' and/or positive 'image_n'"),
    };
    read_validated_image(ctx, requested)
}

/// Resolve, contain, size-check, and read once. Canonicalizing the existing file
/// closes ordinary traversal and symlink escapes. A narrow filesystem TOCTOU remains
/// between canonicalization and open (portable Rust cannot bind an opened handle to
/// its canonical path); after open, metadata and the single bounded read use that
/// same handle, and ingest consumes those validated bytes without reopening source.
pub(crate) fn read_validated_image(ctx: &ToolCtx, requested: &str) -> Result<(PathBuf, Vec<u8>)> {
    let candidate = resolve_candidate(ctx, requested)?;
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("image does not exist: {}", candidate.display()))?;

    if !allowed_canonical_path(ctx, &canonical) {
        bail!(
            "image path is outside configured workspaces, this session's scratch directory, and this session's images/ directory"
        );
    }

    let file = std::fs::File::open(&canonical)
        .with_context(|| format!("failed to open image: {}", canonical.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("image source is not a regular file");
    }
    if metadata.len() > MAX_LOAD_IMAGE_BYTES {
        bail!(
            "image is too large ({} bytes; maximum is {} bytes)",
            metadata.len(),
            MAX_LOAD_IMAGE_BYTES
        );
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_LOAD_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LOAD_IMAGE_BYTES {
        bail!(
            "image grew beyond the {} byte maximum while reading",
            MAX_LOAD_IMAGE_BYTES
        );
    }
    let basename = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !crate::model::attachment::has_image_extension(basename) {
        bail!("not a recognised image: {}", canonical.display());
    }
    match infer::get(&bytes) {
        Some(kind) if kind.mime_type().starts_with("image/") => {}
        _ => bail!("file contents are not a recognised image"),
    }
    Ok((canonical, bytes))
}

fn resolve_candidate(ctx: &ToolCtx, requested: &str) -> Result<PathBuf> {
    let path = Path::new(requested);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    // Session-relative attachment paths: `images/NN-name.ext` (and bare `NN-…`
    // under images when the model drops the prefix).
    if let Some(session) = ctx.session_dir.as_ref() {
        let norm = requested.replace('\\', "/");
        if norm.starts_with("images/") || norm == "images" {
            return Ok(session.join(requested));
        }
        // Bare filename that looks like an attachment: `03-foo.png`
        if looks_like_session_image_basename(&norm) {
            return Ok(session.join("images").join(requested));
        }
    }

    let (idx, bare) = crate::tool::parse_ws_prefix(requested);
    let roots = image_workspace_roots(ctx);
    let root = roots.get(idx).ok_or_else(|| {
        anyhow::anyhow!(
            "workspace index [{idx}] out of range (have {})",
            roots.len()
        )
    })?;
    Ok(root.join(bare))
}

fn looks_like_session_image_basename(name: &str) -> bool {
    if name.contains('/') {
        return false;
    }
    let bytes = name.as_bytes();
    // NN-… with at least one digit pair and a dash
    if bytes.len() < 4 {
        return false;
    }
    bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'-'
        && crate::model::attachment::has_image_extension(name)
}

/// Resolve `[Image #N]` → absolute path under this session's `images/`.
fn resolve_session_image_n(ctx: &ToolCtx, n: usize) -> Result<PathBuf> {
    let session = ctx
        .session_dir
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no active session; cannot resolve image_n"))?;
    resolve_image_marker_in_session(session, n).ok_or_else(|| {
        anyhow::anyhow!("no session image found for [Image #{n}] under images/")
    })
}

/// Public helper: resolve marker N under a session directory (messages.json then glob).
pub(crate) fn resolve_image_marker_in_session(session_dir: &Path, n: usize) -> Option<PathBuf> {
    if n == 0 {
        return None;
    }
    // Prefer messages.json attachment records (authoritative rel_path).
    if let Some(path) = resolve_marker_from_messages_json(session_dir, n) {
        if path.is_file() {
            return Some(path);
        }
    }
    // Fallback: glob images/{NN}-* (zero-padded to two digits, matching ingest).
    resolve_marker_from_images_dir(&session_dir.join("images"), n)
}

fn resolve_marker_from_messages_json(session_dir: &Path, n: usize) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(session_dir.join("messages.json")).ok()?;
    let msgs: Vec<crate::dto::chat::ChatMessage> = serde_json::from_str(&raw).ok()?;
    for msg in msgs {
        for att in msg.attachments {
            if att.marker_n == n {
                let p = session_dir.join(&att.rel_path);
                return Some(p);
            }
        }
    }
    None
}

fn resolve_marker_from_images_dir(images_dir: &Path, n: usize) -> Option<PathBuf> {
    let prefix = format!("{n:02}-");
    let rd = std::fs::read_dir(images_dir).ok()?;
    let mut matches: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&prefix)
                            && crate::model::attachment::has_image_extension(name)
                    })
        })
        .collect();
    matches.sort();
    matches.into_iter().next()
}

fn image_workspace_roots(ctx: &ToolCtx) -> Vec<&PathBuf> {
    ctx.workspaces.iter().collect()
}

fn session_images_canonical(ctx: &ToolCtx) -> Option<PathBuf> {
    let session = ctx.session_dir.as_ref()?;
    // None when images/ is missing — no allowed file can live there yet.
    session.join("images").canonicalize().ok()
}

fn allowed_canonical_path(ctx: &ToolCtx, canonical: &Path) -> bool {
    image_workspace_roots(ctx).into_iter().any(|root| {
        root.canonicalize()
            .map(|root| canonical.starts_with(&root))
            .unwrap_or(false)
    }) || ctx.scratch_dir.as_ref().is_some_and(|scratch| {
        scratch
            .canonicalize()
            .map(|scratch| canonical.starts_with(&scratch))
            .unwrap_or(false)
    }) || session_images_canonical(ctx)
        .is_some_and(|images| canonical.starts_with(&images))
}

/// Build a compact inventory of session images still on disk (for compact/plan seed).
/// Returns `None` when the dir is missing or empty of image files.
pub fn format_session_images_inventory(images_dir: &Path) -> Option<String> {
    const CAP: usize = 20;
    let rd = std::fs::read_dir(images_dir).ok()?;
    let mut entries: Vec<(usize, String)> = Vec::new();
    for ent in rd.filter_map(|e| e.ok()) {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        if !crate::model::attachment::has_image_extension(name) {
            continue;
        }
        let marker_n = name
            .split_once('-')
            .and_then(|(nn, _)| nn.parse::<usize>().ok())
            .unwrap_or(0);
        entries.push((marker_n, name.to_string()));
    }
    if entries.is_empty() {
        return None;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let total = entries.len();
    let mut out = String::from(
        "--- session images still on disk (not in context) ---\n",
    );
    for (marker_n, name) in entries.into_iter().take(CAP) {
        if marker_n > 0 {
            out.push_str(&format!("[Image #{marker_n}] images/{name}\n"));
        } else {
            out.push_str(&format!("images/{name}\n"));
        }
    }
    if total > CAP {
        out.push_str(&format!("+{} more in images/\n", total - CAP));
    }
    out.push_str(
        "To re-inspect any of these, call load_image with the path above (or image_n).\n---",
    );
    Some(out)
}

/// Extract `[Image #N]` marker numbers from text (same grammar as composer).
pub(crate) fn marker_numbers_in_text(text: &str) -> Vec<usize> {
    const PREFIX: &str = "[Image #";
    let mut out = Vec::new();
    for (i, _) in text.match_indices(PREFIX) {
        let after_prefix = &text[i + PREFIX.len()..];
        let digits: String = after_prefix
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() || !after_prefix[digits.len()..].starts_with(']') {
            continue;
        }
        if let Ok(n) = digits.parse::<usize>() {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "load_image_test.rs"]
mod tests;
