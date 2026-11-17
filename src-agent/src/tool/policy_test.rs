#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::dto::chat::{FunctionCall, ToolCall};

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("c-{name}"),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: "{}".into(),
        },
    }
}

fn ctx() -> crate::tool::ToolCtx {
    crate::tool::ToolCtx {
        plan_read_only: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: std::env::temp_dir(),
        workspaces: vec![std::env::temp_dir()],
        dir_cache: std::sync::Arc::new(std::sync::RwLock::new(crate::tool::DirCache::default())),
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
        call_track: crate::tool::CallTrack::new(),
    }
}

#[test]
fn globs_match_the_whole_name() {
    assert!(glob_match("web_fetch", "web_fetch"));
    assert!(!glob_match("web_fetch", "web_fetch2"));
    assert!(!glob_match("web_fetch", "xweb_fetch"));
    assert!(glob_match("computer_*", "computer_act"));
    assert!(glob_match("computer_*", "computer_"));
    assert!(!glob_match("computer_*", "screen_browser_act"));
    assert!(glob_match("mcp__*", "mcp__server__tool"));
    assert!(glob_match("*", "anything"));
    assert!(glob_match("*_act", "computer_act"));
    assert!(glob_match("a?c", "abc"));
    assert!(!glob_match("a?c", "ac"));
    assert!(glob_match("a*b*c", "aXXbYYc"));
    assert!(!glob_match("a*b*c", "aXXbYY"));
    assert!(!glob_match("Web_Fetch", "web_fetch"));
    assert!(!glob_match("", "x"));
}

#[test]
fn list_parsing_and_validation() {
    assert_eq!(parse_list("a, b  c,,d\n e"), vec!["a", "b", "c", "d", "e"]);
    assert!(parse_list("  ,, ").is_empty());
    let ok = validate(&["web_*".into(), "bash".into(), "bash".into()]).unwrap();
    assert_eq!(ok, vec!["web_*", "bash"]);
    assert!(validate(&["".into()]).is_err());
    assert!(validate(&["two words".into()]).is_err());
    assert!(validate(&["a,b".into()]).is_err());
    assert!(validate(&["a\u{0}b".into()]).is_err());
    assert!(validate(&["x".repeat(MAX_PATTERN_CHARS + 1)]).is_err());
    assert!(validate(&vec!["x".to_string(); MAX_PATTERNS + 1]).is_err());
    assert!(validate(&[]).unwrap().is_empty());
}

#[test]
fn config_document_is_read_for_its_deny_list() {
    let doc = serde_json::json!({"tool_policy": {"deny": ["web_*", 7, "bash"]}});
    assert_eq!(deny_of(&doc), vec!["web_*", "bash"]);
    assert!(deny_of(&serde_json::json!({})).is_empty());
    assert!(deny_of(&serde_json::json!({"tool_policy": {}})).is_empty());
    assert!(deny_of(&serde_json::json!({"tool_policy": {"deny": "bash"}})).is_empty());
}

#[test]
fn config_keeps_tool_policy_through_a_round_trip_and_hides_it_when_empty() {
    let mut config = crate::model::app_config::AppConfig::default();
    let bare = serde_json::to_value(&config).unwrap();
    assert!(bare.get("tool_policy").is_none());
    config.tool_policy.deny = vec!["computer_*".into()];
    let text = serde_json::to_string(&config).unwrap();
    let back: crate::model::app_config::AppConfig = serde_json::from_str(&text).unwrap();
    assert_eq!(back.tool_policy.deny, vec!["computer_*"]);
    let older: crate::model::app_config::AppConfig = serde_json::from_str("{}").unwrap();
    assert!(older.tool_policy.is_empty());
}

#[test]
fn nothing_is_denied_by_default() {
    assert!(!is_denied("web_fetch"));
    assert!(!is_denied("bash"));
}

#[test]
fn execute_tool_refuses_a_denied_tool_and_runs_the_rest() {
    let c = ctx();
    assert!(crate::tool::execute_tool(&c, &call("pong")).starts_with("pong!"));
    let _deny = test_deny(&["pong"]);
    let refused = crate::tool::execute_tool(&c, &call("pong"));
    assert_eq!(refused, refusal("pong"));
    assert!(refused.starts_with("error: "));
    // A different tool still runs.
    assert!(!crate::tool::execute_tool(&c, &call("cd")).contains("disabled by the tool policy"));
}

#[test]
fn execute_tool_refuses_an_mcp_tool_by_glob_even_when_unknown() {
    let _deny = test_deny(&["mcp__*"]);
    let out = crate::tool::execute_tool(&ctx(), &call("mcp__files__write"));
    assert_eq!(out, refusal("mcp__files__write"));
}

#[test]
fn execute_tool_refuses_a_denied_tool_even_in_plan_mode() {
    let c = ctx();
    c.plan_read_only
        .store(true, std::sync::atomic::Ordering::Release);
    let _deny = test_deny(&["pong"]);
    assert_eq!(
        crate::tool::execute_tool(&c, &call("pong")),
        refusal("pong")
    );
}

#[test]
fn a_denied_tool_is_never_advertised() {
    assert!(crate::tool::main_tool_names().iter().any(|n| n == "pong"));
    let _deny = test_deny(&["pong", "computer_*", "web_fetch"]);
    let names = crate::tool::main_tool_names();
    assert!(!names.iter().any(|n| n == "pong"));
    assert!(!names.iter().any(|n| n.starts_with("computer_")));
    assert!(!names.iter().any(|n| n == "web_fetch"));
    assert!(names.iter().any(|n| n == "read"));
    let mut list = vec!["read".to_string(), "plan_enter".into(), "mcp__a__b".into()];
    drop(_deny);
    let _deny = test_deny(&["plan_enter", "mcp__*"]);
    retain_allowed(&mut list);
    assert_eq!(list, vec!["read"]);
}

#[test]
fn the_environment_variable_adds_to_the_list() {
    // A pattern no real tool or other test uses, so the process-wide variable is harmless.
    let name = "env_deny_probe_tool";
    assert!(!is_denied(name));
    std::env::set_var(ENV_DENY, "other_probe,env_deny_*  third_probe");
    assert!(is_denied(name));
    assert!(is_denied("third_probe"));
    assert!(!is_denied("pong"));
    assert_eq!(
        crate::tool::execute_tool(&ctx(), &call(name)),
        refusal(name)
    );
    std::env::remove_var(ENV_DENY);
    assert!(!is_denied(name));
}
