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
    inject_image(state, sess_idx, call, &path, bytes, "Screenshot", None)
}

pub(in crate::app::runtime::stream::tools) fn intercept_load_image(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let args = parse_args(call);
    let ctx = crate::app::runtime::stream::spawn::build_tool_ctx(state, sess_idx);
    let (path, bytes) =
        match crate::tool::internet::load_image::read_validated_image_from_args(&ctx, &args) {
            Ok(v) => v,
            Err(e) => return tool_error(state, sess_idx, call, &e.to_string()),
        };
    if crate::tool::internet::load_image::inspection_requested(&args) {
        let mut inspection = match crate::tool::internet::image_inspection::inspect(&bytes, &args) {
            Ok(value) => value,
            Err(e) => return tool_error(state, sess_idx, call, &e.to_string()),
        };
        inspection.metadata["source"]["path"] = path.to_string_lossy().to_string().into();
        inspection.metadata["tool_call_id"] = call.id.clone().into();
        if let Some(png) = inspection.png {
            // Always PNG; ingest allocates a new artifact without overwriting source.
            return inject_image(
                state,
                sess_idx,
                call,
                &path.with_file_name("image-inspection.png"),
                png,
                "Image",
                Some(inspection.metadata),
            );
        }
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), inspection.metadata.to_string()));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    inject_image(state, sess_idx, call, &path, bytes, "Image", None)
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
    mut inspection: Option<serde_json::Value>,
) -> InterceptFlow {
    if !crate::app::runtime::computer::bridge::main_accepts_images(state, sess_idx) {
        return tool_error(state, sess_idx, call, "The existing Main model does not support image input. Select an image-capable Main model explicitly; use load_image with attach=false for numeric image inspection.");
    }
    let result_label = label.to_ascii_lowercase();
    let name = path
        .file_stem()
        .and_then(|v| v.to_str())
        .or_else(|| path.file_name().and_then(|v| v.to_str()))
        .unwrap_or(&result_label)
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
    if let Some(metadata) = &mut inspection {
        metadata["image_n"] = attachment.marker_n.into();
        metadata["image_path"] = images_dir
            .parent()
            .unwrap_or(&images_dir)
            .join(&attachment.rel_path)
            .to_string_lossy()
            .to_string()
            .into();
    }
    let result = if let Some(metadata) = &inspection {
        metadata.to_string()
    } else {
        format!(
            "{result_label} loaded: {name} → {marker}\n{}",
            path.display()
        )
    };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result));
    if let Some(session) = state.rest.sessions[sess_idx].session.as_mut() {
        // Provenance and samples stay in the tool details, not a raw chat block.
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

#[cfg(test)]
mod image_inspection_tests {
    use super::*;
    use crate::{
        app::mode::Mode,
        dto::chat::FunctionCall,
        model::{conversation::Conversation, session::Session},
    };
    use image::GenericImageView;
    use serde_json::json;

    #[test]
    fn image_inspection_ingests_crop_once_and_numeric_sampling_adds_no_attachment() {
        let dir = std::env::temp_dir().join(format!("koma-image-inspect-{}", uuid::Uuid::new_v4()));
        let images = dir.join("images");
        std::fs::create_dir_all(&images).unwrap();
        let source_path = images.join("01-source.png");
        let source = image::RgbaImage::from_fn(8, 6, |x, y| {
            image::Rgba([x as u8 * 20, y as u8 * 30, 127, 255])
        });
        source.save(&source_path).unwrap();
        std::fs::write(images.join(".seq"), "1").unwrap();
        let original = std::fs::read(&source_path).unwrap();
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions[0].session = Some(Session::new(
            // Keep the store-derived images directory inside this fixture too.
            dir.to_string_lossy().into_owned(),
            dir.clone(),
            "test".into(),
            Default::default(),
            Conversation::from_messages(vec![]),
        ));
        let call = |id: &str, args: serde_json::Value| ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "load_image".into(),
                arguments: args.to_string(),
            },
        };
        intercept_load_image(
            &mut state,
            0,
            &call(
                "colors",
                json!({"image_n":1,"attach":false,"points":[{"x":3,"y":2}]}),
            ),
        );
        let rt = &state.rest.sessions[0];
        let numeric: serde_json::Value =
            serde_json::from_str(&rt.tool_results.last().unwrap().1).unwrap();
        assert_eq!(numeric["samples"][0]["hex"], "#3C3C7F");
        assert!(rt
            .session
            .as_ref()
            .unwrap()
            .conversation
            .messages()
            .is_empty());
        assert_eq!(std::fs::read_to_string(images.join(".seq")).unwrap(), "1");
        intercept_load_image(
            &mut state,
            0,
            &call(
                "crop",
                json!({"image_n":1,"crop":{"x":2,"y":1,"width":3,"height":2},"points":[{"x":3,"y":2}]}),
            ),
        );
        let rt = &state.rest.sessions[0];
        let raw = &rt.tool_results.last().unwrap().1;
        let result: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("{error}: {raw}"));
        assert_eq!(result["inspection_only"], true);
        assert_eq!(result["image_n"], 2);
        assert_eq!(result["tool_call_id"], "crop");
        let message = rt
            .session
            .as_ref()
            .unwrap()
            .conversation
            .messages()
            .last()
            .unwrap();
        assert_eq!(message.attachments.len(), 1);
        let crop_path = dir.join(&message.attachments[0].rel_path);
        assert_eq!(
            crop_path.to_string_lossy(),
            result["image_path"].as_str().unwrap()
        );
        let crop = image::open(&crop_path).unwrap();
        assert_eq!(crop.dimensions(), (3, 2));
        assert_eq!(crop.get_pixel(1, 1).0, [60, 60, 127, 255]);
        assert!(message.content.starts_with("[Image loaded:"));
        assert!(!message.content.contains("source_pixels_per_image"));
        assert_eq!(std::fs::read(source_path).unwrap(), original);
        assert!(
            !rt.computer.actionable,
            "saved inspection must not create an actionable desktop observation"
        );
        assert!(rt.computer.outbound.is_none());
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
