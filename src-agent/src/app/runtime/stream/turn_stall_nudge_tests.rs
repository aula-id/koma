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
