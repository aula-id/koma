//! Screenshot-load interceptor: loads a screenshot PNG into the conversation
//! as an image attachment so the model can visually inspect it.
//!
//! The `load_screenshot` tool's `run()` only validates/resolves; the actual
//! image attachment injection happens here because `Tool::run()` cannot produce
//! image-bearing tool results.

use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

use super::InterceptFlow;

pub(in crate::app::runtime::stream::tools) fn intercept_load_screenshot(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    // 1. Parse the `screenshot` argument.
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let name = match args.get("screenshot").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: missing required argument 'screenshot'".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    // 2. Resolve the workspace (same as build_tool_ctx).
    let workspace = state.rest.sessions[sess_idx].effective_cwd();

    // 3. Resolve screenshot path using the catalog.
    let resolved = match crate::model::screenshot_catalog::resolve_screenshot_path(&workspace, name)
    {
        Some(p) => p,
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!(
                    "error: screenshot '{name}' not found or not a valid PNG under .screenshoot/"
                ),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    let stem = resolved
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();

    // 4. Get the session's images_dir.
    let images_dir = match state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.images_dir())
    {
        Some(d) => d,
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: no active session for image ingest".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    // 5. Ingest the PNG into the session's image store.
    let (attachment, marker) =
        match crate::model::attachment::ingest_image_from_path(&images_dir, &resolved) {
            Ok(r) => r,
            Err(e) => {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!("error: failed to ingest screenshot: {e}"),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        };

    // 6. Push the tool result (plain text).
    state.rest.sessions[sess_idx].tool_results.push((
        call.id.clone(),
        format!(
            "screenshot loaded: {stem} → {marker}\n{}",
            resolved.display()
        ),
    ));

    // 7. Push a synthetic user message carrying the image attachment so the
    //    model can visually inspect it. This goes BEFORE the next model hop,
    //    using the same pattern as mid-turn steers (see dispatch.rs:266-273).
    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        let _ = crate::model::msglog::append(
            &sess.path,
            crate::dto::chat::Role::User,
            &format!("[Screenshot loaded: {stem}]"),
            None,
            None,
        );
        sess.conversation
            .push_user_with_attachments(format!("[Screenshot loaded: {stem}]"), vec![attachment]);
        let _ = sess.save();
    }

    // 8. Advance to the next tool call.
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}
