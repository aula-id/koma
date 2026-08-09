#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::graph::ChecklistNode;

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
    assert_eq!(m.graph_tasks.len(), 2);
    assert_eq!(m.graph_tasks[0].title, "implement");
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
fn parse_mission_ready_accepts_parent_objects() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a"],
        "lane": "full",
        "graph_tasks": [
            "epic",
            {"title": "leaf", "parent": "epic"}
        ],
    });
    let m = parse_mission_ready_args(&args).unwrap();
    assert_eq!(m.graph_tasks.len(), 2);
    assert_eq!(m.graph_tasks[1].parent_title.as_deref(), Some("epic"));
}

#[test]
fn parse_mission_ready_standard_rejects_megatask() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a", "b", "c"],
        "lane": "standard",
        "graph_tasks": ["do everything"],
    });
    let err = parse_mission_ready_args(&args).unwrap_err();
    assert!(err.contains("megatask"), "got {err}");
}

#[test]
fn decision_texts_are_distinct() {
    assert_ne!(mission_approved_compact_text(), mission_denied_text());
    assert!(mission_approved_compact_text().contains("compact"));
    assert!(mission_denied_text().contains("SDLC"));
    let body = mission_approved_text("{\"goal\":\"x\"}");
    assert!(body.contains("APPROVED MISSION"));
    assert!(body.contains("{\"goal\":\"x\"}"));
    assert!(mission_binding_failed_text("x").contains("NOT approved"));
}

#[test]
fn parse_mission_verify_defaults_pass_true() {
    let args = json!({
        "node_id": "t1",
        "evidence": "cargo test ok",
    });
    let (id, evidence, pass, gate) = parse_mission_verify_args(&args).unwrap();
    assert_eq!(id.as_deref(), Some("t1"));
    assert_eq!(evidence, "cargo test ok");
    assert!(pass);
    assert!(gate.is_none());
}

#[test]
fn parse_mission_verify_human_gate() {
    let args = json!({
        "evidence": "user signed off",
        "human_gate": "review API",
    });
    let (_id, _e, _p, gate) = parse_mission_verify_args(&args).unwrap();
    assert_eq!(gate.as_deref(), Some("review API"));
}

#[test]
fn mission_verify_schema_human_gate_is_request_not_self_approve() {
    let t = MissionVerify;
    let desc = t.description();
    assert!(
        desc.contains("PARKS") || desc.to_lowercase().contains("user"),
        "description must make clear human_gate is user-gated"
    );
    let params = t.parameters();
    let hg = params["properties"]["human_gate"]["description"]
        .as_str()
        .unwrap_or("");
    assert!(
        hg.to_lowercase().contains("user") || hg.contains("y/n"),
        "schema must not claim model self-approves: {hg}"
    );
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

#[test]
fn checklist_node_roundtrip_shape() {
    let n = ChecklistNode {
        title: "t".into(),
        status: "pending".into(),
        parent_title: Some("p".into()),
        id: None,

        owned_paths: vec![],
    };
    assert_eq!(n.parent_title.as_deref(), Some("p"));
}


#[test]
fn parse_mission_prepare_empty_args_returns_none() {
    let args = json!({});
    let note = parse_mission_prepare_args(&args).unwrap();
    assert!(note.is_none());
}

#[test]
fn parse_mission_prepare_with_note() {
    let args = json!({ "note": "  worktrees ready  " });
    let note = parse_mission_prepare_args(&args).unwrap();
    assert_eq!(note.as_deref(), Some("worktrees ready"));
}

#[test]
fn parse_mission_prepare_blank_note_returns_none() {
    let args = json!({ "note": "   " });
    let note = parse_mission_prepare_args(&args).unwrap();
    assert!(note.is_none());
}

#[test]
fn mission_prepare_result_text() {
    assert!(mission_prepare_result("").contains("transitioning to execute"));
    assert!(mission_prepare_result("done").contains("(done)"));
}

#[test]
fn parse_mission_ready_with_target_branch() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a"],
        "graph_tasks": ["t1"],
        "target_branch": "  develop  ",
    });
    let m = parse_mission_ready_args(&args).unwrap();
    assert_eq!(m.target_branch.as_deref(), Some("develop"));
}

#[test]
fn parse_mission_ready_without_target_branch() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a"],
        "graph_tasks": ["t1"],
    });
    let m = parse_mission_ready_args(&args).unwrap();
    assert!(m.target_branch.is_none());
}

#[test]
fn parse_mission_ready_rejects_bad_target_branch() {
    let args = json!({
        "highlights": "h",
        "goal": "g",
        "acceptance": ["a"],
        "graph_tasks": ["t1"],
        "target_branch": "-bad",
    });
    let err = parse_mission_ready_args(&args).unwrap_err();
    assert!(err.contains("target_branch"), "got: {err}");
}

#[test]
fn mission_ready_schema_has_target_branch() {
    let t = MissionReady;
    let params = t.parameters();
    let tb = params["properties"]["target_branch"]["description"]
        .as_str()
        .unwrap_or("");
    assert!(
        tb.contains("integration") || tb.contains("merges into"),
        "schema must describe target_branch purpose: {tb}"
    );
    assert!(
        tb.contains("main/master") || tb.contains("main"),
        "schema must mention main/master guard: {tb}"
    );
}
