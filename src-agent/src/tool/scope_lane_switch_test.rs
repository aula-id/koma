use super::*;
use crate::app::sec::active::Inactive;
use crate::app::sec::lane::Lane;

fn ctx(session: &str, lane: Option<Lane>, block: Option<Inactive>) -> ToolCtx {
    crate::tool::ToolCtx {
        session_id: session.into(),
        cyber_lane: lane,
        sec_block: block.map(|b| b.reason().to_string()),
        sec_inactive: Vec::new(),
        ..crate::tool::test_ctx()
    }
}

#[test]
fn discovery_and_lane_choice_work_before_a_lane_is_set() {
    let c = ctx("gate-no-lane", None, Some(Inactive::NoLane));
    let mut args = serde_json::json!({});
    assert_eq!(lane_refusal(&c, "sec_catalog", &mut args), None);
    assert_eq!(lane_refusal(&c, "sec_lane", &mut args), None);
    assert!(sec_enabled(&c, "sec_catalog"));
    // Everything else still needs a lane.
    assert!(lane_refusal(&c, "sec_http", &mut args).is_some());
    assert!(lane_refusal(&c, "sec_run", &mut args).is_some());
    assert!(!sec_enabled(&c, "sec_http"));
}

#[test]
fn a_model_choice_applies_for_the_rest_of_the_turn_and_keeps_the_matrix() {
    let session = "gate-override";
    let c = ctx(session, None, Some(Inactive::NoLane));
    crate::app::sec::lane_switch::request(session, Some(Lane::Research));
    assert_eq!(lane_of(&c), Some(Lane::Research));
    let mut args = serde_json::json!({});
    // Research is local only: a network tool is still refused by the lane matrix.
    assert!(lane_refusal(&c, "sec_http", &mut args).is_some());
    assert_eq!(lane_refusal(&c, "sec_decode", &mut args), None);
    // Switching to red opens the network tool.
    crate::app::sec::lane_switch::request(session, Some(Lane::Red));
    assert_eq!(lane_refusal(&c, "sec_http", &mut serde_json::json!({})), None);
    // sec_lane stays callable inside a lane, so the model can narrow it again.
    assert_eq!(lane_refusal(&c, "sec_lane", &mut serde_json::json!({})), None);
    crate::app::sec::lane_switch::request(session, None);
    assert_eq!(lane_of(&c), None);
}

#[test]
fn plan_mode_and_disabled_security_cannot_be_bypassed_by_a_choice() {
    for (session, block) in [("gate-plan", Inactive::PlanMode), ("gate-off", Inactive::Disabled)] {
        let c = ctx(session, None, Some(block));
        crate::app::sec::lane_switch::request(session, Some(Lane::Red));
        assert_eq!(lane_of(&c), None, "{block:?}");
        assert!(lane_refusal(&c, "sec_lane", &mut serde_json::json!({})).is_some(), "{block:?}");
        assert!(lane_refusal(&c, "sec_http", &mut serde_json::json!({})).is_some(), "{block:?}");
    }
    // Plan mode keeps read-only discovery; disabled security does not.
    assert!(sec_enabled(&ctx("gate-plan", None, Some(Inactive::PlanMode)), "sec_catalog"));
    assert!(!sec_enabled(&ctx("gate-off", None, Some(Inactive::Disabled)), "sec_catalog"));
}
