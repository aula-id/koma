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
    }
}

#[test]
fn browser_inspect_rejects_simple_mode() {
    let ctx = make_ctx(InternetMode::Simple);
    let args = json!({"what": "html"});
    let result = BrowserInspect.run(&ctx, &args).unwrap();
    assert!(result.contains("requires internet mode `full`"), "{result}");
}

#[test]
fn browser_inspect_rejects_bad_what() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({"what": "cookies"});
    let result = BrowserInspect.run(&ctx, &args).unwrap();
    assert!(result.contains("unknown inspect target"), "{result}");
}

#[test]
fn browser_inspect_metadata() {
    assert_eq!(BrowserInspect.name(), "browser_inspect");
    assert!(!BrowserInspect.description().is_empty());
}
