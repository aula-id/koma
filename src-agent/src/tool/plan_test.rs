use super::*;

#[test]
fn parse_plan_ready_accepts_summary_and_plan() {
    let args = json!({ "summary": "  do the thing  ", "plan": " step 1\nstep 2 " });
    let (summary, plan) = parse_plan_ready_args(&args).unwrap();
    assert_eq!(summary, "do the thing");
    assert_eq!(plan, "step 1\nstep 2");
}

#[test]
fn parse_plan_ready_rejects_missing_summary() {
    let args = json!({ "plan": "step 1" });
    let err = parse_plan_ready_args(&args).unwrap_err();
    assert!(err.starts_with("error:"), "got: {err}");
    assert!(err.contains("summary"));
}

#[test]
fn parse_plan_ready_rejects_blank_plan() {
    let args = json!({ "summary": "s", "plan": "   " });
    let err = parse_plan_ready_args(&args).unwrap_err();
    assert!(err.starts_with("error:"), "got: {err}");
    assert!(err.contains("plan"));
}

#[test]
fn parse_plan_ready_rejects_non_string() {
    let args = json!({ "summary": 42, "plan": "step" });
    assert!(parse_plan_ready_args(&args).is_err());
}

#[test]
fn approved_text_embeds_plan_path() {
    let t = plan_approved_text("/tmp/sess/plan.md");
    assert!(t.contains("/tmp/sess/plan.md"));
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
