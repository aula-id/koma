#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Server-side UI guarantee: generic ApproveTool / DenyTool must not answer a
//! parked `plan_ready` or `mission_ready` — those require PlanDecision (y/a/n).

use super::*;
use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::dto::chat::{FunctionCall, ToolCall};

fn park_ready(state: &mut AppState, name: &str) {
    let s = state.rest.fg_mut();
    s.waiting = true;
    s.awaiting_approval = true;
    s.approval_reason = None;
    s.pending_tool_calls = vec![ToolCall {
        id: format!("call-{name}"),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: "{}".into(),
        },
    }];
    s.tool_idx = 0;
    s.tool_results.clear();
}

fn assert_park_intact(state: &AppState, name: &str) {
    assert!(
        state.rest.fg().awaiting_approval,
        "park must remain awaiting approval"
    );
    assert_eq!(state.rest.fg().tool_idx, 0);
    assert!(state.rest.fg().tool_results.is_empty());
    assert_eq!(
        state
            .rest
            .fg()
            .pending_tool_calls
            .first()
            .map(|c| c.function.name.as_str()),
        Some(name)
    );
}

#[test]
fn approve_tool_rejects_parked_mission_ready() {
    let mut state = AppState::new(Mode::Chat);
    park_ready(&mut state, "mission_ready");
    let mut client = None;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = handle_approve_tool(&mut state, &mut client, rt.handle())
        .expect_err("ApproveTool must reject mission_ready");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("PlanDecision") || msg.contains("mission_ready"),
        "error should name PlanDecision path: {msg}"
    );
    assert_park_intact(&state, "mission_ready");
}

#[test]
fn approve_tool_rejects_parked_plan_ready() {
    let mut state = AppState::new(Mode::Chat);
    park_ready(&mut state, "plan_ready");
    let mut client = None;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = handle_approve_tool(&mut state, &mut client, rt.handle())
        .expect_err("ApproveTool must reject plan_ready");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("PlanDecision") || msg.contains("plan_ready"),
        "error should name PlanDecision path: {msg}"
    );
    assert_park_intact(&state, "plan_ready");
}

#[test]
fn deny_tool_rejects_parked_mission_ready() {
    let mut state = AppState::new(Mode::Chat);
    park_ready(&mut state, "mission_ready");
    let err = handle_deny_tool(&mut state).expect_err("DenyTool must reject mission_ready");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("PlanDecision") || msg.contains("mission_ready"),
        "error should name PlanDecision path: {msg}"
    );
    assert_park_intact(&state, "mission_ready");
}

#[test]
fn approval_after_cancellation_cannot_execute_pending_call() {
    let mut state = AppState::new(Mode::Chat);
    park_ready(&mut state, "write");
    state.rest.fg_mut().awaiting_approval = false;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(handle_approve_tool(&mut state, &mut None, runtime.handle()).is_err());
    assert_eq!(state.rest.fg().tool_idx, 0);
    assert!(state.rest.fg().tool_results.is_empty());
}

#[test]
fn outage_approval_does_not_override_plan_mode() {
    let mut state = AppState::new(Mode::Chat);
    park_ready(&mut state, "write");
    state.rest.fg_mut().agent_mode = crate::app::state::AgentMode::Plan;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = handle_approve_tool(&mut state, &mut None, runtime.handle()).unwrap_err();
    assert!(error.to_string().contains("read-only"));
    assert_park_intact(&state, "write");
}
