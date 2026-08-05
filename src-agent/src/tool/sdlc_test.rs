#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn parse_mission_ready_accepts_required_fields() {
    let args = json!({
        "highlights": "  ship feature X  ",
        "goal": " add feature X ",
        "acceptance": ["tests green", "docs updated"],
        "graph_tasks": ["implement", "verify"],
        "lane": "express",
        "non_goals": ["rewrite Y"],
    });
    let m = parse_mission_ready_args(&args).unwrap();
    assert_eq!(m.highlights, "ship feature X");
    assert_eq!(m.goal, "add feature X");
    assert_eq!(m.acceptance, vec!["tests green", "docs updated"]);
    assert_eq!(m.graph_tasks, vec!["implement", "verify"]);
    assert_eq!(m.lane, "express");
    assert_eq!(m.non_goals, vec!["rewrite Y"]);
}

#[test]
fn parse_mission_ready_defaults_lane_and_optional_arrays() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a"],
        "graph_tasks": ["t1"],
    });
    let m = parse_mission_ready_args(&args).unwrap();
    assert_eq!(m.lane, "standard");
    assert!(m.non_goals.is_empty());
    assert!(m.verify_plan.is_empty());
    assert!(m.human_gates.is_empty());
    assert!(m.risks.is_empty());
    assert!(m.rationale.is_empty());
}

#[test]
fn parse_mission_ready_rejects_missing_goal() {
    let args = json!({
        "highlights": "h",
        "acceptance": ["a"],
        "graph_tasks": ["t"],
    });
    let err = parse_mission_ready_args(&args).unwrap_err();
    assert!(err.starts_with("error:"), "got: {err}");
    assert!(err.contains("goal"));
}

#[test]
fn parse_mission_ready_rejects_empty_acceptance() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": [],
        "graph_tasks": ["t"],
    });
    let err = parse_mission_ready_args(&args).unwrap_err();
    assert!(err.contains("acceptance"));
}

#[test]
fn decision_texts_are_distinct() {
    assert_ne!(mission_approved_compact_text(), mission_denied_text());
    assert!(mission_approved_compact_text().contains("compact"));
    assert!(mission_denied_text().contains("SDLC"));
    let body = mission_approved_text("{\"goal\":\"x\"}");
    assert!(body.contains("APPROVED MISSION"));
    assert!(body.contains("{\"goal\":\"x\"}"));
}

#[test]
fn parse_mission_verify_defaults_pass_true() {
    let args = json!({
        "node_id": "t1",
        "evidence": "cargo test ok",
    });
    let (id, evidence, pass) = parse_mission_verify_args(&args).unwrap();
    assert_eq!(id.as_deref(), Some("t1"));
    assert_eq!(evidence, "cargo test ok");
    assert!(pass);
}

#[test]
fn parse_mission_verify_rejects_empty_evidence() {
    let args = json!({ "node_id": "t1", "evidence": "  " });
    let err = parse_mission_verify_args(&args).unwrap_err();
    assert!(err.contains("evidence"));
}

#[test]
fn parse_mission_integrate_force_flag() {
    let args = json!({ "summary": "shipped", "force_branch_only": true });
    let (summary, force) = parse_mission_integrate_args(&args).unwrap();
    assert_eq!(summary, "shipped");
    assert!(force);
}
