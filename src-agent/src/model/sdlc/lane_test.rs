#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::graph::ChecklistNode;

fn node(title: &str, parent: Option<&str>) -> ChecklistNode {
    ChecklistNode {
        title: title.into(),
        status: "pending".into(),
        parent_title: parent.map(|s| s.into()),
        id: None,
        owned_paths: vec![],
    }
}

#[test]
fn parse_lanes() {
    assert_eq!(Lane::parse("express").unwrap(), Lane::Express);
    assert_eq!(Lane::parse("STANDARD").unwrap(), Lane::Standard);
    assert!(Lane::parse("nope").is_err());
}

#[test]
fn express_prefers_branch_only_done() {
    assert!(Lane::Express.prefer_branch_only());
    assert!(Lane::Express.branch_ready_completes_mission());
    assert!(!Lane::Standard.prefer_branch_only());
    assert!(!Lane::Full.branch_ready_completes_mission());
}

#[test]
fn express_graph_free() {
    assert!(validate_lane_graph("express", &[node("mega", None)], 5).is_ok());
}

#[test]
fn standard_rejects_megatask() {
    let err = validate_lane_graph("standard", &[node("mega", None)], 3).unwrap_err();
    assert!(err.contains("megatask"), "{err}");
}

#[test]
fn full_requires_tree_or_three_leaves() {
    assert!(validate_lane_graph("full", &[node("one", None)], 1).is_err());
    assert!(validate_lane_graph(
        "full",
        &[
            node("a", None),
            node("b", None),
            node("c", None),
        ],
        1
    )
    .is_ok());
    assert!(validate_lane_graph(
        "full",
        &[node("epic", None), node("leaf", Some("epic"))],
        1
    )
    .is_ok());
}
