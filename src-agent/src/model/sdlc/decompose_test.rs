#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::graph::{ensure_tables, replace_nodes_from_checklist, ChecklistNode};
use rusqlite::Connection;

fn mem() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    ensure_tables(&c).unwrap();
    c
}

#[test]
fn standard_rejects_single_megatask() {
    let nodes = vec![ChecklistNode {
        title: "do everything".into(),
        status: "pending".into(),
        parent_title: None,
        id: None,

        owned_paths: vec![],
    }];
    let err = validate_lane_graph("standard", &nodes, 3).unwrap_err();
    assert!(err.contains("megatask"));
}

#[test]
fn full_requires_tree_or_three_leaves() {
    let one = vec![ChecklistNode {
        title: "only".into(),
        status: "pending".into(),
        parent_title: None,
        id: None,

        owned_paths: vec![],
    }];
    assert!(validate_lane_graph("full", &one, 1).is_err());

    let three: Vec<_> = (0..3)
        .map(|i| ChecklistNode {
            title: format!("t{i}"),
            status: "pending".into(),
            parent_title: None,
            id: None,

            owned_paths: vec![],
        })
        .collect();
    assert!(validate_lane_graph("full", &three, 1).is_ok());

    let tree = vec![
        ChecklistNode {
            title: "epic".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,

            owned_paths: vec![],
        },
        ChecklistNode {
            title: "leaf".into(),
            status: "pending".into(),
            parent_title: Some("epic".into()),
            id: None,

            owned_paths: vec![],
        },
    ];
    assert!(validate_lane_graph("full", &tree, 1).is_ok());
}

#[test]
fn delegation_requires_open_leaf() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "parent".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "child".into(),
                status: "pending".into(),
                parent_title: Some("parent".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let all = graph::list_all(&conn).unwrap();
    let parent = all.iter().find(|n| n.title == "parent").unwrap();
    let child = all.iter().find(|n| n.title == "child").unwrap();

    assert!(validate_task_delegation(&conn, None, "x").is_err());
    assert!(validate_task_delegation(&conn, Some(&parent.id), "x").is_err());
    let claim = validate_task_delegation(&conn, Some(&child.id), "do the child").unwrap();
    assert_eq!(claim.title, "child");
    assert_eq!(
        graph::get_node(&conn, &child.id).unwrap().unwrap().status,
        "active"
    );
    // Second delegation on the already-active leaf must fail exclusive claim.
    let err2 = validate_task_delegation(&conn, Some(&child.id), "continue child").unwrap_err();
    assert!(
        err2.contains("not claimable") || err2.contains("could not claim"),
        "unexpected second-delegation err: {err2}"
    );
    // Direct second claim_leaf must also fail exclusive.
    assert!(graph::claim_leaf(&conn, &child.id).is_err());
}

#[test]
fn prompt_too_long_rejected() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "leaf".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        }],
    )
    .unwrap();
    let id = graph::list_all(&conn).unwrap()[0].id.clone();
    let big = "x".repeat(TASK_PROMPT_HARD_MAX + 1);
    let err = validate_task_delegation(&conn, Some(&id), &big).unwrap_err();
    assert!(err.contains("exceeds"));
}

#[test]
fn scope_banner_includes_owned_paths() {
    let claim = LeafClaim {
        node_id: "n-test-000".into(),
        title: "test task".into(),
        owned_paths: vec!["src/foo.rs".into(), "src/bar/**".into()],
    };
    let banner = scope_banner(&claim);
    assert!(banner.contains("node_id: n-test-000"));
    assert!(banner.contains("title: test task"));
    assert!(banner.contains("src/foo.rs"));
    assert!(banner.contains("src/bar/**"));
    assert!(banner.contains("FORBIDDEN"));
}

#[test]
fn scope_banner_includes_handoff_instructions() {
    let claim = LeafClaim {
        node_id: "n-test-h0".into(),
        title: "handoff task".into(),
        owned_paths: vec![],
    };
    let banner = scope_banner(&claim);
    assert!(
        banner.contains("SDLC_HANDOFF_JSON_START"),
        "banner must include handoff start marker"
    );
    assert!(
        banner.contains("SDLC_HANDOFF_JSON_END"),
        "banner must include handoff end marker"
    );
    assert!(
        banner.contains("done|partial|blocked"),
        "banner must describe status values"
    );
    assert!(
        banner.contains("mission_verify"),
        "banner must warn that done does not seal"
    );
}

#[test]
fn scope_banner_omits_ownership_when_empty() {
    let claim = LeafClaim {
        node_id: "n-test-001".into(),
        title: "no ownership".into(),
        owned_paths: vec![],
    };
    let banner = scope_banner(&claim);
    assert!(banner.contains("node_id: n-test-001"));
    assert!(!banner.contains("owned_paths"));
    assert!(!banner.contains("FORBIDDEN"));
}
