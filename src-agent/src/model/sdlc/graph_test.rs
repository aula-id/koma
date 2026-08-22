#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::model::sdlc::handoff::{
    validate_handoff, ChildProposal, HandoffStatus, HandoffUpdates, SdlcHandoff,
    ValidatedHandoff, CURRENT_HANDOFF_VERSION,
};
use rusqlite::Connection;

fn mem() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    conn
}

fn flat(items: &[(&str, &str)]) -> Vec<ChecklistNode> {
    items
        .iter()
        .map(|(t, s)| ChecklistNode {
            title: (*t).into(),
            status: (*s).into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        })
        .collect()
}

fn active_leaf(conn: &Connection, title: &str, owned_paths: Vec<String>) -> String {
    replace_nodes_from_checklist(
        conn,
        &[ChecklistNode {
            title: title.into(),
            status: "active".into(),
            parent_title: None,
            id: None,
            owned_paths,
        }],
    )
    .unwrap();
    list_all(conn).unwrap()[0].id.clone()
}

fn validated_handoff(node_id: &str, status: HandoffStatus) -> ValidatedHandoff {
    validate_handoff(&SdlcHandoff {
        version: CURRENT_HANDOFF_VERSION,
        node_id: node_id.into(),
        status,
        summary: "handoff summary".into(),
        artifacts: vec![],
        evidence_refs: vec![],
        commit_shas: vec![],
        decisions: vec![],
        updates: HandoffUpdates::default(),
    })
    .unwrap()
}

#[test]
fn verify_bit_and_false_done_reopen() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending"), ("b", "pending")])).unwrap();
    let a_id = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|n| n.title == "a")
        .unwrap()
        .id;
    // Legacy/raw false-done row (checklist path can no longer create bare done).
    conn.execute(
        "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
        rusqlite::params![a_id],
    )
    .unwrap();
    let false_done = list_false_done(&conn).unwrap();
    assert_eq!(false_done.len(), 1);
    assert_eq!(false_done[0].title, "a");

    set_verify_bit_with_evidence(&conn, &false_done[0].id, true, None).unwrap();
    assert!(list_false_done(&conn).unwrap().is_empty());

    set_verify_bit_with_evidence(&conn, &false_done[0].id, false, None).unwrap();
    let open = list_open(&conn).unwrap();
    assert!(open
        .iter()
        .any(|n| n.id == false_done[0].id && n.status == "active"));
}

#[test]
fn stable_id_and_verify_preserve() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("x", "pending"), ("y", "pending")])).unwrap();
    let x_id_before = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|n| n.title == "x")
        .unwrap()
        .id;
    set_verify_bit_with_evidence(&conn, &x_id_before, true, None).unwrap();

    replace_nodes_from_checklist(&conn, &flat(&[("x", "done"), ("y", "pending")])).unwrap();
    let sealed = list_sealed(&conn).unwrap();
    let x_node2 = sealed.iter().find(|n| n.title == "x").unwrap();
    assert_eq!(x_node2.id, x_id_before);
    assert!(x_node2.verify_bit);

    replace_nodes_from_checklist(&conn, &flat(&[("x", "pending"), ("y", "pending")])).unwrap();
    let open = list_open(&conn).unwrap();
    let x_open = open.iter().find(|n| n.title == "x").unwrap();
    assert_eq!(x_open.id, x_id_before);
    assert!(!x_open.verify_bit);
}

#[test]
fn hierarchy_parent_rollup_and_leaf_only_verify() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "epic".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "leaf-a".into(),
                status: "pending".into(),
                parent_title: Some("epic".into()),
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "leaf-b".into(),
                status: "pending".into(),
                parent_title: Some("epic".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let all = list_all(&conn).unwrap();
    let epic = all.iter().find(|n| n.title == "epic").unwrap();
    let a = all.iter().find(|n| n.title == "leaf-a").unwrap();
    let b = all.iter().find(|n| n.title == "leaf-b").unwrap();

    assert!(set_verify_bit_with_evidence(&conn, &epic.id, true, None).is_err());

    set_verify_bit_with_evidence(&conn, &a.id, true, None).unwrap();
    set_verify_bit_with_evidence(&conn, &b.id, true, None).unwrap();
    let epic2 = get_node(&conn, &epic.id).unwrap().unwrap();
    assert_eq!(epic2.status, "done");
    assert!(epic2.verify_bit);
    // Parent rollup event must exist alongside verified children.
    let rollup_n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'rollup' AND detail = 'all_children_verified'",
            [&epic.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(rollup_n >= 1, "expected atomic parent rollup event");

    // Invalidate child → ancestors reopen.
    set_verify_bit_with_evidence(&conn, &a.id, false, None).unwrap();
    let epic3 = get_node(&conn, &epic.id).unwrap().unwrap();
    assert_eq!(epic3.status, "active");
    assert!(!epic3.verify_bit);
}

#[test]
fn verify_and_parent_rollup_are_one_transaction() {
    // Behavioral atomicity: after successful verify of the last open child,
    // parent is done+verified AND a rollup event exists — there is no window
    // where the child is done without parent rollup (same commit).
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "p".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "c".into(),
                status: "pending".into(),
                parent_title: Some("p".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let all = list_all(&conn).unwrap();
    let p = all.iter().find(|n| n.title == "p").unwrap().id.clone();
    let c = all.iter().find(|n| n.title == "c").unwrap().id.clone();
    set_verify_bit_with_evidence(&conn, &c, true, Some("tests green")).unwrap();
    let parent = get_node(&conn, &p).unwrap().unwrap();
    let child = get_node(&conn, &c).unwrap().unwrap();
    assert_eq!(child.status, "done");
    assert!(child.verify_bit);
    assert_eq!(parent.status, "done");
    assert!(parent.verify_bit);
    let ev: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'verify_evidence'",
            [&c],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ev, 1);
    let roll: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'rollup'",
            [&p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(roll, 1);
}

#[test]
fn update_status_rollup_reopens_done_parent() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "p".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "c".into(),
                status: "pending".into(),
                parent_title: Some("p".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let all = list_all(&conn).unwrap();
    let p = all.iter().find(|n| n.title == "p").unwrap().id.clone();
    let c = all.iter().find(|n| n.title == "c").unwrap().id.clone();
    set_verify_bit_with_evidence(&conn, &c, true, None).unwrap();
    assert_eq!(get_node(&conn, &p).unwrap().unwrap().status, "done");
    // Reopen child via status update — parent must roll open in same path.
    update_node_status(&conn, &c, "active").unwrap();
    let parent = get_node(&conn, &p).unwrap().unwrap();
    assert_eq!(parent.status, "active");
    assert!(!parent.verify_bit);
}

#[test]
fn rejects_cycle_and_deep_tree() {
    let err = validate_checklist_nodes(&[
        ChecklistNode {
            title: "a".into(),
            status: "pending".into(),
            parent_title: Some("b".into()),
            id: None,

            owned_paths: vec![],
        },
        ChecklistNode {
            title: "b".into(),
            status: "pending".into(),
            parent_title: Some("a".into()),
            id: None,

            owned_paths: vec![],
        },
    ])
    .unwrap_err();
    assert!(err.to_string().contains("cycle"));

    let err = validate_checklist_nodes(&[
        ChecklistNode {
            title: "e".into(),
            status: "pending".into(),
            parent_title: None,
            id: None,

            owned_paths: vec![],
        },
        ChecklistNode {
            title: "s".into(),
            status: "pending".into(),
            parent_title: Some("e".into()),
            id: None,

            owned_paths: vec![],
        },
        ChecklistNode {
            title: "t".into(),
            status: "pending".into(),
            parent_title: Some("s".into()),
            id: None,

            owned_paths: vec![],
        },
        ChecklistNode {
            title: "x".into(),
            status: "pending".into(),
            parent_title: Some("t".into()),
            id: None,

            owned_paths: vec![],
        },
    ])
    .unwrap_err();
    assert!(err.to_string().contains("deeper"));
}

#[test]
fn cancellation_is_not_verification() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "cancelled"), ("b", "pending")]))
        .unwrap();
    let b_id = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|n| n.title == "b")
        .unwrap()
        .id;
    conn.execute(
        "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
        rusqlite::params![b_id],
    )
    .unwrap();
    // only b is false-done leaf
    let fd = list_false_done(&conn).unwrap();
    assert_eq!(fd.len(), 1);
    assert_eq!(fd[0].title, "b");
    assert!(!all_required_leaves_verified(&conn).unwrap());
}

#[test]
fn mission_meta_roundtrip() {
    let conn = mem();
    set_mission_meta(&conn, "k", "v").unwrap();
    assert_eq!(get_mission_meta(&conn, "k").unwrap().as_deref(), Some("v"));
    assert_eq!(get_mission_meta(&conn, "missing").unwrap(), None);
}

#[test]
fn update_node_status_unknown_fails() {
    let conn = mem();
    assert!(update_node_status(&conn, "nope", "done").is_err());
}

#[test]
fn append_event_unknown_node_fails_closed() {
    let conn = mem();
    // Verification records its evidence event in the same transaction as the
    // node mutation. Unknown membership must fail before any event is added.
    let err = set_verify_bit_with_evidence(&conn, "nope", true, Some("detail")).unwrap_err();
    assert!(
        err.to_string().contains("unknown node"),
        "unexpected err: {err}"
    );
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "failed evidence append must leave zero events");

    // Known leaf: evidence event commits with verification.
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
    let id = list_all(&conn).unwrap()[0].id.clone();
    set_verify_bit_with_evidence(&conn, &id, true, Some("hello")).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sdlc_events \
             WHERE node_id = ?1 AND kind = 'verify_evidence'",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn parent_identity_preserved_on_title_match() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "p".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "c".into(),
                status: "pending".into(),
                parent_title: Some("p".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let c1 = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|n| n.title == "c")
        .unwrap();
    let pid = c1.parent_id.clone().unwrap();

    // Re-upsert without parent_title — parent_id preserved.
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "p".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "c".into(),
                status: "active".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let c2 = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|n| n.title == "c")
        .unwrap();
    assert_eq!(c2.parent_id.as_deref(), Some(pid.as_str()));
    assert_eq!(c2.id, c1.id);
}

#[test]
fn verify_with_evidence_is_atomic_on_unknown() {
    let conn = mem();
    // Unknown node: no evidence row may be left behind.
    assert!(set_verify_bit_with_evidence(&conn, "nope", true, Some("ev")).is_err());
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn update_node_status_atomic_and_clears_verify_when_reopening() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
    let id = list_all(&conn).unwrap()[0].id.clone();
    set_verify_bit_with_evidence(&conn, &id, true, None).unwrap();
    let done = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(done.status, "done");
    assert!(done.verify_bit);

    update_node_status(&conn, &id, "active").unwrap();
    let reopened = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(reopened.status, "active");
    assert!(!reopened.verify_bit);

    let kinds: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT kind FROM sdlc_events WHERE node_id = ?1 ORDER BY id")
            .unwrap();
        stmt.query_map([&id], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert!(kinds.iter().any(|k| k == "status_change"));
}

#[test]
fn snapshot_checklist_roundtrips_for_restore() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "epic".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            },
            ChecklistNode {
                title: "leaf".into(),
                status: "active".into(),
                parent_title: Some("epic".into()),
                id: None,

                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let snap = snapshot_checklist(&conn).unwrap();
    assert_eq!(snap.len(), 2);
    // Mutate graph away from snapshot.
    replace_nodes_from_checklist(&conn, &flat(&[("other", "pending")])).unwrap();
    assert_eq!(list_open(&conn).unwrap().len(), 1);
    assert_eq!(list_open(&conn).unwrap()[0].title, "other");
    // Restore.
    replace_nodes_from_checklist(&conn, &snap).unwrap();
    let titles: std::collections::HashSet<_> = list_open(&conn)
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert!(titles.contains("epic"));
    assert!(titles.contains("leaf"));
    assert!(!titles.contains("other"));
}

#[test]
fn claim_leaf_is_exclusive_second_claim_fails() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
    let id = list_all(&conn).unwrap()[0].id.clone();
    claim_leaf(&conn, &id).unwrap();
    let n = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(n.status, "active");
    // Second claim on already-active leaf must fail closed.
    let err = claim_leaf(&conn, &id).unwrap_err().to_string();
    assert!(
        err.contains("already claimed") || err.contains("not claimable"),
        "unexpected err: {err}"
    );
    // Still a single active node.
    assert_eq!(get_node(&conn, &id).unwrap().unwrap().status, "active");
    // Done leaf also not claimable — seal via verify first.
    set_verify_bit_with_evidence(&conn, &id, true, Some("ok")).unwrap();
    assert!(claim_leaf(&conn, &id).is_err());
}

#[test]
fn claim_leaf_rejects_second_active_leaf() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending"), ("b", "pending")])).unwrap();
    let nodes = list_all(&conn).unwrap();
    let a = nodes.iter().find(|n| n.title == "a").unwrap().id.clone();
    let b = nodes.iter().find(|n| n.title == "b").unwrap().id.clone();
    claim_leaf(&conn, &a).unwrap();
    let err = claim_leaf(&conn, &b).unwrap_err().to_string();
    assert!(
        err.contains("another leaf is already active"),
        "unexpected: {err}"
    );
}

#[test]
fn bare_done_rejected_without_verify() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
    let id = list_all(&conn).unwrap()[0].id.clone();
    let err = update_node_status(&conn, &id, "done")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("mission_verify") || err.contains("verify_bit"),
        "{err}"
    );
    // Checklist replace cannot invent bare done either.
    let err = replace_nodes_from_checklist(&conn, &flat(&[("a", "done")]))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("mission_verify") || err.contains("verify_bit") || err.contains("done"),
        "{err}"
    );
}

#[test]
fn owned_paths_roundtrip_through_checklist() {
    let conn = mem();
    let items = vec![
        ChecklistNode {
            title: "task-a".into(),
            status: "active".into(),
            parent_title: None,
            id: None,
            owned_paths: vec!["src/foo.rs".into(), "src/bar/**".into()],
        },
        ChecklistNode {
            title: "task-b".into(),
            status: "active".into(),
            parent_title: None,
            id: None,
            owned_paths: vec![],
        },
    ];
    replace_nodes_from_checklist(&conn, &items).unwrap();

    // Verify owned_paths are persisted in the graph.
    let all = list_all(&conn).unwrap();
    let a = all.iter().find(|n| n.title == "task-a").unwrap();
    assert_eq!(a.owned_paths, vec!["src/foo.rs", "src/bar/**"]);
    let b = all.iter().find(|n| n.title == "task-b").unwrap();
    assert!(b.owned_paths.is_empty());

    // Snapshot preserves owned_paths.
    let snap = snapshot_checklist(&conn).unwrap();
    let sa = snap.iter().find(|n| n.title == "task-a").unwrap();
    assert_eq!(sa.owned_paths, vec!["src/foo.rs", "src/bar/**"]);

    // get_node also returns owned_paths.
    let ga = get_node(&conn, &a.id).unwrap().unwrap();
    assert_eq!(ga.owned_paths, vec!["src/foo.rs", "src/bar/**"]);
}

#[test]
fn check_path_ownership_rejects_foreign_write() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "node-a".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec!["src/foo.rs".into()],
            },
            ChecklistNode {
                title: "node-b".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec!["src/bar.rs".into()],
            },
        ],
    )
    .unwrap();
    let all = list_all(&conn).unwrap();
    let id_a = all.iter().find(|n| n.title == "node-a").unwrap().id.clone();
    let id_b = all.iter().find(|n| n.title == "node-b").unwrap().id.clone();

    // Node A writing to its own path is allowed.
    assert!(check_path_ownership(&conn, Some(&id_a), "src/foo.rs").is_ok());
    // Node A writing to node B's path is REJECTED (foreign).
    assert!(check_path_ownership(&conn, Some(&id_a), "src/bar.rs").is_err());
    // Node B writing to node A's path is REJECTED.
    assert!(check_path_ownership(&conn, Some(&id_b), "src/foo.rs").is_err());
    // Own non-empty owned_paths: unowned path outside own globs is denied.
    assert!(check_path_ownership(&conn, Some(&id_a), "README.md").is_err());
    // Main session (no active node) writing to any owned path is rejected.
    assert!(check_path_ownership(&conn, None, "src/foo.rs").is_err());
    assert!(check_path_ownership(&conn, None, "src/bar.rs").is_err());
}

#[test]
fn check_path_ownership_empty_own_allows_unowned() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "empty-own".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            },
            ChecklistNode {
                title: "foreign".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec!["src/secret.rs".into()],
            },
        ],
    )
    .unwrap();
    let all = list_all(&conn).unwrap();
    let id = all
        .iter()
        .find(|n| n.title == "empty-own")
        .unwrap()
        .id
        .clone();
    // Empty own: any non-foreign path ok.
    assert!(check_path_ownership(&conn, Some(&id), "README.md").is_ok());
    assert!(check_path_ownership(&conn, Some(&id), "src/other.rs").is_ok());
    // Foreign still denied.
    assert!(check_path_ownership(&conn, Some(&id), "src/secret.rs").is_err());
}

#[test]
fn check_path_ownership_glob_matching() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[ChecklistNode {
            title: "glob-task".into(),
            status: "active".into(),
            parent_title: None,
            id: None,
            owned_paths: vec!["src/**/*.rs".into(), "tests/*.rs".into()],
        }],
    )
    .unwrap();
    let id = list_all(&conn).unwrap()[0].id.clone();
    let fake_id = "n-not-this-one-000";

    // Glob match: src/sub/mod.rs matches src/**/*.rs
    assert!(check_path_ownership(&conn, Some(fake_id), "src/sub/mod.rs").is_err());
    // Glob match: tests/unit.rs matches tests/*.rs
    assert!(check_path_ownership(&conn, Some(fake_id), "tests/unit.rs").is_err());
    // No match on foreign empty-own: Cargo.toml is not under foreign globs.
    assert!(check_path_ownership(&conn, Some(fake_id), "Cargo.toml").is_ok());
    // Own node writing inside globs is allowed.
    assert!(check_path_ownership(&conn, Some(&id), "src/sub/mod.rs").is_ok());
    // Own node writing outside non-empty owned_paths is denied.
    assert!(check_path_ownership(&conn, Some(&id), "Cargo.toml").is_err());
}

#[test]
fn handoff_partial_keeps_active_and_appends_note() {
    let conn = mem();
    let id = active_leaf(&conn, "leaf", vec![]);
    let mut handoff = validated_handoff(&id, HandoffStatus::Partial);
    handoff.envelope.updates.node_note = Some("progress note".into());

    let outcome = apply_handoff(&conn, &id, &handoff).unwrap();
    assert_eq!(
        outcome,
        HandoffOutcome {
            children_added: false,
            claim_must_be_cleared: false,
        }
    );
    let node = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(node.status, "active");
    assert!(!node.verify_bit);
    assert_eq!(node.notes, "progress note");
}

#[test]
fn handoff_blocked_only_blocks_claimed_leaf() {
    let conn = mem();
    let id = active_leaf(&conn, "leaf", vec![]);
    let handoff = validated_handoff(&id, HandoffStatus::Blocked);

    let outcome = apply_handoff(&conn, &id, &handoff).unwrap();
    assert_eq!(
        outcome,
        HandoffOutcome {
            children_added: false,
            claim_must_be_cleared: true,
        }
    );
    let node = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(node.status, "blocked");
    assert!(!node.verify_bit);
    assert_eq!(list_all(&conn).unwrap().len(), 1);
}

#[test]
fn handoff_done_never_seals_or_verifies() {
    let conn = mem();
    let id = active_leaf(&conn, "leaf", vec![]);
    let handoff = validated_handoff(&id, HandoffStatus::Done);

    let outcome = apply_handoff(&conn, &id, &handoff).unwrap();
    // done is report-only: node stays active, claim stays valid until verify.
    assert_eq!(
        outcome,
        HandoffOutcome {
            children_added: false,
            claim_must_be_cleared: false,
        }
    );
    let node = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(node.status, "active");
    assert!(!node.verify_bit);
}

#[test]
fn handoff_wrong_and_stale_ids_are_denied_without_application_writes() {
    let conn = mem();
    let id = active_leaf(&conn, "leaf", vec![]);
    let handoff = validated_handoff(&id, HandoffStatus::Partial);
    let events_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();

    assert!(apply_handoff(&conn, "n-wrong", &handoff).is_err());
    assert_eq!(get_node(&conn, &id).unwrap().unwrap().status, "active");
    let events_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(events_after, events_before);

    update_node_status(&conn, &id, "blocked").unwrap();
    let stale_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert!(apply_handoff(&conn, &id, &handoff).is_err());
    assert_eq!(get_node(&conn, &id).unwrap().unwrap().status, "blocked");
    let stale_events_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stale_events_after, stale_events);
}

#[test]
fn handoff_children_inherit_paths_and_require_claim_clear() {
    let conn = mem();
    let owned_paths = vec!["src/sdlc/**".into(), "tests/sdlc.rs".into()];
    let id = active_leaf(&conn, "parent", owned_paths.clone());
    let mut handoff = validated_handoff(&id, HandoffStatus::Partial);
    handoff.envelope.updates.child_proposals = vec![ChildProposal {
        title: "child work".into(),
        note: Some("follow-up".into()),
    }];

    let outcome = apply_handoff(&conn, &id, &handoff).unwrap();
    assert_eq!(
        outcome,
        HandoffOutcome {
            children_added: true,
            claim_must_be_cleared: true,
        }
    );
    assert!(!is_leaf(&conn, &id).unwrap());
    let child = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|node| node.title == "child work")
        .unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(id.as_str()));
    assert_eq!(child.status, "pending");
    assert_eq!(child.owned_paths, owned_paths);
    assert_eq!(child.notes, "follow-up");
    assert_eq!(get_node(&conn, &id).unwrap().unwrap().status, "active");
}

#[test]
fn handoff_duplicate_and_depth_child_failures_rollback() {
    let conn = mem();
    replace_nodes_from_checklist(
        &conn,
        &[
            ChecklistNode {
                title: "parent".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            },
            ChecklistNode {
                title: "taken".into(),
                status: "cancelled".into(),
                parent_title: Some("parent".into()),
                id: None,
                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let parent = list_all(&conn)
        .unwrap()
        .into_iter()
        .find(|node| node.title == "parent")
        .unwrap()
        .id;
    let mut duplicate = validated_handoff(&parent, HandoffStatus::Partial);
    duplicate.envelope.updates.node_note = Some("must not persist".into());
    duplicate.envelope.updates.child_proposals = vec![
        ChildProposal {
            title: "new child".into(),
            note: None,
        },
        ChildProposal {
            title: "taken".into(),
            note: None,
        },
    ];
    let events_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert!(apply_handoff(&conn, &parent, &duplicate).is_err());
    assert_eq!(get_node(&conn, &parent).unwrap().unwrap().notes, "");
    assert!(list_all(&conn)
        .unwrap()
        .iter()
        .all(|node| node.title != "new child"));
    let events_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(events_after, events_before);

    let depth_conn = mem();
    replace_nodes_from_checklist(
        &depth_conn,
        &[
            ChecklistNode {
                title: "root".into(),
                status: "active".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            },
            ChecklistNode {
                title: "middle".into(),
                status: "active".into(),
                parent_title: Some("root".into()),
                id: None,
                owned_paths: vec![],
            },
            ChecklistNode {
                title: "deep leaf".into(),
                status: "active".into(),
                parent_title: Some("middle".into()),
                id: None,
                owned_paths: vec![],
            },
        ],
    )
    .unwrap();
    let leaf = list_all(&depth_conn)
        .unwrap()
        .into_iter()
        .find(|node| node.title == "deep leaf")
        .unwrap()
        .id;
    let mut too_deep = validated_handoff(&leaf, HandoffStatus::Partial);
    too_deep.envelope.updates.node_note = Some("must not persist".into());
    too_deep.envelope.updates.child_proposals = vec![ChildProposal {
        title: "fourth level".into(),
        note: None,
    }];
    let depth_events: i64 = depth_conn
        .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| row.get(0))
        .unwrap();
    assert!(apply_handoff(&depth_conn, &leaf, &too_deep).is_err());
    assert_eq!(get_node(&depth_conn, &leaf).unwrap().unwrap().notes, "");
    assert_eq!(list_all(&depth_conn).unwrap().len(), 3);
    assert_eq!(
        depth_conn
            .query_row("SELECT COUNT(*) FROM sdlc_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        depth_events
    );
}

#[test]
fn latest_verified_commit_shas_extracts_correctly() {
    let conn = mem();
    replace_nodes_from_checklist(&conn, &flat(&[("a", "pending"), ("b", "pending")])).unwrap();
    let all = list_all(&conn).unwrap();
    let a_id = all.iter().find(|n| n.title == "a").unwrap().id.clone();
    let b_id = all.iter().find(|n| n.title == "b").unwrap().id.clone();

    // Node a: two verify events, only second has commit
    set_verify_bit_with_evidence(&conn, &a_id, true, Some("first pass")).unwrap();
    set_verify_bit_with_evidence(&conn, &a_id, false, None).unwrap();
    set_verify_bit_with_evidence(&conn, &a_id, true, Some("tests green | commit:abc1234def"))
        .unwrap();

    // Node b: verify with commit only
    set_verify_bit_with_evidence(&conn, &b_id, true, Some("build ok | commit:ff00aa1"))
        .unwrap();

    let node_ids = vec![a_id.clone(), b_id.clone()];
    let shas = latest_verified_commit_shas(&conn, &node_ids).unwrap();

    // a: should have abc1234def
    assert_eq!(shas.len(), 2);
    let a_shas = shas.get(&a_id).unwrap();
    assert!(
        a_shas.iter().any(|s| s == "abc1234def"),
        "expected abc1234def in a_shas: {a_shas:?}"
    );

    // b: should have ff00aa1
    let b_shas = shas.get(&b_id).unwrap();
    assert!(
        b_shas.iter().any(|s| s == "ff00aa1"),
        "expected ff00aa1 in b_shas: {b_shas:?}"
    );

    // Non-existent node: empty
    let missing = latest_verified_commit_shas(&conn, &["n-missing-000".to_string()]).unwrap();
    assert!(missing.get("n-missing-000").unwrap().is_empty());
}

#[test]
fn handoff_accepted_and_rejected_audits_are_json_and_rejection_is_audit_only() {
    let conn = mem();
    let id = active_leaf(&conn, "leaf", vec![]);
    let mut handoff = validated_handoff(&id, HandoffStatus::Partial);
    handoff.envelope.summary = "completed parser".into();
    handoff.envelope.artifacts = vec!["src/parser.rs".into()];
    handoff.envelope.evidence_refs = vec!["tests/parser.txt".into()];
    handoff.envelope.commit_shas = vec!["abcdef1234567890abcdef1234567890abcdef12".into()];
    handoff.envelope.decisions = vec!["kept the parser strict".into()];
    apply_handoff(&conn, &id, &handoff).unwrap();

    let accepted: String = conn
        .query_row(
            "SELECT detail FROM sdlc_events WHERE kind = 'handoff_accepted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let accepted: serde_json::Value = serde_json::from_str(&accepted).unwrap();
    assert_eq!(accepted["outcome"], "accepted");
    assert_eq!(accepted["summary"], "completed parser");
    assert_eq!(accepted["status"], "partial");
    assert_eq!(accepted["artifacts"][0], "src/parser.rs");
    assert_eq!(accepted["evidence_refs"][0], "tests/parser.txt");
    assert_eq!(
        accepted["commit_shas"][0],
        "abcdef1234567890abcdef1234567890abcdef12"
    );
    assert_eq!(accepted["decisions"][0], "kept the parser strict");

    let before = get_node(&conn, &id).unwrap().unwrap();
    record_rejected_handoff(&conn, Some("n-other"), Some(&id), "stale claim").unwrap();
    let after = get_node(&conn, &id).unwrap().unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.notes, before.notes);
    assert_eq!(after.verify_bit, before.verify_bit);
    let rejected: String = conn
        .query_row(
            "SELECT detail FROM sdlc_events WHERE kind = 'handoff_rejected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let rejected: serde_json::Value = serde_json::from_str(&rejected).unwrap();
    assert_eq!(rejected["outcome"], "rejected");
    assert_eq!(rejected["reason"], "stale claim");
}
