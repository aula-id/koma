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

fn specs_for(avatar: &str, kind: LaneKind) -> Vec<crate::room::lane::ToolSpec> {
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
    router.advertised()
}

fn names_for(avatar: &str, kind: LaneKind) -> Vec<String> {
    specs_for(avatar, kind).into_iter().map(|s| s.name).collect()
}

#[test]
fn a_request_never_carries_two_definitions_of_one_function() {
    // The provider layer adds a built-in for every name in `advertise`, then the definitions.
    for avatar in ["chief", "red", "reviewer", "reporter"] {
        for kind in [LaneKind::Chat, LaneKind::Work] {
            let (advertise, defs) = super::model_client::wire_tools(&specs_for(avatar, kind));
            let mut names: Vec<String> = crate::tool::all_tools()
                .iter()
                .filter(|t| advertise.iter().any(|n| n == t.name()))
                .map(|t| t.name().to_string())
                .collect();
            names.extend(defs.iter().map(|d| d.function.name.clone()));
            let mut seen = std::collections::BTreeSet::new();
            for name in &names {
                assert!(seen.insert(name.clone()), "{avatar}/{kind:?}: {name} is defined twice");
            }
            assert!(names.iter().any(|n| n == "read") || avatar == "clerk", "{avatar}/{kind:?} still has its tools");
        }
    }
}

#[test]
fn every_advertised_tool_has_a_valid_unique_schema() {
    for avatar in ["chief", "red", "reviewer", "reporter", "clerk"] {
        for kind in [LaneKind::Chat, LaneKind::Work] {
            let specs = specs_for(avatar, kind);
            let mut seen = std::collections::BTreeSet::new();
            for spec in &specs {
                assert!(seen.insert(spec.name.clone()), "{avatar}/{kind:?}: duplicate tool {}", spec.name);
                assert!(!spec.description.trim().is_empty(), "{avatar}/{kind:?}: {} has no description", spec.name);
                assert_eq!(spec.parameters.get("type").and_then(|t| t.as_str()), Some("object"), "{avatar}/{kind:?}: {} schema is not an object: {}", spec.name, spec.parameters);
                assert!(spec.parameters.get("properties").is_some_and(|p| p.is_object()), "{avatar}/{kind:?}: {} has no properties object: {}", spec.name, spec.parameters);
            }
        }
    }
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

fn lane_builtins(root: &std::path::Path) -> CtxBuiltins {
    let mut ctx = context(root);
    ctx.session_dir = Some(root.join("session"));
    CtxBuiltins::from_ctx(ctx)
}

fn text_of(builtins: &CtxBuiltins, avatar: &str, kind: LaneKind, name: &str, args: serde_json::Value) -> String {
    builtins.run(avatar, kind, &call(name, args))
}

#[test]
fn a_lane_runs_a_background_job_and_polls_it() {
    let root = std::env::temp_dir().join(format!("koma-room-bg-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("mkdir");
    let b = lane_builtins(&root);
    let started = text_of(&b, "red", LaneKind::Work, "bash", json!({ "command": "echo first; sleep 1; echo second", "run_in_background": true }));
    assert!(started.contains("bash-1"), "{started}");
    // Another lane has its own numbering and cannot see this job.
    let other = text_of(&b, "chief", LaneKind::Chat, "bash_output", json!({ "job_id": "bash-1" }));
    assert!(other.contains("no such job"), "{other}");

    let mut seen = String::new();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        seen = text_of(&b, "red", LaneKind::Work, "bash_output", json!({ "job_id": "bash-1", "tail_lines": 5 }));
        if seen.contains("[exit 0]") {
            break;
        }
    }
    assert!(seen.contains("[exit 0]") && seen.contains("first") && seen.contains("second"), "{seen}");

    let long = text_of(&b, "red", LaneKind::Work, "bash", json!({ "command": "sleep 30", "run_in_background": true }));
    assert!(long.contains("bash-2"), "{long}");
    let killed = text_of(&b, "red", LaneKind::Work, "bash_kill", json!({ "job_id": "bash-2" }));
    assert!(killed.contains("killed"), "{killed}");
    let status = text_of(&b, "red", LaneKind::Work, "bash_output", json!({ "job_id": "bash-2" }));
    assert!(status.contains("[killed]"), "{status}");

    let git = text_of(&b, "red", LaneKind::Work, "bash", json!({ "command": "git status", "run_in_background": true }));
    assert!(git.contains("git_operator"), "{git}");
    assert!(text_of(&b, "red", LaneKind::Work, "bash_output", json!({ "job_id": "bash-9" })).contains("no such job"));
}

#[test]
fn cd_moves_only_the_lane_that_asked() {
    let root = std::env::temp_dir().join(format!("koma-room-cd-{}", std::process::id()));
    let inner = root.join("inner");
    std::fs::create_dir_all(&inner).expect("mkdir");
    let b = lane_builtins(&root);
    let moved = text_of(&b, "red", LaneKind::Work, "cd", json!({ "path": inner.to_string_lossy() }));
    assert!(moved.contains("working directory is now"), "{moved}");
    let red = text_of(&b, "red", LaneKind::Work, "bash", json!({ "command": "pwd" }));
    assert!(red.contains("inner"), "red runs in the new directory: {red}");
    let chief = text_of(&b, "chief", LaneKind::Chat, "bash", json!({ "command": "pwd" }));
    assert!(!chief.contains("inner"), "the chief did not move: {chief}");
    let refused = text_of(&b, "red", LaneKind::Work, "cd", json!({ "path": "/" }));
    assert!(!refused.contains("working directory is now"), "outside the roots is refused: {refused}");
}

/// Tools the coding agent has that room lanes deliberately do not, each with the reason.
/// Every other chat tool must be a host handler, so adding a chat tool forces a decision.
const NOT_IN_THE_ROOM: &[(&str, &str)] = &[
    ("computer_windows", "desktop control; the lab works through the shell and the browser tools"),
    ("computer_select_window", "desktop control"),
    ("computer_observe", "desktop control"),
    ("computer_act", "desktop control"),
    ("screen_browser_tabs", "desktop browser control; the lab has browser_* tools"),
    ("screen_browser_observe", "desktop browser control"),
    ("screen_browser_act", "desktop browser control"),
    ("screen_browser_read", "desktop browser control"),
    ("screen_browser_eval", "desktop browser control"),
    ("screen_browser_wait_human", "desktop browser control"),
    ("task", "sub-agents; the room delegates with group_delegate to its own specialists"),
    ("task_output", "sub-agents"),
    ("task_kill", "sub-agents"),
    ("task_send", "sub-agents"),
    ("plan_enter", "the coding session's Plan mode"),
    ("plan_ready", "the coding session's Plan mode"),
    ("mission_ready", "the coding session's SDLC mode"),
    ("mission_draft", "the coding session's SDLC mode"),
    ("mission_clear", "the coding session's SDLC mode"),
    ("mission_verify", "the coding session's SDLC mode"),
    ("mission_prepare", "the coding session's SDLC mode"),
    ("mission_integrate", "the coding session's SDLC mode"),
    ("seqthink", "internal to the coding session"),
    ("pong", "internal plumbing"),
    ("dir_cache_update", "internal plumbing"),
    ("git_cred", "stores credentials in the coding session's settings"),
    ("git_worktree", "shadow worktrees of the coding session's project bucket"),
    ("load_screenshot", "image messages"),
    ("show_image", "image messages"),
    ("search_screenshots", "screenshot library of the coding session"),
    ("describe_screenshot", "screenshot library of the coding session"),
];

#[test]
fn every_coding_agent_tool_is_in_the_room_or_has_a_stated_reason_not_to_be() {
    use crate::room::registry::HOST_HANDLERS;
    let mut unclassified = Vec::new();
    for name in crate::tool::main_tool_names() {
        let hosted = HOST_HANDLERS.contains(&name.as_str());
        let excluded = NOT_IN_THE_ROOM.iter().any(|(n, _)| *n == name);
        assert!(!(hosted && excluded), "{name} is both hosted and excluded");
        if !hosted && !excluded {
            unclassified.push(name);
        }
    }
    assert!(unclassified.is_empty(), "decide for each: host it in the room or list it in NOT_IN_THE_ROOM: {unclassified:?}");
    for (name, _) in NOT_IN_THE_ROOM {
        assert!(
            crate::tool::all_tools().iter().any(|t| t.name() == *name),
            "{name} is listed as excluded but is not a tool any more; remove it"
        );
    }
    // Every hosted handler is a real compiled tool, so the allowlist cannot advertise a ghost.
    let compiled = super::builtins::host_tool_specs();
    for name in HOST_HANDLERS {
        assert!(compiled.iter().any(|s| s.name == *name), "{name} is a host handler with no compiled tool");
    }
}

const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn a_lane_can_load_an_image_the_user_pointed_at() {
    let root = std::env::temp_dir().join(format!("koma-room-image-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    let shot = root.join("login.png");
    std::fs::write(&shot, PNG_1X1).expect("write png");
    let b = lane_builtins(&root);

    let (result, messages) = b.run_with_messages("red", LaneKind::Chat, &call("load_image", json!({ "path": shot.to_string_lossy() })));
    assert!(result.contains("image loaded: login") && result.contains("[Image #1]"), "{result}");
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.role, crate::dto::chat::Role::User);
    assert_eq!(message.attachments.len(), 1);
    let kept = root.join("session/lanes/red.chat").join(&message.attachments[0].rel_path);
    assert!(kept.is_file(), "the lane keeps its own copy: {}", kept.display());

    // The wire builder reads it back from the lane's directory.
    let ctx = crate::dto::openrouter::ImageWireCtx { session_dir: root.join("session/lanes/red.chat"), model_takes_images: true };
    let wire = crate::dto::openrouter::to_wire_with_images(messages, Some(&ctx), false);
    let json = serde_json::to_string(&wire).expect("wire");
    assert!(json.contains("image_url") && json.contains("data:image/png"), "{json}");

    // A file outside the workspace, and a text file, are refused with a reason.
    let (outside, none) = b.run_with_messages("red", LaneKind::Chat, &call("load_image", json!({ "path": "/etc/hosts" })));
    assert!(outside.starts_with("error:") && none.is_empty(), "{outside}");
}
