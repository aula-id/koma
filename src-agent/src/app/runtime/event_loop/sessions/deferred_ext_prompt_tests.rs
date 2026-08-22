#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{ext_prompt_body, ext_prompts_ready, EXT_TURN_BUDGET};

/// The injected body leads with EXT_PROMPT_MARK and lists each buffered prompt as
/// its own `[ext:<id>] <text>` line, then the trailing instruction.
#[test]
fn body_leads_with_mark_and_lists_each_prompt() {
    let prompts = vec![
        ("alpha.ext".to_string(), "do X".to_string()),
        ("beta.ext".to_string(), "do Y".to_string()),
    ];
    let body = ext_prompt_body(&prompts);
    assert!(
        body.starts_with(crate::dto::chat::EXT_PROMPT_MARK),
        "the body must lead with EXT_PROMPT_MARK so it renders compactly + strips on the wire"
    );
    // Strip the mark → the model-visible body: joined lines + trailer.
    let visible = body
        .strip_prefix(crate::dto::chat::EXT_PROMPT_MARK)
        .unwrap();
    assert_eq!(
        visible,
        "[ext:alpha.ext] do X\n[ext:beta.ext] do Y\nThese prompts were injected by extensions; act on them as user requests."
    );
}

/// A single buffered prompt still gets the mark + its line + the trailer.
#[test]
fn single_prompt_body_shape() {
    let body = ext_prompt_body(&[("x.ext".to_string(), "hello".to_string())]);
    let visible = body
        .strip_prefix(crate::dto::chat::EXT_PROMPT_MARK)
        .unwrap();
    assert_eq!(
        visible,
        "[ext:x.ext] hello\nThese prompts were injected by extensions; act on them as user requests."
    );
}

/// The gate is IDLE-ONLY and needs BOTH a client and a live session; buffered
/// entries SURVIVE a working state (untouched until idle). `injected_turns` below
/// budget in every case here.
#[test]
fn gate_is_idle_only_and_needs_client_and_session() {
    // All preconditions met → ready to inject.
    assert!(ext_prompts_ready(true, false, true, true, 0));
    // WORKING → never ready: buffered entries survive a working state untouched.
    assert!(!ext_prompts_ready(true, true, true, true, 0));
    // No buffered prompts → nothing to inject.
    assert!(!ext_prompts_ready(false, false, true, true, 0));
    // No client / no session → can't run the turn.
    assert!(!ext_prompts_ready(true, false, false, true, 0));
    assert!(!ext_prompts_ready(true, false, true, false, 0));
}

/// Cost-DoS guard (review finding): the gate additionally requires
/// `injected_turns < EXT_TURN_BUDGET`. One below budget is still ready; AT or
/// OVER budget is never ready, even with every other precondition satisfied.
#[test]
fn gate_is_not_ready_once_turn_budget_exhausted() {
    // One below budget → still ready.
    assert!(ext_prompts_ready(
        true,
        false,
        true,
        true,
        EXT_TURN_BUDGET - 1
    ));
    // Exactly at budget → not ready (this is the park point the toast fires on).
    assert!(!ext_prompts_ready(true, false, true, true, EXT_TURN_BUDGET));
    // Past budget (the toast block's post-nudge value) → still not ready.
    assert!(!ext_prompts_ready(
        true,
        false,
        true,
        true,
        EXT_TURN_BUDGET + 1
    ));
}
