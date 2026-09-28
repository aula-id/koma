use super::should_stall_nudge;

#[test]
fn stall_under_budget_nudges() {
    assert!(should_stall_nudge("Let me read more:", 0));
    assert!(should_stall_nudge("Let me read more:", 1));
}

#[test]
fn stall_at_budget_stops() {
    assert!(!should_stall_nudge("Let me read more:", 2));
}

#[test]
fn complete_answer_does_not_nudge() {
    assert!(!should_stall_nudge("Here is the answer.\nDone.", 0));
}

#[test]
fn computer_observation_promise_requires_active_recovery() {
    let mut rt = crate::app::state::SessionRuntime::new();
    let text = "Observing again after the desktop shift.";
    rt.computer.status.message = "Desktop changed during capture; observe again".into();
    assert!(!super::observation_recovery_stalled(&rt, text));
    rt.computer.status.enabled = true;
    assert!(super::observation_recovery_stalled(&rt, text));
    for final_answer in [
        "Opened the collection.",
        "I cannot continue; please take over.",
        "Should I observe again?",
        "Observing again requires your permission.",
        "Observing again confirmed that the collection is open.",
        "Observing again requires your permission.\nPlease enable sharing.",
    ] {
        assert!(!super::observation_recovery_stalled(&rt, final_answer));
    }
    rt.computer.status.paused = true;
    assert!(!super::observation_recovery_stalled(&rt, text));
    rt.computer.status.paused = false;
    rt.computer.actionable = true;
    assert!(!super::observation_recovery_stalled(&rt, text));
    rt.computer.actionable = false;
    rt.computer.status.message = "Completed 1 inputs".into();
    assert!(!super::observation_recovery_stalled(&rt, text));
}

#[test]
fn computer_observation_stall_exhaustion_is_visible() {
    let mut state = crate::app::state::AppState::new(crate::app::mode::Mode::Chat);
    let rt = state.rest.fg_mut();
    let dir = std::env::temp_dir().join(format!("koma-stall-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    rt.session = Some(crate::model::session::Session::new(
        "s".into(),
        dir.clone(),
        "test".into(),
        Default::default(),
        crate::model::conversation::Conversation::from_messages(vec![]),
    ));
    rt.computer.status.enabled = true;
    rt.computer.status.message = "Desktop changed during capture; observe again".into();
    rt.main_stall_nudges = 2;
    rt.begin_stream();
    rt.append_token("Observing again after the desktop shift.");
    rt.waiting = true;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    super::advance_turn(&mut state, 0, &None, runtime.handle());
    let rt = state.rest.fg();
    assert!(!rt.waiting);
    assert!(rt.status.contains("model did not request"));
    assert!(rt
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages()
        .iter()
        .any(|m| m.content.contains("Computer task paused:")));
    assert!(rt.computer.outbound.is_none());
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

fn desktop_turn_fixture() -> (crate::app::state::AppState, std::path::PathBuf) {
    use crate::dto::chat::{ChatMessage, FunctionCall, Role, ToolCall};
    let mut state = crate::app::state::AppState::new(crate::app::mode::Mode::Chat);
    let dir = std::env::temp_dir().join(format!("koma-desktop-turn-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let rt = state.rest.fg_mut();
    rt.session = Some(crate::model::session::Session::new(
        "s".into(),
        dir.clone(),
        "test".into(),
        Default::default(),
        crate::model::conversation::Conversation::from_messages(vec![
            ChatMessage::new(Role::System, "Use computer tools for the desktop task."),
            ChatMessage::new(Role::User, "Open Compass and inspect the collection."),
            ChatMessage::assistant_with_tools(
                "Checking whether Compass is up.".into(),
                vec![ToolCall {
                    id: "sources".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "computer_windows".into(),
                        arguments: "{}".into(),
                    },
                }],
            ),
        ]),
    ));
    rt.session
        .as_mut()
        .unwrap()
        .conversation
        .push_tool("sources".into(), "22 sources available".into());
    rt.computer.status.enabled = true;
    rt.computer.actionable = true;
    rt.computer.status.message = "Completed 1 inputs".into();
    rt.agent_steps = 1;
    (state, dir)
}

#[test]
fn computer_successful_round_still_requires_promised_tool_call() {
    let (mut state, dir) = desktop_turn_fixture();
    let rt = state.rest.fg_mut();
    for promise in [
        "Compass is focused — selecting the screen and inspecting it.",
        "Selecting the screen and inspecting it.",
        "Chrome is open. Clicking the Gmail tab now.",
        "The app is open.\n\nInspecting the screen next.",
        "I'm scrolling to the collection.",
        "Switching to Discord.",
        "Clicking `mpos_dev` in the sidebar.",
        "I’ll inspect the desktop now.",
        "", // Empty/reasoning-only completion after desktop tools.
    ] {
        assert!(super::computer_turn_stalled(rt, promise), "{promise}");
    }
    for final_answer in [
        "Compass is open and the collection is visible.",
        "Opening Compass succeeded.",
        "Should I select the screen?",
        "I need you to approve screen recording first.",
        "I'll wait for your approval.",
        "Selecting the screen would require another approval.",
        "I cannot inspect this window; please take over.",
    ] {
        assert!(
            !super::computer_turn_stalled(rt, final_answer),
            "{final_answer}"
        );
    }
    rt.computer.status.paused = true;
    assert!(!super::computer_turn_stalled(rt, "Selecting the screen."));
    rt.computer.status.paused = false;
    rt.computer.status.enabled = false;
    assert!(!super::computer_turn_stalled(rt, "Selecting the screen."));
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn computer_turn_scope_skips_observations_and_reminders_but_ends_at_new_user() {
    use crate::dto::chat::{Attachment, AttachmentKind};
    let (mut state, dir) = desktop_turn_fixture();
    let rt = state.rest.fg_mut();
    let conversation = &mut rt.session.as_mut().unwrap().conversation;
    conversation.push_user_with_attachments(
        "Computer observation [Image #1]. fixture",
        vec![Attachment {
            kind: AttachmentKind::Image,
            marker_n: 1,
            rel_path: "images/fixture.png".into(),
            mime: "image/png".into(),
        }],
    );
    conversation.push_user(super::COMPUTER_RECOVERY_NUDGE_MSG);
    assert!(super::computer_turn_stalled(rt, "Selecting the screen."));
    rt.session
        .as_mut()
        .unwrap()
        .conversation
        .push_user("Stop the desktop task. Explain Rust lifetimes instead.");
    assert!(!super::computer_turn_stalled(rt, "Checking the example."));
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn computer_success_stall_exhaustion_or_missing_client_is_visible() {
    for budget in [0, 2] {
        let (mut state, dir) = desktop_turn_fixture();
        let rt = state.rest.fg_mut();
        rt.main_stall_nudges = budget;
        rt.begin_stream();
        rt.append_token("Compass is focused — selecting the screen and inspecting it.");
        rt.waiting = true;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        super::advance_turn(&mut state, 0, &None, runtime.handle());
        let rt = state.rest.fg();
        assert!(!rt.waiting);
        assert!(rt.status.contains("model did not request"));
        let expected = if budget == 2 {
            "after two reminders"
        } else {
            "could not start the follow-up"
        };
        let messages = rt.session.as_ref().unwrap().conversation.messages();
        assert!(messages
            .iter()
            .any(|m| m.content.contains("Computer task paused:") && m.content.contains(expected)));
        assert!(rt.computer.outbound.is_none());
        assert!(rt.computer.status.enabled);
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn computer_handoff_does_not_restart_or_add_false_pause_notice() {
    let (mut state, dir) = desktop_turn_fixture();
    let rt = state.rest.fg_mut();
    rt.begin_stream();
    rt.append_token("I'll wait for your approval.");
    rt.waiting = true;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    super::advance_turn(&mut state, 0, &None, runtime.handle());
    let rt = state.rest.fg();
    assert!(!rt.waiting);
    assert_eq!(rt.main_stall_nudges, 0);
    assert!(!rt
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages()
        .iter()
        .any(|m| m.content == super::MAIN_STALL_NUDGE_MSG
            || m.content == super::COMPUTER_RECOVERY_NUDGE_MSG
            || m.content.contains("Computer task paused:")));
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn computer_success_stall_schedules_bounded_model_continuation_without_input() {
    let (mut state, dir) = desktop_turn_fixture();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = Some(std::sync::Arc::new(
        crate::service::openrouter::OpenRouterClient::new(),
    ));
    // Do not drive this runtime: verify scheduling without contacting a model
    // provider, launching apps, capturing the desktop, or injecting input.
    for expected_budget in [1, 2] {
        let rt = state.rest.fg_mut();
        rt.begin_stream();
        rt.append_token("Compass is focused — selecting the screen and inspecting it.");
        rt.waiting = true;
        super::advance_turn(&mut state, 0, &client, runtime.handle());
        let rt = state.rest.fg_mut();
        assert!(rt.waiting);
        assert_eq!(rt.main_stall_nudges, expected_budget);
        assert!(rt.active_rx.is_some());
        assert!(rt.computer.outbound.is_none());
        rt.current_task.take().unwrap().abort();
        rt.active_rx.take();
    }
    let rt = state.rest.fg_mut();
    rt.begin_stream();
    rt.append_token("Selecting the screen and inspecting it.");
    super::advance_turn(&mut state, 0, &client, runtime.handle());
    let rt = state.rest.fg();
    assert!(!rt.waiting);
    assert!(rt.active_rx.is_none());
    assert!(rt.current_task.is_none());
    assert!(rt.computer.outbound.is_none());
    let messages = rt.session.as_ref().unwrap().conversation.messages();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.content == super::COMPUTER_RECOVERY_NUDGE_MSG)
            .count(),
        2
    );
    assert!(messages
        .last()
        .unwrap()
        .content
        .contains("after two reminders"));
    drop(runtime);
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}
