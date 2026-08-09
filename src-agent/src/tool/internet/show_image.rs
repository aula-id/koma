//! `show_image` tool — open the terminal image viewer overlay.
//!
//! Resolves an image path (workspace file or screenshot catalog entry) and
//! opens the ImageOverlay mode so the user can view it in the terminal.

use super::ToolCtx;
use anyhow::Result;
use serde_json::{json, Value};

pub struct ShowImage;

impl super::Tool for ShowImage {
    fn name(&self) -> &'static str {
        "show_image"
    }

    fn description(&self) -> &'static str {
        "Open the terminal image viewer for a PNG/JPEG image. Resolves paths from the workspace, \
         screenshot catalog, or absolute paths. The viewer opens as a fullscreen overlay in the TUI."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the image file, or a screenshot catalog stem (filename without extension)."
                },
                "label": {
                    "type": "string",
                    "description": "Optional label for the image (e.g. 'screenshot', 'attachment')."
                }
            },
            "required": ["path"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let path_str = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'path'"))?;

        let label = args
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("image")
            .to_string();

        // Resolve the path: try as absolute, then as screenshot catalog stem.
        let image_path = resolve_image_path(ctx, path_str)?;

        // We can't set the mode directly from ToolCtx (it's immutable), so we
        // return the resolved path and let the caller (interceptor or tool
        // dispatch) set the overlay. For now, just return what we found.
        Ok(format!(
            "image viewer: {}\nsource: {}\nPress Esc to close, Left/Right to navigate if multiple.",
            image_path.display(),
            label
        ))
    }
}

fn resolve_image_path(ctx: &ToolCtx, path_str: &str) -> Result<std::path::PathBuf> {
    use std::path::Path;

    let p = Path::new(path_str);

    // 1. Absolute path to existing file.
    if p.is_absolute() && p.exists() {
        return Ok(p.to_path_buf());
    }

    // 2. Relative to workspace.
    let ws_path = ctx.workspace.join(path_str);
    if ws_path.exists() {
        return Ok(ws_path);
    }

    // 3. Screenshot catalog stem: look in .screenshoot/<stem>.png
    let catalog_path = ctx
        .workspace
        .join(".screenshoot")
        .join(format!("{path_str}.png"));
    if catalog_path.exists() {
        return Ok(catalog_path);
    }

    // 4. Screenshot catalog: try resolve via the catalog helper.
    if let Some(resolved) =
        crate::model::screenshot_catalog::resolve_screenshot_path(&ctx.workspace, path_str)
    {
        return Ok(resolved);
    }

    anyhow::bail!("image not found: tried absolute, workspace-relative, and screenshot catalog for '{path_str}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::settings::InternetMode;
    use crate::tool::{Tool, ToolCtx};
    use std::sync::{Arc, RwLock};

    fn make_ctx(internet_mode: InternetMode) -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            workspaces: vec![std::env::temp_dir()],
            dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            internet_mode,
            ssh_key: None,
            skill_registry: None,
            active_skill_names: None,
            mcp_manager: None,
            sec_manager: None,
            bash_saving: false,
            bash_log_dir: None,
            session_dir: None,
            active_skill_dirs: vec![],
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
        }
    }

    #[test]
    fn show_image_missing_path() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({});
        let result = ShowImage.run(&ctx, &args);
        assert!(result.is_err());
    }

    #[test]
    fn show_image_metadata() {
        assert_eq!(ShowImage.name(), "show_image");
        assert!(!ShowImage.description().is_empty());
    }
}
