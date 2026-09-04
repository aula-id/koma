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
/// Delegates to [`super::lane::validate_lane_graph`] (single policy home).
pub fn validate_lane_graph(
    lane: &str,
    nodes: &[graph::ChecklistNode],
    acceptance_len: usize,
) -> Result<(), String> {
    super::lane::validate_lane_graph(lane, nodes, acceptance_len)
}

/// Result of a successful leaf claim for task delegation.
#[derive(Debug, Clone)]
pub struct LeafClaim {
    pub node_id: String,
    pub title: String,
    /// Glob patterns this node owns (for path-ownership enforcement).
    pub owned_paths: Vec<String>,
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
        owned_paths: node.owned_paths.clone(),
    })
}

/// Build the scope banner prepended to a subagent prompt.
pub fn scope_banner(claim: &LeafClaim) -> String {
    let ownership = if claim.owned_paths.is_empty() {
        String::new()
    } else {
        let patterns: Vec<&str> = claim.owned_paths.iter().map(|s| s.as_str()).collect();
        format!(
            "owned_paths: [{}]\n\
             Write/edit/delete to paths matching these patterns is your responsibility.\n\
             Write/edit/delete to paths matching a DIFFERENT active node's patterns is FORBIDDEN.\n",
            patterns.join(", ")
        )
    };
    format!(
        "[SDLC leaf scope]\n\
         node_id: {}\n\
         title: {}\n\
         {ownership}\
         Stay inside this leaf only. Do not expand scope to sibling or parent nodes.\n\
         \n\
         To report completion, include a JSON block in your final output wrapped in:\n\
         <!-- SDLC_HANDOFF_JSON_START -->\n\
         {{ \"version\": 1, \"node_id\": \"...\", \"status\": \"done|partial|blocked\", \"summary\": \"...\", ... }}\n\
         <!-- SDLC_HANDOFF_JSON_END -->\n\
         Use status \"done\" when your work is complete, \"partial\" for progress, \"blocked\" for impediments.\n\
         Do NOT set status to \"done\" expecting graph sealing — only mission_verify seals nodes.\n\
         ---\n\n",
        claim.node_id, claim.title
    )
}

#[cfg(test)]
#[path = "decompose_test.rs"]
mod tests;
