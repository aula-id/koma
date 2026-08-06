//! Anti-megatask / hierarchy helpers for SDLC execute.
//!
//! Main agent acts as PM: only OPEN leaves may be delegated via `task`, and
//! each delegation must carry an explicit `node_id`. Parents roll up from
//! children; they are never directly verified.

use super::graph::{self, GraphTask};
use rusqlite::Connection;

/// Hard cap on subagent task prompts in SDLC execute (chars).
pub const TASK_PROMPT_HARD_MAX: usize = 4000;

/// Stall window: if tools ran but the graph has not advanced for this many
/// seconds while open leaves remain, the keeper injects a stall nudge.
pub const KEEPER_STALL_SECS: i64 = 90;

/// mission_meta key: unix secs of last finished tool round in SDLC.
pub const META_LAST_TOOL_ROUND_AT: &str = "last_tool_round_at";

/// mission_meta key: last observed graph content fingerprint for stall detection.
pub const META_LAST_GRAPH_FINGERPRINT: &str = "last_graph_fingerprint";

/// Validate lane against the proposed graph structure.
///
/// - `express`: free form
/// - `standard`: hard-reject a single flat node with ≥3 acceptance criteria
/// - `full`: requires a tree (any parent link) OR ≥3 leaves
pub fn validate_lane_graph(
    lane: &str,
    nodes: &[graph::ChecklistNode],
    acceptance_len: usize,
) -> Result<(), String> {
    let lane = lane.trim().to_ascii_lowercase();
    let leaf_count = count_leaves(nodes);
    let has_tree = nodes.iter().any(|n| n.parent_title.is_some());

    match lane.as_str() {
        "express" => Ok(()),
        "full" => {
            if has_tree || leaf_count >= 3 {
                Ok(())
            } else {
                Err(
                    "error: full lane requires a hierarchical graph (parent links) \
                     or at least 3 leaf tasks"
                        .into(),
                )
            }
        }
        // standard (default) and anything else
        _ => {
            if nodes.len() == 1 && !has_tree && acceptance_len >= 3 {
                Err(
                    "error: standard lane rejects a single megatask with ≥3 acceptance \
                     criteria — decompose into multiple leaf tasks or a hierarchy"
                        .into(),
                )
            } else {
                Ok(())
            }
        }
    }
}

fn count_leaves(nodes: &[graph::ChecklistNode]) -> usize {
    let parents: std::collections::HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.parent_title.as_deref())
        .collect();
    nodes
        .iter()
        .filter(|n| !parents.contains(n.title.as_str()))
        .count()
}

/// Result of a successful leaf claim for task delegation.
#[derive(Debug, Clone)]
pub struct LeafClaim {
    pub node_id: String,
    pub title: String,
}

/// Validate that a `task` delegation targets exactly one OPEN leaf.
///
/// - `node_id` required
/// - target must exist, be open (not done/cancelled), and be a leaf
/// - prompt length ≤ TASK_PROMPT_HARD_MAX
/// - multi-leaf title patterns in the prompt are rejected
pub fn validate_task_delegation(
    conn: &Connection,
    node_id: Option<&str>,
    prompt: &str,
) -> Result<LeafClaim, String> {
    let node_id = node_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "error: SDLC execute requires task.node_id set to an OPEN leaf graph node".to_string()
        })?;

    if prompt.chars().count() > TASK_PROMPT_HARD_MAX {
        return Err(format!(
            "error: task prompt exceeds {TASK_PROMPT_HARD_MAX} characters — \
             split the work; one leaf per delegation"
        ));
    }

    let all = graph::list_all(conn).map_err(|e| format!("error: graph read failed: {e}"))?;
    let node = all
        .iter()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("error: unknown graph node_id '{node_id}'"))?;

    if node.status == "done" || node.status == "cancelled" {
        return Err(format!(
            "error: node '{node_id}' is {} — only OPEN leaves may be delegated",
            node.status
        ));
    }

    if !graph::is_leaf(conn, &node.id).unwrap_or(false) {
        return Err(format!(
            "error: node '{node_id}' ('{}') is a parent — delegate OPEN leaves only",
            node.title
        ));
    }

    // Soft multi-leaf smell: prompt enumerates several distinct OPEN leaf titles.
    let open_leaves = graph::list_open_leaves(conn).unwrap_or_default();
    let mentioned: Vec<&GraphTask> = open_leaves
        .iter()
        .filter(|n| n.id != node.id && prompt.contains(&n.title) && n.title.len() >= 8)
        .collect();
    if mentioned.len() >= 2 {
        return Err("error: task prompt references multiple OPEN leaf titles — \
             one leaf per delegation (anti-megatask)"
            .into());
    }

    // Every task delegation must atomically claim an OPEN pending/blocked leaf.
    // Already-active leaves fail exclusive claim — no second delegation may
    // target the same active leaf (defeats the exclusive claim invariant).
    if node.status != "pending" && node.status != "blocked" {
        return Err(format!(
            "error: node '{node_id}' status '{}' is not claimable — only pending/blocked OPEN leaves may be delegated",
            node.status
        ));
    }
    graph::claim_leaf(conn, &node.id)
        .map_err(|e| format!("error: could not claim leaf '{node_id}': {e}"))?;

    Ok(LeafClaim {
        node_id: node.id.clone(),
        title: node.title.clone(),
    })
}

/// Build the scope banner prepended to a subagent prompt.
pub fn scope_banner(claim: &LeafClaim) -> String {
    format!(
        "[SDLC leaf scope]\n\
         node_id: {}\n\
         title: {}\n\
         Stay inside this leaf only. Do not expand scope to sibling or parent nodes.\n\
         ---\n\n",
        claim.node_id, claim.title
    )
}

#[cfg(test)]
mod tests {
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
        }];
        assert!(validate_lane_graph("full", &one, 1).is_err());

        let three: Vec<_> = (0..3)
            .map(|i| ChecklistNode {
                title: format!("t{i}"),
                status: "pending".into(),
                parent_title: None,
                id: None,
            })
            .collect();
        assert!(validate_lane_graph("full", &three, 1).is_ok());

        let tree = vec![
            ChecklistNode {
                title: "epic".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,
            },
            ChecklistNode {
                title: "leaf".into(),
                status: "pending".into(),
                parent_title: Some("epic".into()),
                id: None,
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
                },
                ChecklistNode {
                    title: "child".into(),
                    status: "pending".into(),
                    parent_title: Some("parent".into()),
                    id: None,
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
            }],
        )
        .unwrap();
        let id = graph::list_all(&conn).unwrap()[0].id.clone();
        let big = "x".repeat(TASK_PROMPT_HARD_MAX + 1);
        let err = validate_task_delegation(&conn, Some(&id), &big).unwrap_err();
        assert!(err.contains("exceeds"));
    }
}
