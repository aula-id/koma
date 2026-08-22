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
    );
    assert!(read_validated_image(&ctx, "[1]a.png").is_ok());
    assert!(read_validated_image(&ctx, "b.png").is_ok());
    assert!(read_validated_image(&ctx, &scratch.join("c.png").display().to_string()).is_ok());
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
    let ctx = ctx(cwd, vec![ws], scratch);
    assert!(read_validated_image(&ctx, "outside.png").is_err());
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
