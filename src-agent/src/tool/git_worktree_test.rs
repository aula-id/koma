#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::{Arc, RwLock};

fn test_ctx(wt: PathBuf) -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: PathBuf::from("."),
        workspaces: vec![PathBuf::from(".")],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: Some(wt),
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
        call_track: crate::tool::CallTrack::new(),
    }
}

#[test]
fn rejects_escaping_names_on_both() {
    let base = Path::new("/wt");
    for w in [false, true] {
        for bad in ["../../x", "/abs", "a/b", "..", ".", ""] {
            assert!(worktree_path(base, bad, w).is_err(), "{bad:?} windows={w}");
        }
    }
}

#[test]
fn windows_only_names() {
    let base = Path::new("/wt");
    for bad in ["C:x", "a:b", "con", "Nul.txt", "x."] {
        assert!(worktree_path(base, bad, true).is_err(), "{bad:?}");
        assert_eq!(worktree_path(base, bad, false).unwrap(), base.join(bad));
    }
    for w in [false, true] {
        assert_eq!(worktree_path(base, "wt_1", w).unwrap(), base.join("wt_1"));
    }
}

#[test]
fn run_rejects_bad_name_without_touching_fs() {
    let tmp = std::env::temp_dir().join(format!("koma-wt-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let base = tmp.join("worktrees");
    let ctx = test_ctx(base.clone());
    for action in ["create", "remove"] {
        let args = json!({"action": action, "name": "../x"});
        assert!(GitWorktree.run(&ctx, &args).is_err(), "{action}");
    }
    assert!(!base.exists());
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&tmp);
}
