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
fn web_screenshot_rejects_simple_mode() {
    let ctx = make_ctx(InternetMode::Simple);
    let args = json!({"url": "https://example.com"});
    let result = WebScreenshot.run(&ctx, &args).unwrap();
    assert!(result.contains("requires internet mode `full`"), "{result}");
}

#[test]
fn web_screenshot_rejects_bad_url() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({"url": "ftp://example.com"});
    let result = WebScreenshot.run(&ctx, &args).unwrap();
    assert!(result.contains("must start with http"), "{result}");
}

#[test]
fn web_screenshot_missing_both_args() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({});
    let result = WebScreenshot.run(&ctx, &args).unwrap();
    assert!(result.contains("provide either"), "{result}");
}

#[test]
fn web_screenshot_metadata() {
    assert_eq!(WebScreenshot.name(), "web_screenshot");
    assert!(!WebScreenshot.description().is_empty());
}

#[test]
fn screenshot_filename_basic() {
    let f = screenshot_filename("https://example.com/page");
    assert!(f.ends_with(".png"));
    assert!(f.contains("example_com"));
    assert!(f.contains("page"));
}

#[test]
fn screenshot_filename_no_path() {
    let f = screenshot_filename("https://example.com");
    assert!(f.ends_with(".png"));
    assert!(f.contains("example_com"));
}

#[test]
fn screenshot_filename_sanitises_special_chars() {
    let f = screenshot_filename("https://example.com/a/b/c?q=1&r=2");
    assert!(!f.contains('?'));
    assert!(!f.contains('='));
    assert!(f.ends_with(".png"));
}
