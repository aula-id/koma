//! Image-load interceptors. Both tools share one synthetic attachment pipeline.
use super::InterceptFlow;
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use std::path::Path;

pub(in crate::app::runtime::stream::tools) fn intercept_load_screenshot(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let args = parse_args(call);
    let name = match args.get("screenshot").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => {
            return tool_error(
                state,
                sess_idx,
                call,
                "missing required argument 'screenshot'",
            )
        }
    };
    let workspace = state.rest.sessions[sess_idx].effective_cwd();
    let path = match crate::model::screenshot_catalog::resolve_screenshot_path(&workspace, name) {
        Some(v) => v,
        None => {
            return tool_error(
                state,
                sess_idx,
                call,
                &format!("screenshot '{name}' not found or not a valid PNG under .screenshoot/"),
            )
        }
    };
    let bytes = match std::fs::read(&path) {
        Ok(v) => v,
        Err(e) => {
            return tool_error(
                state,
                sess_idx,
                call,
                &format!("failed to read screenshot: {e}"),
            )
        }
    };
    inject_image(
        state,
        sess_idx,
        call,
        &path,
        bytes,
        "Screenshot",
        "screenshot",
    )
}

pub(in crate::app::runtime::stream::tools) fn intercept_load_image(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let args = parse_args(call);
    let requested = match crate::tool::internet::load_image::image_arg(&args) {
        Ok(v) => v,
        Err(e) => return tool_error(state, sess_idx, call, &e.to_string()),
    };
    let ctx = crate::app::runtime::stream::spawn::build_tool_ctx(state, sess_idx);
    let (path, bytes) =
        match crate::tool::internet::load_image::read_validated_image(&ctx, requested) {
            Ok(v) => v,
            Err(e) => return tool_error(state, sess_idx, call, &e.to_string()),
        };
    inject_image(state, sess_idx, call, &path, bytes, "Image", "image")
}

fn parse_args(call: &ToolCall) -> serde_json::Value {
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}))
}

fn inject_image(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    path: &Path,
    bytes: Vec<u8>,
    label: &str,
    result_label: &str,
) -> InterceptFlow {
    let name = path
        .file_stem()
        .and_then(|v| v.to_str())
        .or_else(|| path.file_name().and_then(|v| v.to_str()))
        .unwrap_or(result_label)
        .to_string();
    let basename = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("image.png");
    let images_dir = match state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|v| v.images_dir())
    {
        Some(v) => v,
        None => return tool_error(state, sess_idx, call, "no active session for image ingest"),
    };
    let (attachment, marker) =
        match crate::model::attachment::ingest_image_bytes(&images_dir, basename, &bytes) {
            Ok(v) => v,
            Err(e) => {
                return tool_error(
                    state,
                    sess_idx,
                    call,
                    &format!("failed to ingest {result_label}: {e}"),
                )
            }
        };
    state.rest.sessions[sess_idx].tool_results.push((
        call.id.clone(),
        format!(
            "{result_label} loaded: {name} → {marker}\n{}",
            path.display()
        ),
    ));
    if let Some(session) = state.rest.sessions[sess_idx].session.as_mut() {
        let message = format!("[{label} loaded: {name}]");
        let _ = crate::model::msglog::append(
            &session.path,
            crate::dto::chat::Role::User,
            &message,
            None,
            None,
        );
        session
            .conversation
            .push_user_with_attachments(message, vec![attachment]);
        let _ = session.save();
    }
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

fn tool_error(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    message: &str,
) -> InterceptFlow {
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), format!("error: {message}")));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}
