use super::*;
use crate::model::settings::InternetMode;
use crate::tool::{Tool, ToolCtx};
use std::sync::{Arc, RwLock};

fn make_ctx(internet_mode: InternetMode) -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: std::env::temp_dir(),
        workspaces: vec![std::env::temp_dir()],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: None,
        download_dir: None,
        scratch_dir: None,
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
        call_track: crate::tool::CallTrack::new(),
        repeat_notices: crate::tool::new_repeat_notices(),
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
