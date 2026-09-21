#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::tool::{CallTrack, ToolCtx};
use serde_json::json;
use std::sync::{Arc, RwLock};

fn ctx() -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: std::path::PathBuf::from("."),
        workspaces: vec![std::path::PathBuf::from(".")],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: None,
        download_dir: None,
        scratch_dir: None,
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
        sdlc_active_node_id: None,
        search_engine: None,
        call_track: CallTrack::new(),
    }
}

#[test]
fn short_output_passes_through() {
    let raw = "hello\nworld".to_string();
    assert_eq!(clip_tool_output("read", raw.clone()), raw);
}

#[test]
fn char_cap_wins_over_line_cap() {
    // One long line — well under 20 lines, over 20k chars.
    let raw = "x".repeat(MAX_TOOL_OUTPUT_CHARS + 50);
    let out = clip_tool_output("read", raw);
    assert!(out.contains("[truncated:"));
    assert!(out.contains("offset/limit"));
    let body = out.split("\n\n[truncated:").next().unwrap();
    assert_eq!(body.chars().count(), MAX_TOOL_OUTPUT_CHARS);
    assert_eq!(line_count(body), 1);
}

#[test]
fn line_cap_applies_when_under_char_budget() {
    let raw = (0..50)
        .map(|i| format!("line-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let out = clip_tool_output("bash", raw);
    assert!(out.contains("[truncated:"));
    let body = out.split("\n\n[truncated:").next().unwrap();
    assert_eq!(line_count(body), MAX_TOOL_OUTPUT_LINES);
    assert!(body.chars().count() < MAX_TOOL_OUTPUT_CHARS);
    assert!(body.contains("line-0"));
    assert!(body.contains("line-19"));
    assert!(!body.contains("line-20"));
}

#[test]
fn subagent_output_is_not_clipped() {
    let raw = (0..80)
        .map(|i| format!("report-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let out = clip_tool_output("task", raw.clone());
    assert_eq!(out, raw);
    assert!(!out.contains("[truncated:"));
}

#[test]
fn media_workdir_sentinel_survives_clip() {
    let mut raw = String::from("MEDIA_WORKDIR:/tmp/koma/media\n");
    raw.push_str(&"y\n".repeat(40));
    let out = clip_tool_output("web_download", raw);
    assert!(out.starts_with("MEDIA_WORKDIR:/tmp/koma/media\n"));
    assert!(out.contains("[truncated:"));
}

#[test]
fn fingerprint_sorts_object_keys() {
    let a = json!({"path": "a.rs", "offset": 0, "limit": 20});
    let b = json!({"limit": 20, "offset": 0, "path": "a.rs"});
    assert_eq!(fingerprint("read", &a), fingerprint("read", &b));
    let c = json!({"path": "a.rs", "offset": 20, "limit": 20});
    assert_ne!(fingerprint("read", &a), fingerprint("read", &c));
}

#[test]
fn repeat_warns_only_on_exact_same_args() {
    let t = ctx();
    let same = json!({"path": "desk.rs", "offset": 0, "limit": 20});
    let other = json!({"path": "desk.rs", "offset": 20, "limit": 20});

    let first = finish_tool_output(&t, "read", &same, "ok".into());
    assert!(!first.contains("[repeat:"));

    let second = finish_tool_output(&t, "read", &same, "ok".into());
    assert!(second.contains("[repeat:"));
    assert!(second.contains("2 times"));
    assert!(second.contains("path/offset/limit"));

    let paged = finish_tool_output(&t, "read", &other, "ok".into());
    assert!(!paged.contains("[repeat:"));
}

#[test]
fn grep_and_bash_repeat_use_their_own_wording() {
    let t = ctx();
    let grep = json!({"pattern": "TODO", "path": "src"});
    let _ = finish_tool_output(&t, "grep", &grep, "hit".into());
    let again = finish_tool_output(&t, "grep", &grep, "hit".into());
    assert!(again.contains("exact grep"));

    let cmd = json!({"command": "ls -la"});
    let _ = finish_tool_output(&t, "bash", &cmd, "out".into());
    let again = finish_tool_output(&t, "bash", &cmd, "out".into());
    assert!(again.contains("exact command"));
}

#[test]
fn subagent_is_not_tracked() {
    let t = ctx();
    let args = json!({"agent": "explore", "prompt": "look around"});
    let first = finish_tool_output(&t, "task", &args, "report\n".repeat(30));
    let second = finish_tool_output(&t, "task", &args, "report\n".repeat(30));
    assert!(!first.contains("[repeat:"));
    assert!(!second.contains("[repeat:"));
    assert!(!second.contains("[truncated:"));
}
