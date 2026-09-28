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
