#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::tool::{CallTrack, ToolCtx};
use serde_json::json;
use std::sync::{Arc, RwLock};

fn ctx_for(ws: Vec<std::path::PathBuf>) -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: ws[0].clone(),
        workspaces: ws,
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
        skill_scope: None,
        skill_rank: None,
        mcp_manager: None,
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

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("koma-search-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(unix)]
#[test]
fn grep_glob_works_through_a_symlinked_workspace() {
    let real = tmp("real");
    std::fs::write(real.join("a.txt"), "needle\n").unwrap();
    let link = std::env::temp_dir().join(format!("koma-search-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let ctx = ctx_for(vec![link.clone()]);
    let out = Grep
        .run(&ctx, &json!({"pattern": "needle", "glob": "*.txt"}))
        .unwrap();
    assert!(out.contains("a.txt"), "{out}");
    let out = Glob.run(&ctx, &json!({"pattern": "*.txt"})).unwrap();
    assert!(out.contains("a.txt"), "{out}");
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&real);
}

#[test]
fn missing_file_lists_nearest_existing_directory() {
    let ws = tmp("nf");
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("sub").join("here.txt"), "x").unwrap();
    let ctx = ctx_for(vec![ws.clone()]);
    let out = crate::tool::fs::Read
        .run(&ctx, &json!({"path": "sub/missing.txt"}))
        .unwrap_or_else(|e| e.to_string());
    assert!(out.contains("here.txt"), "{out}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[cfg(unix)]
#[test]
fn multi_workspace_grep_displays_the_configured_root_not_the_canonical_one() {
    let real = tmp("mreal");
    let other = tmp("mother");
    std::fs::write(real.join("a.txt"), "needle\n").unwrap();
    let link = std::env::temp_dir().join(format!("koma-search-mlink-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let ctx = ctx_for(vec![link.clone(), other.clone()]);
    let out = Grep
        .run(&ctx, &json!({"pattern": "needle", "glob": "**/*.txt"}))
        .unwrap();
    assert!(out.contains(&link.to_string_lossy().to_string()), "{out}");
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&real);
    let _ = std::fs::remove_dir_all(&other);
}
