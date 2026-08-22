//! `load_image` tool — securely load an existing workspace or session-scratch image.

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
        "Load an existing image file from a configured workspace or this session's exact scratch directory into the next model message for visual inspection."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Image path. Relative paths resolve in configured workspaces; absolute paths must be inside a workspace or this session's scratch directory."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let requested = image_arg(args)?;
        let (path, _) = read_validated_image(ctx, requested)?;
        Ok(format!("load_image: validated {}", path.display()))
    }
}

pub(crate) fn image_arg(args: &Value) -> Result<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required non-empty string argument 'path'"))
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
        bail!("image path is outside configured workspaces and this session's scratch directory");
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

fn image_workspace_roots(ctx: &ToolCtx) -> Vec<&PathBuf> {
    ctx.workspaces.iter().collect()
}

fn allowed_canonical_path(ctx: &ToolCtx, canonical: &Path) -> bool {
    image_workspace_roots(ctx).into_iter().any(|root| {
        root.canonicalize()
            .map(|root| canonical.starts_with(root))
            .unwrap_or(false)
    }) || ctx.scratch_dir.as_ref().is_some_and(|scratch| {
        scratch
            .canonicalize()
            .map(|scratch| canonical.starts_with(scratch))
            .unwrap_or(false)
    })
}

#[cfg(test)]
#[path = "load_image_test.rs"]
mod tests;
