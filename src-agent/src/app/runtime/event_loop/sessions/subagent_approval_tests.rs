//! A sub-agent's risky call parked on the parent session
//! (`subagent_risky = parent`): how it reaches clients, who can answer it, and
//! what happens when nobody does.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::subagents::drain_subagents;
use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::app::subagent::{AgentEvent, ApprovalDecision, SubAgent, SubAgentApproval, SubAgentStatus};
use crate::dto::chat::{FunctionCall, ToolCall};
use crate::ipc::proto::turn::ApprovalPolicy;

struct Fixture {
    events: UnboundedSender<AgentEvent>,
    decisions: UnboundedReceiver<ApprovalDecision>,
}

fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
        },
    }
}

/// Add a running sub-agent with `id` to session 0, optionally already parked.
fn add_subagent(state: &mut AppState, id: usize, parked: Option<&str>) -> Fixture {
    let abort = tokio::spawn(std::future::pending::<()>()).abort_handle();
    let (events, rx) = mpsc::unbounded_channel();
    let (inject_tx, _inject_rx) = mpsc::unbounded_channel();
    let (approval_tx, decisions) = mpsc::unbounded_channel();
    state.rest.sessions[0].subagents.push(SubAgent {
        id,
        agent_name: "general".into(),
        label: "task".into(),
        model_id: String::new(),
        status: SubAgentStatus::Running,
        abort,
        rx,
        inject_tx,
        approval_tx,
        pending_approval: parked.map(|call_id| SubAgentApproval {
            call: tool_call(call_id, "write"),
        }),
        transcript: Vec::new(),
        messages: Vec::new(),
        live_text: String::new(),
        tool_call_id: None,
        detached: true,
        nudged: true,
        ext_owned: false,
        usage_tokens_in: 0,
        usage_tokens_out: 0,
        usage_cost: 0.0,
        sdlc_active_node_id: None,
    });
    Fixture { events, decisions }
}

fn drain(state: &mut AppState) {
    let handle = tokio::runtime::Handle::current();
    drain_subagents(state, 0, &None, &handle);
}

fn approve(state: &mut AppState) -> anyhow::Result<()> {
    let handle = tokio::runtime::Handle::current();
    crate::app::runtime::actions::apply_action(
        crate::controller::input::Action::ApproveTool,
        state,
        &mut None,
        &handle,
    )
}

fn deny(state: &mut AppState) -> anyhow::Result<()> {
    let handle = tokio::runtime::Handle::current();
    crate::app::runtime::actions::apply_action(
        crate::controller::input::Action::DenyTool,
        state,
        &mut None,
        &handle,
    )
}

#[tokio::test]
async fn a_request_from_the_sub_agent_parks_on_the_parent() {
    let mut state = AppState::new(Mode::Chat);
    let fixture = add_subagent(&mut state, 3, None);
    fixture
        .events
        .send(AgentEvent::ApprovalRequest {
            call_id: "w1".into(),
            name: "write".into(),
            args: r#"{"path":"a.txt"}"#.into(),
        })
        .unwrap();
    drain(&mut state);
    let rt = &state.rest.sessions[0];
    assert!(!rt.awaiting_approval, "the real flag is the session's own");
    assert!(rt.approval_parked());
    let (awaiting, reason, calls, idx) = rt.approval_view();
    assert!(awaiting);
    assert_eq!(idx, 0);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "subagent-3:w1");
    assert_eq!(calls[0].function.name, "write");
    let reason = reason.unwrap();
    assert!(reason.contains("sub-agent #3 (general)") && reason.contains("write"), "{reason}");
}

#[tokio::test]
async fn the_parked_call_reaches_the_client_snapshot() {
    let mut state = AppState::new(Mode::Chat);
    let _fixture = add_subagent(&mut state, 3, Some("w1"));
    let snapshot = crate::ipc::snapshot::projection::build_snapshot(&state);
    let session = &snapshot.sessions[0];
    assert!(session.awaiting_approval);
    assert_eq!(session.pending_tool_calls[session.tool_idx].id, "subagent-3:w1");
    assert!(session.approval_reason.as_deref().unwrap().contains("sub-agent #3"));
}

#[tokio::test]
async fn approve_runs_the_call_and_deny_refuses_only_that_call() {
    for (approve_it, expect) in [(true, "approve"), (false, "deny")] {
        let mut state = AppState::new(Mode::Chat);
        let mut fixture = add_subagent(&mut state, 3, Some("w1"));
        // Mid-turn work of the parent itself must survive the answer.
        state.rest.sessions[0].waiting = true;
        if approve_it {
            approve(&mut state).unwrap();
        } else {
            deny(&mut state).unwrap();
        }
        let decision = fixture.decisions.try_recv().unwrap();
        assert_eq!(decision.call_id(), "w1", "the sub-agent's own call id");
        match (expect, decision) {
            ("approve", ApprovalDecision::Approve { .. }) => {}
            ("deny", ApprovalDecision::Deny { result, .. }) => assert_eq!(result, "denied by user"),
            (_, other) => panic!("{expect}: {other:?}"),
        }
        let rt = &state.rest.sessions[0];
        assert!(!rt.approval_parked(), "answered");
        assert!(rt.waiting, "the parent's own turn is untouched");
        assert!(matches!(rt.subagents[0].status, SubAgentStatus::Running));
    }
}

#[tokio::test]
async fn parked_calls_are_answered_one_at_a_time_lowest_id_first() {
    let mut state = AppState::new(Mode::Chat);
    let mut high = add_subagent(&mut state, 7, Some("h1"));
    let mut low = add_subagent(&mut state, 2, Some("l1"));
    assert_eq!(state.rest.sessions[0].approval_call().unwrap().id, "subagent-2:l1");
    approve(&mut state).unwrap();
    assert!(low.decisions.try_recv().is_ok());
    assert!(high.decisions.try_recv().is_err(), "the other still waits");
    assert_eq!(state.rest.sessions[0].approval_call().unwrap().id, "subagent-7:h1");
    deny(&mut state).unwrap();
    assert!(matches!(high.decisions.try_recv(), Ok(ApprovalDecision::Deny { .. })));
}

#[tokio::test]
async fn the_sessions_own_approval_is_answered_before_a_sub_agents() {
    let mut state = AppState::new(Mode::Chat);
    let mut fixture = add_subagent(&mut state, 3, Some("w1"));
    {
        let rt = &mut state.rest.sessions[0];
        rt.awaiting_approval = true;
        rt.pending_tool_calls = vec![tool_call("own", "bash")];
        rt.tool_idx = 0;
    }
    let (_, _, calls, _) = state.rest.sessions[0].approval_view();
    assert_eq!(calls[0].id, "own", "the parent's own call is the one shown");
    assert_eq!(state.rest.sessions[0].approval_call().unwrap().id, "own");
    assert!(fixture.decisions.try_recv().is_err());
    // A sub-agent id is never accepted for the session's own call.
    assert!(!state.rest.sessions[0].answer_subagent_approval(true, ""));
    assert!(fixture.decisions.try_recv().is_err());
}

#[tokio::test]
async fn a_finished_sub_agent_leaves_nothing_to_answer() {
    let mut state = AppState::new(Mode::Chat);
    let _fixture = add_subagent(&mut state, 3, Some("w1"));
    state.rest.sessions[0].subagents[0].status = SubAgentStatus::Killed;
    assert!(!state.rest.sessions[0].approval_parked());
    assert!(approve(&mut state).is_err(), "nothing awaits approval");
}

#[tokio::test]
async fn an_unattended_session_rejects_the_request_without_parking() {
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].policy.approval = ApprovalPolicy::Reject;
    let mut fixture = add_subagent(&mut state, 3, None);
    fixture
        .events
        .send(AgentEvent::ApprovalRequest {
            call_id: "w1".into(),
            name: "write".into(),
            args: "{}".into(),
        })
        .unwrap();
    drain(&mut state);
    assert!(!state.rest.sessions[0].approval_parked());
    match fixture.decisions.try_recv().unwrap() {
        ApprovalDecision::Deny { call_id, result } => {
            assert_eq!(call_id, "w1");
            let v: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(v["error"], "approval_rejected");
            assert_eq!(v["executed"], false);
        }
        other => panic!("{other:?}"),
    }
}
