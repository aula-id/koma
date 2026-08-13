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
    let mut roots: Vec<&PathBuf> = Vec::new();
    if !ctx.workspace.as_os_str().is_empty() {
        roots.push(&ctx.workspace);
    }
    for root in &ctx.workspaces {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
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
mod tests {
    use super::*;
    use crate::tool::{DirCache, Tool};
    use std::sync::{Arc, RwLock};

    const PNG: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

    fn ctx(workspace: PathBuf, workspaces: Vec<PathBuf>, scratch: PathBuf) -> ToolCtx {
        ToolCtx {
            workspace,
            workspaces,
            dir_cache: Arc::new(RwLock::new(DirCache::default())),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            scratch_dir: Some(scratch),
            internet_mode: Default::default(),
            ssh_key: None,
            skill_registry: None,
            active_skill_names: None,
            mcp_manager: None,
            sec_manager: None,
            bash_saving: true,
            bash_log_dir: None,
            session_dir: None,
            active_skill_dirs: vec![],
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
        }
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn schema_and_registry_shape() {
        let tool = LoadImage;
        assert_eq!(tool.name(), "load_image");
        assert_eq!(tool.parameters()["required"], json!(["path"]));
        assert!(crate::tool::all_tools()
            .iter()
            .any(|t| t.name() == "load_image"));
        assert!(!crate::tool::agent_selectable_tools().contains(&"load_image".to_string()));
    }

    #[test]
    fn accepts_workspace_active_workspace_and_exact_scratch() {
        let base = std::env::temp_dir().join(format!("koma-load-image-ok-{}", std::process::id()));
        let ws = base.join("ws");
        let active = base.join("worktree");
        let scratch = base.join("scratch/session-a");
        put(&ws.join("a.png"), PNG);
        put(&active.join("b.png"), PNG);
        put(&scratch.join("c.png"), PNG);
        let ctx = ctx(active.clone(), vec![ws.clone()], scratch.clone());
        assert!(read_validated_image(&ctx, &ws.join("a.png").display().to_string()).is_ok());
        assert!(read_validated_image(&ctx, "b.png").is_ok());
        assert!(read_validated_image(&ctx, &scratch.join("c.png").display().to_string()).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn rejects_disallowed_and_invalid_sources() {
        let base = std::env::temp_dir().join(format!("koma-load-image-no-{}", std::process::id()));
        let ws = base.join("ws");
        let scratch = base.join("scratch/session-a");
        let other = base.join("scratch/session-b/x.png");
        let persistent = base.join("sessions/session-a/x.png");
        let outside = base.join("outside/x.png");
        let sibling = base.join("ws-sibling/x.png");
        for p in [&other, &persistent, &outside, &sibling] {
            put(p, PNG);
        }
        put(&ws.join("text.png"), b"not an image");
        std::fs::create_dir_all(ws.join("folder.png")).unwrap();
        put(&ws.join("nested/a.png"), PNG);
        let ctx = ctx(ws.clone(), vec![ws.clone()], scratch);
        for p in [&other, &persistent, &outside, &sibling] {
            assert!(
                read_validated_image(&ctx, &p.display().to_string()).is_err(),
                "{}",
                p.display()
            );
        }
        assert!(read_validated_image(&ctx, "../outside/x.png").is_err());
        assert!(read_validated_image(&ctx, "folder.png").is_err());
        assert!(read_validated_image(&ctx, "text.png").is_err());
        assert!(read_validated_image(&ctx, "missing.png").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, ws.join("escape.png")).unwrap();
            assert!(read_validated_image(&ctx, "escape.png").is_err());
        }
        let _ = std::fs::remove_dir_all(base);
    }
}
