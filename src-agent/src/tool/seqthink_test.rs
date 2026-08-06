#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::{Arc, RwLock};

/// A minimal `ToolCtx` for tests — `seqthink` never touches it.
fn test_ctx() -> ToolCtx {
    ToolCtx {
        workspace: std::path::PathBuf::from("."),
        workspaces: vec![std::path::PathBuf::from(".")],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: None,
        download_dir: None,
        internet_mode: crate::model::settings::InternetMode::default(),
        ssh_key: None,
        skill_registry: None,
        active_skill_names: None,
        active_skill_dirs: Vec::new(),
        mcp_manager: None,
        sec_manager: None,
        bash_saving: true,
        bash_log_dir: None,
        session_dir: None,
        allow_scratch: true,
        sdlc_assess: false,
    }
}

#[test]
fn echoes_thought_number_and_next_thought_needed() {
    let ctx = test_ctx();
    let args = json!({
        "thought": "first step",
        "next_thought_needed": true,
        "thought_number": 1,
        "total_thoughts": 3
    });
    let result = SeqThink.run(&ctx, &args).unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["thought_number"], 1);
    assert_eq!(v["total_thoughts"], 3);
    assert_eq!(v["next_thought_needed"], true);
}

#[test]
fn bumps_total_thoughts_when_thought_number_exceeds_it() {
    let ctx = test_ctx();
    let args = json!({
        "thought": "went further than planned",
        "next_thought_needed": false,
        "thought_number": 5,
        "total_thoughts": 3
    });
    let result = SeqThink.run(&ctx, &args).unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["thought_number"], 5);
    assert_eq!(v["total_thoughts"], 5);
    assert_eq!(v["next_thought_needed"], false);
}

#[test]
fn missing_required_field_returns_error_string() {
    let ctx = test_ctx();
    let args = json!({
        "next_thought_needed": true,
        "thought_number": 1,
        "total_thoughts": 1
    });
    let result = SeqThink.run(&ctx, &args).unwrap();
    assert!(
        result.starts_with("error:"),
        "expected error string, got: {result}"
    );
    assert!(result.contains("thought"));
}

#[test]
fn missing_thought_number_returns_error_string() {
    let ctx = test_ctx();
    let args = json!({
        "thought": "step",
        "next_thought_needed": true,
        "total_thoughts": 1
    });
    let result = SeqThink.run(&ctx, &args).unwrap();
    assert!(
        result.starts_with("error:"),
        "expected error string, got: {result}"
    );
}
