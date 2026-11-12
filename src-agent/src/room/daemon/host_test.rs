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
fn every_lane_of_a_role_is_offered_the_hosts_tools_its_role_grants() {
    let builtins = CtxBuiltins::from_ctx(context(std::path::Path::new(".")));
    let red_work = names_for("red", LaneKind::Work);
    assert!(red_work.iter().any(|n| n == "bash"));
    assert!(red_work.iter().any(|n| n == "web_search"));
    assert!(red_work.iter().any(|n| n == "browser_tabs"));
    assert!(red_work.iter().all(|n| !n.starts_with("sec_")));

    let red_chat = names_for("red", LaneKind::Chat);
    assert!(red_chat.iter().any(|n| n == "bash"));
    assert!(red_chat.iter().any(|n| n == "web_search"));
    assert!(red_chat.iter().any(|n| n == "group_task_complete"));

    let reviewer = names_for("reviewer", LaneKind::Work);
    assert!(reviewer.iter().any(|n| n == "bash"));
    assert!(reviewer
        .iter()
        .all(|n| n != "web_search" && n != "browser_tabs"));

    let chief = names_for("chief", LaneKind::Chat);
    assert!(chief.iter().any(|n| n == "group_delegate"));
    for tool in ["bash", "read", "write", "edit", "grep", "web_search"] {
        assert!(chief.iter().any(|n| n == tool), "the chief lacks {tool}");
    }
    assert!(chief.iter().all(|n| !n.starts_with("sec_")));

    let clerk = names_for("clerk", LaneKind::Work);
    assert!(clerk.iter().all(|n| n != "bash" && n != "read"), "the clerk gets no host tools");

    assert_eq!(builtins.specs(LaneKind::Chat), builtins.specs(LaneKind::Work));
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

#[test]
fn message_find_reads_the_lanes_own_archive() {
    use crate::dto::chat::{ChatMessage, Role};
    use crate::room::lane::{lane_dir, Transcript};

    let root = std::env::temp_dir().join(format!("koma-room-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut ctx = context(&root);
    ctx.session_dir = Some(root.clone());
    let lanes = root.join("lanes");

    let mut chief = Transcript::open_dir(lane_dir(&lanes, "chief", LaneKind::Chat));
    chief.push(ChatMessage::new(Role::User, "the staging host is zebra-17"));
    let mut red = Transcript::open_dir(lane_dir(&lanes, "red", LaneKind::Work));
    red.push(ChatMessage::new(Role::User, "port scan finished on quokka-9"));

    let builtins = CtxBuiltins::from_ctx(ctx);
    let find = |avatar: &str, kind: LaneKind, query: &str| {
        builtins.run(avatar, kind, &call("message_find", json!({ "query": query })))
    };
    let own = find("chief", LaneKind::Chat, "zebra-17");
    assert!(own.contains("zebra-17"), "{own}");
    let other = find("chief", LaneKind::Chat, "quokka-9");
    assert!(!other.contains("port scan finished"), "a lane must not read another lane's archive: {other}");
    assert!(find("red", LaneKind::Work, "quokka-9").contains("quokka-9"));
}

#[test]
fn drss_shapes_a_long_lane_from_its_archive_and_keeps_the_live_turn() {
    use crate::app::runtime::shortsend::{shape, GoalWire};
    use crate::dto::chat::{ChatMessage, Role};
    use crate::room::lane::Transcript;
    use crate::service::context_limits::ContextLimits;

    let dir = std::env::temp_dir().join(format!("koma-room-drss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut lane = Transcript::open_dir(dir.clone());
    for n in 0..60 {
        lane.push(ChatMessage::new(Role::User, format!("question {n}: {}", "alpha beta gamma ".repeat(120))));
        lane.push(ChatMessage::new(Role::Assistant, format!("answer {n}: {}", "delta epsilon ".repeat(120))));
    }
    lane.push(ChatMessage::new(Role::User, "what is the staging host?"));

    let mut history = vec![ChatMessage::new(Role::System, "you are the chief")];
    history.extend(lane.messages().iter().cloned());
    let before = history.len();

    let limits = ContextLimits { requested_model: "m".into(), matched_model: None, match_method: "test".into(), catalogue_source: "test".into(), catalog_window: None, route_window: None, effective_window: 24_000, auto_output_tokens: 2_000, max_completion_tokens: None };
    let goal = GoalWire { source: "room".into(), objective: "find the staging host".into(), charter: String::new() };
    let shaped = shape(history, &dir, &crate::model::settings::Settings::default(), "what is the staging host?", &goal, &limits, 500).expect("shape");

    assert!(shaped.drss_active, "a lane over its window is condensed");
    assert!(shaped.history.len() < before, "older turns left the outgoing request");
    assert_eq!(shaped.history.first().map(|m| m.role), Some(Role::System));
    assert_eq!(shaped.history.last().map(|m| m.content.as_str()), Some("what is the staging host?"), "the live user turn is protected");
    assert_eq!(lane.messages().len(), before - 1, "the lane's own record is untouched (all turns, no system message)");
}
