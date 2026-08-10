//! Show-image interceptor: resolves an image path and confirms it exists.
//!
//! The image is rendered inline in the chat transcript by the transcript
//! renderer — no separate overlay mode is needed.

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

use super::InterceptFlow;

pub(in crate::app::runtime::stream::tools) fn intercept_show_image(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
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

    let workspace = state.rest.sessions[sess_idx].effective_cwd();

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

    state.rest.sessions[sess_idx].tool_results.push((
        call.id.clone(),
        format!(
            "image resolved: {} ({label})\nRendered inline in the chat transcript.",
            image_path.display()
        ),
    ));

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

    if p.is_absolute() && p.exists() {
        return Ok(p.to_path_buf());
    }

    let ws_path = workspace.join(path_str);
    if ws_path.exists() {
        return Ok(ws_path);
    }

    let catalog_path = workspace.join(".screenshoot").join(format!("{path_str}.png"));
    if catalog_path.exists() {
        return Ok(catalog_path);
    }

    if let Some(resolved) =
        crate::model::screenshot_catalog::resolve_screenshot_path(workspace, path_str)
    {
        return Ok(resolved);
    }

    Err(format!(
        "image not found: tried absolute, workspace-relative, and screenshot catalog for '{path_str}'"
    ))
}
