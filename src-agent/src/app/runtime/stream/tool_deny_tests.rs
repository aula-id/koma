//! The hard tool deny list on the main turn loop: calls the model streamed
//! natively, calls harvested from its text, and calls that an intercept or a
//! bridge would otherwise run before `execute_tool` is reached.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::app::state::AppState;
use crate::dto::chat::{FunctionCall, ToolCall};
use crate::tool::policy::{refusal, test_deny};

fn fixture() -> (AppState, std::path::PathBuf, tokio::runtime::Runtime) {
    let mut state = AppState::new(crate::app::mode::Mode::Chat);
    let dir = std::env::temp_dir().join(format!("koma-tool-deny-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let rt = state.rest.fg_mut();
    rt.session = Some(crate::model::session::Session::new(
        "s".into(),
        dir.clone(),
        "test".into(),
        Default::default(),
        crate::model::conversation::Conversation::from_messages(vec![]),
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    (state, dir, runtime)
}

fn call(id: &str, name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.to_string(),
        },
    }
}

/// Everything the model would be shown for `id`: the stored result, or the
/// tool message already committed to the conversation.
fn result_for(state: &AppState, id: &str) -> Option<String> {
    let rt = &state.rest.sessions[0];
    if let Some((_, r)) = rt.tool_results.iter().find(|(i, _)| i == id) {
        return Some(r.clone());
    }
    rt.session.as_ref().and_then(|s| {
        s.conversation
            .messages()
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some(id))
            .map(|m| m.content.clone())
    })
}

/// Every tool result the turn produced, committed or still waiting.
fn tool_messages(state: &AppState) -> Vec<String> {
    let rt = &state.rest.sessions[0];
    let mut out: Vec<String> = rt.tool_results.iter().map(|(_, r)| r.clone()).collect();
    if let Some(s) = rt.session.as_ref() {
        out.extend(
            s.conversation
                .messages()
                .iter()
                .filter(|m| m.role == crate::dto::chat::Role::Tool)
                .map(|m| m.content.clone()),
        );
    }
    out
}

#[test]
fn native_calls_to_a_denied_tool_get_an_error_and_never_run() {
    let (mut state, dir, runtime) = fixture();
    let _deny = test_deny(&["pong", "computer_*", "bash", "task"]);
    for (id, name, args) in [
        ("p", "pong", serde_json::json!({})),
        ("c", "computer_windows", serde_json::json!({})),
        (
            "b",
            "bash",
            serde_json::json!({"command": "echo hi", "run_in_background": true}),
        ),
        (
            "t",
            "task",
            serde_json::json!({"agent": "general", "task": "x"}),
        ),
    ] {
        state.rest.sessions[0]
            .pending_tool_calls
            .push(call(id, name, args));
    }
    super::process_tools(&mut state, 0, &None, runtime.handle());
    for (id, name) in [
        ("p", "pong"),
        ("c", "computer_windows"),
        ("b", "bash"),
        ("t", "task"),
    ] {
        assert_eq!(
            result_for(&state, id).as_deref(),
            Some(refusal(name).as_str()),
            "{name}"
        );
    }
    let rt = &state.rest.sessions[0];
    // No bridge was entered, no job started, no sub-agent spawned.
    assert!(rt.computer.outbound.is_none());
    assert!(rt.pending_tool_tasks.is_empty());
    assert!(rt.subagents.is_empty());
    assert!(!rt.awaiting_approval && !rt.awaiting_classify);
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_allowed_tool_still_runs_beside_a_denied_one() {
    let (mut state, dir, runtime) = fixture();
    state.rest.sessions[0]
        .pending_tool_calls
        .push(call("p", "pong", serde_json::json!({})));
    {
        let _deny = test_deny(&["web_fetch"]);
        super::process_tools(&mut state, 0, &None, runtime.handle());
    }
    assert!(result_for(&state, "p").unwrap().starts_with("pong!"));
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_call_harvested_from_text_is_refused_like_a_native_one() {
    let (mut state, dir, runtime) = fixture();
    let _deny = test_deny(&["pong"]);
    let rt = state.rest.fg_mut();
    rt.begin_stream();
    rt.append_token(r#"Sure. <tool_call>{"name":"pong","arguments":{}}</tool_call>"#);
    rt.waiting = true;
    super::advance_turn(&mut state, 0, &None, runtime.handle());
    // The span became a pending call (the harvest ran) and the loop answered it.
    assert_eq!(tool_messages(&state), vec![refusal("pong")]);
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_harvested_call_runs_when_nothing_is_denied() {
    let (mut state, dir, runtime) = fixture();
    let rt = state.rest.fg_mut();
    rt.begin_stream();
    rt.append_token(r#"<tool_call>{"name":"pong","arguments":{}}</tool_call>"#);
    rt.waiting = true;
    super::advance_turn(&mut state, 0, &None, runtime.handle());
    let results = tool_messages(&state);
    assert_eq!(results.len(), 1);
    assert!(results[0].starts_with("pong!"));
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}
