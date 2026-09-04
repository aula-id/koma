use super::*;
use crate::tool::{DirCache, Tool};
use std::sync::{Arc, RwLock};

const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

fn ctx(
    workspace: PathBuf,
    workspaces: Vec<PathBuf>,
    scratch: PathBuf,
    session_dir: Option<PathBuf>,
) -> ToolCtx {
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
        session_dir,
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
    assert!(tool.parameters()["properties"].get("path").is_some());
    assert!(tool.parameters()["properties"].get("image_n").is_some());
    // path is no longer strictly required (image_n alone is valid)
    assert!(
        tool.parameters().get("required").is_none()
            || tool.parameters()["required"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false)
    );
    assert!(crate::tool::all_tools()
        .iter()
        .any(|t| t.name() == "load_image"));
    assert!(!crate::tool::agent_selectable_tools().contains(&"load_image".to_string()));
}

#[test]
fn accepts_configured_workspace_worktree_and_exact_scratch() {
    let base = std::env::temp_dir().join(format!("koma-load-image-ok-{}", std::process::id()));
    let ws = base.join("ws");
    let worktree = base.join("worktree");
    let scratch = base.join("scratch/session-a");
    put(&ws.join("a.png"), PNG);
    put(&worktree.join("b.png"), PNG);
    put(&scratch.join("c.png"), PNG);
    let ctx = ctx(
        worktree.clone(),
        vec![worktree.clone(), ws.clone()],
        scratch.clone(),
        None,
    );
    assert!(read_validated_image(&ctx, "[1]a.png").is_ok());
    assert!(read_validated_image(&ctx, "b.png").is_ok());
    assert!(read_validated_image(&ctx, &scratch.join("c.png").display().to_string()).is_ok());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn accepts_session_images_dir_path_and_image_n() {
    let base = std::env::temp_dir().join(format!("koma-load-image-sess-{}", std::process::id()));
    let ws = base.join("ws");
    let scratch = base.join("scratch/session-a");
    let session = base.join("sessions/session-a");
    let img = session.join("images/03-foo.png");
    put(&img, PNG);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    let ctx = ctx(ws, vec![base.join("ws")], scratch, Some(session.clone()));

    assert!(
        read_validated_image(&ctx, &img.display().to_string()).is_ok(),
        "absolute under session images/"
    );
    assert!(
        read_validated_image(&ctx, "images/03-foo.png").is_ok(),
        "relative images/…"
    );
    assert!(
        read_validated_image(&ctx, "03-foo.png").is_ok(),
        "bare NN- basename"
    );
    let args = json!({"image_n": 3});
    assert!(read_validated_image_from_args(&ctx, &args).is_ok());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn session_images_resolve_prefers_messages_json_rel_path() {
    let base = std::env::temp_dir().join(format!("koma-load-image-json-{}", std::process::id()));
    let session = base.join("sess");
    let img = session.join("images/02-from-json.png");
    put(&img, PNG);
    let messages = json!([{
        "role": "user",
        "content": "see [Image #2]",
        "attachments": [{
            "marker_n": 2,
            "rel_path": "images/02-from-json.png",
            "mime": "image/png"
        }]
    }]);
    std::fs::write(
        session.join("messages.json"),
        serde_json::to_string(&messages).unwrap(),
    )
    .unwrap();
    let resolved = resolve_image_marker_in_session(&session, 2).unwrap();
    assert_eq!(resolved, img);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn rejects_live_cwd_outside_configured_roots() {
    let base = std::env::temp_dir().join(format!("koma-load-image-cwd-{}", std::process::id()));
    let ws = base.join("ws");
    let cwd = base.join("outside-cwd");
    let scratch = base.join("scratch/session-a");
    put(&cwd.join("outside.png"), PNG);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    let ctx = ctx(cwd, vec![ws], scratch, None);
    assert!(read_validated_image(&ctx, "outside.png").is_err());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn rejects_disallowed_and_invalid_sources() {
    let base = std::env::temp_dir().join(format!("koma-load-image-no-{}", std::process::id()));
    let ws = base.join("ws");
    let scratch = base.join("scratch/session-a");
    let other = base.join("scratch/session-b/x.png");
    // Persistent session file NOT under images/ must still be denied.
    let persistent_root = base.join("sessions/session-a/x.png");
    let sibling_session_images = base.join("sessions/session-b/images/01-x.png");
    let outside = base.join("outside/x.png");
    let sibling = base.join("ws-sibling/x.png");
    for p in [
        &other,
        &persistent_root,
        &sibling_session_images,
        &outside,
        &sibling,
    ] {
        put(p, PNG);
    }
    put(&ws.join("text.png"), b"not an image");
    std::fs::create_dir_all(ws.join("folder.png")).unwrap();
    put(&ws.join("nested/a.png"), PNG);
    let session_a = base.join("sessions/session-a");
    std::fs::create_dir_all(session_a.join("images")).unwrap();
    let ctx = ctx(
        ws.clone(),
        vec![ws.clone()],
        scratch,
        Some(session_a),
    );
    for p in [
        &other,
        &persistent_root,
        &sibling_session_images,
        &outside,
        &sibling,
    ] {
        assert!(
            read_validated_image(&ctx, &p.display().to_string()).is_err(),
            "should reject {}",
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
        // Symlink from session images/ out of allowlist must fail after canonicalize.
        let escape_in_images = base
            .join("sessions/session-a/images")
            .join("99-escape.png");
        std::os::unix::fs::symlink(&outside, &escape_in_images).unwrap();
        assert!(read_validated_image(&ctx, "images/99-escape.png").is_err());
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn marker_numbers_in_text_extracts_unique() {
    let nums = marker_numbers_in_text("see [Image #3] and [Image #1] and [Image #3]");
    assert_eq!(nums, vec![3, 1]);
    assert!(marker_numbers_in_text("no markers").is_empty());
    assert!(marker_numbers_in_text("[Image #]").is_empty());
}
