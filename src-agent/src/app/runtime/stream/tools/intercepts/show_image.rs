//! Show-image interceptor: opens the image viewer overlay so the user can
//! view an image in the terminal using Kitty or half-block rendering.
//!
//! The `show_image` tool's `run()` resolves the path; the actual mode
//! switch happens here because `Tool::run()` takes an immutable `ToolCtx`.

use crate::app::mode::{ImageEntry, ImageOverlayState};
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

use super::InterceptFlow;

pub(in crate::app::runtime::stream::tools) fn intercept_show_image(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    // 1. Parse the arguments.
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let path_str = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: missing required argument 'path'".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("image")
        .to_string();

    // 2. Resolve the workspace.
    let workspace = state.rest.sessions[sess_idx].effective_cwd();

    // 3. Resolve the image path.
    let image_path = match resolve_image_path(&workspace, &path_str) {
        Ok(p) => p,
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: {e}"),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    // 4. Set the ImageOverlay mode.
    let entry = ImageEntry {
        path: image_path.clone(),
        label: label.clone(),
    };
    let overlay = ImageOverlayState {
        images: vec![entry],
        active_index: 0,
        source_label: label,
        kitty: crate::view::image_render::ImageRenderer::detect().kitty
            == crate::view::image_render::KittySupport::Yes,
        kitty_placement: 0,
    };
    state.rest.sessions[sess_idx].mode =
        crate::app::mode::Mode::ImageOverlay(Box::new(overlay));

    // 5. Push the tool result (plain text).
    state.rest.sessions[sess_idx].tool_results.push((
        call.id.clone(),
        format!(
            "image viewer opened: {}\nPress Esc to close, Left/Right to navigate.",
            image_path.display()
        ),
    ));

    // 6. Advance to the next tool call.
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

/// Resolve an image path from the workspace and screenshot catalog.
fn resolve_image_path(
    workspace: &std::path::Path,
    path_str: &str,
) -> Result<std::path::PathBuf, String> {
    use std::path::Path;

    let p = Path::new(path_str);

    // 1. Absolute path to existing file.
    if p.is_absolute() && p.exists() {
        return Ok(p.to_path_buf());
    }

    // 2. Relative to workspace.
    let ws_path = workspace.join(path_str);
    if ws_path.exists() {
        return Ok(ws_path);
    }

    // 3. Screenshot catalog stem.
    let catalog_path = workspace.join(".screenshoot").join(format!("{path_str}.png"));
    if catalog_path.exists() {
        return Ok(catalog_path);
    }

    // 4. Screenshot catalog resolve.
    if let Some(resolved) =
        crate::model::screenshot_catalog::resolve_screenshot_path(workspace, path_str)
    {
        return Ok(resolved);
    }

    Err(format!(
        "image not found: tried absolute, workspace-relative, and screenshot catalog for '{path_str}'"
    ))
}
