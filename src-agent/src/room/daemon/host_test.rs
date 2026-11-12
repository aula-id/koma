#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, RwLock};

use serde_json::json;
use tokio::sync::mpsc::unbounded_channel;

use super::selftest::run_secure_selftest;
use crate::dto::chat::{FunctionCall, ToolCall};
use crate::room::actor::{BuiltinHost, LaneRef, RoomToolRouter};
use crate::room::clerk::Clerk;
use crate::room::daemon::builtins::CtxBuiltins;
use crate::room::lane::{LaneKind, ToolRouter};
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use crate::room::work::{Coordinator, WorkStore};
use crate::tool::{execute_tool, ToolCtx};

fn context(root: &std::path::Path) -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: root.to_path_buf(),
        session_id: "sess-test".into(),
        cyber_lane: None,
        sec_block: None,
        sec_inactive: Vec::new(),
        workspaces: vec![root.to_path_buf()],
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
        bash_saving: false,
        bash_log_dir: None,
        session_dir: None,
        allow_scratch: true,
        sdlc_assess: false,
        sdlc_active_node_id: None,
        search_engine: None,
        call_track: crate::tool::CallTrack::new(),
    }
}

fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("c-{name}"),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.to_string(),
        },
    }
}

fn names_for(avatar: &str, kind: LaneKind) -> Vec<String> {
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let coordinator = Arc::new(Coordinator::new(
        WorkStore::open_in_memory().expect("store"),
        Arc::clone(&shelf),
        Arc::clone(&registry),
    ));
    let snapshot = registry.current();
    let (tx, _rx) = unbounded_channel();
    let router = RoomToolRouter::new(
        LaneRef {
            avatar: avatar.into(),
            kind,
            assignment_id: None,
            is_chief: avatar == "chief",
            is_clerk: avatar == "clerk",
        },
        coordinator,
        shelf,
        Arc::new(std::sync::Mutex::new(Clerk::new())),
        snapshot,
        Arc::new(CtxBuiltins::from_ctx(context(std::path::Path::new(".")))),
        tx,
    );
    router.advertised().into_iter().map(|s| s.name).collect()
}

#[test]
fn a_work_lane_is_offered_its_host_tools_and_a_chat_lane_is_not() {
    let builtins = CtxBuiltins::from_ctx(context(std::path::Path::new(".")));
    let red_work = names_for("red", LaneKind::Work);
    assert!(red_work.iter().any(|n| n == "bash"));
    assert!(red_work.iter().any(|n| n == "web_search"));
    assert!(red_work.iter().any(|n| n == "browser_tabs"));
    assert!(red_work.iter().all(|n| !n.starts_with("sec_")));

    let red_chat = names_for("red", LaneKind::Chat);
    assert!(red_chat.iter().all(|n| n != "bash" && n != "web_search"));
    assert!(red_chat.iter().any(|n| n == "group_task_complete"));

    let reviewer = names_for("reviewer", LaneKind::Work);
    assert!(reviewer.iter().any(|n| n == "bash"));
    assert!(reviewer
        .iter()
        .all(|n| n != "web_search" && n != "browser_tabs"));

    let chief = names_for("chief", LaneKind::Chat);
    assert!(chief.iter().any(|n| n == "group_delegate"));
    assert!(chief.iter().all(|n| n != "bash"));

    let chat_specs = builtins.specs(LaneKind::Chat);
    assert!(chat_specs.is_empty());
    assert!(builtins
        .specs(LaneKind::Work)
        .iter()
        .all(|s| !s.name.starts_with("sec_")));
}

#[test]
fn bash_runs_on_the_host_and_a_sec_tool_is_refused() {
    let root = std::env::temp_dir().join(format!(
        "koma-room-host-{}-{}",
        std::process::id(),
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let builtins = CtxBuiltins::from_ctx(context(&root));

    let out = builtins.run(
        "red",
        LaneKind::Work,
        &call("bash", json!({ "command": "echo room-host" })),
    );
    assert!(out.contains("room-host"), "{out}");
    assert!(out.contains("exit code: 0"), "{out}");

    let refused = builtins.run(
        "red",
        LaneKind::Work,
        &call("sec_http", json!({ "url": "http://example.test" })),
    );
    assert!(refused.contains("not available"), "{refused}");

    let direct = execute_tool(
        &context(&root),
        &call("sec_http", json!({ "url": "http://example.test" })),
    );
    assert!(
        direct.contains("unknown tool") || direct.contains("not available"),
        "{direct}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_secure_selftest_answers_one_line() {
    run_secure_selftest().expect("selftest");
}
