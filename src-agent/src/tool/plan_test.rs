#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn parse_plan_ready_accepts_highlights_and_plan() {
    let args = json!({ "highlights": "  do the thing  ", "plan": " step 1\nstep 2 " });
    let (highlights, plan) = parse_plan_ready_args(&args).unwrap();
    assert_eq!(highlights, "do the thing");
    assert_eq!(plan, "step 1\nstep 2");
}

#[test]
fn parse_plan_ready_rejects_missing_highlights() {
    let args = json!({ "plan": "step 1" });
    let err = parse_plan_ready_args(&args).unwrap_err();
    assert!(err.starts_with("error:"), "got: {err}");
    assert!(err.contains("highlights"));
}

#[test]
fn parse_plan_ready_rejects_blank_plan() {
    let args = json!({ "highlights": "s", "plan": "   " });
    let err = parse_plan_ready_args(&args).unwrap_err();
    assert!(err.starts_with("error:"), "got: {err}");
    assert!(err.contains("plan"));
}

#[test]
fn parse_plan_ready_rejects_non_string() {
    let args = json!({ "highlights": 42, "plan": "step" });
    assert!(parse_plan_ready_args(&args).is_err());
}

#[test]
fn approved_text_embeds_reviewed_plan_body() {
    let t = plan_approved_text_with_body("Reviewed change: keep the existing API");
    assert!(t.contains("Reviewed change: keep the existing API"));
    assert!(t.contains("approved"));
}

#[test]
fn decision_texts_are_distinct() {
    assert_ne!(plan_approved_compact_text(), plan_denied_text());
    assert!(plan_approved_compact_text().contains("compact"));
    assert!(plan_denied_text().contains("plan mode"));
}

#[test]
fn plan_path_is_session_dir_plus_plan_md() {
    use crate::model::conversation::Conversation;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    let sess = Session::new(
        "sid".to_string(),
        std::path::PathBuf::from("/tmp/koma-sessions/sid"),
        "pwd".to_string(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    );
    assert_eq!(
        sess.plan_path(),
        std::path::PathBuf::from("/tmp/koma-sessions/sid/plan.md")
    );
}

#[test]
fn always_on_prompt_does_not_order_plan_enter_or_mission_clear() {
    let tools = include_str!("../../../src-misc/system-tools.txt");
    let prompt = include_str!("../../../src-misc/system-prompt.txt");
    for text in [tools, prompt] {
        assert!(
            !text.contains("plan_enter"),
            "plan_enter standing order leaked into the always-on prompt"
        );
        assert!(
            !text.contains("mission_clear"),
            "mission_clear standing order leaked into the always-on prompt"
        );
    }
    assert!(
        prompt.contains("call plan_ready and wait for the runtime approval step"),
        "Plan-mode plan_ready sentence missing from system-prompt.txt"
    );
}
