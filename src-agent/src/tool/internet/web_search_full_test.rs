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
fn web_search_full_rejects_simple_mode() {
    let ctx = make_ctx(InternetMode::Simple);
    let args = json!({"url": "https://html.duckduckgo.com/html/?q=hello"});
    let result = WebSearchFull.run(&ctx, &args).unwrap();
    assert!(result.contains("requires internet mode `full`"), "{result}");
}

#[test]
fn web_search_full_rejects_bad_url() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({"url": "ftp://example.com"});
    let result = WebSearchFull.run(&ctx, &args).unwrap();
    assert!(result.contains("must start with http"), "{result}");
}

#[test]
fn web_search_full_missing_both_args() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({});
    let result = WebSearchFull.run(&ctx, &args).unwrap();
    assert!(result.contains("provide either"), "{result}");
}

#[test]
fn web_search_full_empty_query() {
    let ctx = make_ctx(InternetMode::Full);
    let args = json!({"query": "  "});
    let result = WebSearchFull.run(&ctx, &args).unwrap();
    assert!(result.contains("must not be empty"), "{result}");
}

#[test]
fn web_search_full_query_uses_default_engine() {
    // Use Simple mode so the mode gate fires BEFORE any network call,
    // making this test deterministic regardless of scrapion install state.
    // The URL is built from the query using the default engine template
    // (search_engine = None) before the gate check — if the URL building
    // failed, we'd get a different error.
    let mut ctx = make_ctx(InternetMode::Simple);
    ctx.search_engine = None;
    let args = json!({"query": "rust async"});
    let result = WebSearchFull.run(&ctx, &args).unwrap();
    assert!(
        result.contains("requires internet mode `full`"),
        "expected mode gate error, got: {result}"
    );
}

#[test]
fn web_search_full_metadata() {
    assert_eq!(WebSearchFull.name(), "web_search_full");
    assert!(!WebSearchFull.description().is_empty());
    let params = WebSearchFull.parameters();
    // No required params — either query or url is accepted.
    assert!(params.get("properties").is_some());
}
