//! SDLC Keeper: deterministic post-turn check that catches false-done nodes
//! (marked done without verify evidence), stalled progress, and nudges the
//! model to verify or integrate. No second LLM required for the base path.

use std::path::Path;

use super::decompose::{KEEPER_STALL_SECS, META_LAST_GRAPH_FINGERPRINT, META_LAST_TOOL_ROUND_AT};
use super::graph;
use super::mission::current_git_branch;
use super::Mission;

/// Typed action produced by the keeper when the mission contract is invalid
/// and requires a fail-closed reassessment rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperAction {
    /// Mission contract hash is invalid, graph hash is missing, or the
    /// mission binding was lost. The runtime/deferred boundary must mark
    /// the disk mission as needing reassessment and transition to assess.
    RequireReassessment { reason: String },
}

/// Report produced by the keeper evaluation.
#[derive(Debug, Clone, Default)]
pub struct KeeperReport {
    /// Tasks reopened from false-done: "id: title"
    pub reopened: Vec<String>,
    /// Full inject body if action is needed, to be pushed as a user turn.
    pub inject: Option<String>,
    /// Current mission phase hint.
    pub phase_hint: Option<String>,
    /// Typed action for the runtime/deferred boundary to consume.
    pub action: Option<KeeperAction>,
}

/// Content-hash of the last inject we sent, stored in mission_meta so we
/// dedupe identical nudges within one idle window.
const META_KEY_LAST_INJECT: &str = "keeper_last_inject";

/// Run keeper against session dir. Only meaningful if mission is approved and
/// phase is execute|integrate. Returns a `KeeperReport` with optional inject.
pub fn evaluate(session_dir: &Path) -> KeeperReport {
    let mut report = KeeperReport::default();

    // 1. Load mission; bail if none or not approved / not active.
    let mission = match Mission::load(session_dir) {
        Some(m) if m.approved && !m.needs_reapproval => m,
        _ => return report,
    };
    report.phase_hint = Some(mission.phase.clone());

    if mission.phase != "execute" && mission.phase != "integrate" && mission.phase != "prepare" {
        return report;
    }

    // Fail-closed: invalid contract or lost binding → reassessment rail.
    // Validate the bound worktree itself, never the session path: keeper may run
    // outside the mission worktree, while the frozen binding remains authoritative.
    let binding_lost = match mission.worktree_path.as_deref() {
        Some(path) if Path::new(path).is_dir() && mission.branch.is_some() => {
            let worktree = Path::new(path);
            let live_branch = current_git_branch(worktree);
            mission
                .validate_binding(worktree, live_branch.as_deref())
                .is_err()
        }
        _ => true,
    };
    if !mission.hash_valid() || mission.graph_hash.is_none() || binding_lost {
        let reason = if !mission.hash_valid() {
            "contract hash invalid"
        } else if mission.graph_hash.is_none() {
            "graph hash missing"
        } else {
            "mission binding lost"
        };
        report.action = Some(KeeperAction::RequireReassessment {
            reason: reason.to_string(),
        });
        report.inject = Some(
            "[SDLC keeper]\n\
             Mission contract hash/graph binding is invalid or legacy-unbound. \
             Fail closed: return to assess, call mission_ready again (amendment path)."
                .into(),
        );
        return finalize_dedupe(session_dir, report);
    }

    // 2. Open msglog, ensure tables.
    let conn = match crate::model::msglog::open(session_dir) {
        Ok(c) => c,
        Err(_) => return report,
    };
    let _ = graph::ensure_tables(&conn);

    // 3. Reopen false-done nodes (leaves only; ancestors included).
    let reopened = match graph::reopen_false_done(&conn) {
        Ok(r) => r,
        Err(_) => return report,
    };

    if !reopened.is_empty() {
        let reopened_lines: Vec<String> = reopened
            .iter()
            .map(|n| format!("- {}: {}", n.id, n.title))
            .collect();
        report.reopened = reopened
            .iter()
            .map(|n| format!("{}: {}", n.id, n.title))
            .collect();

        let inject = format!(
            "[SDLC keeper]\n\
             False-done reopened (done without verify evidence). Do NOT treat these as finished.\n\
             Reopened:\n{}\n\
             Run the verify_plan steps, then call mission_verify for each LEAF node \
             before sealing done again.\n\
             OPEN/SEALED law still applies. Cancellation is never verification.",
            reopened_lines.join("\n")
        );
        report.inject = Some(inject);
        return finalize_dedupe_conn(&conn, report);
    }

    // 4. Deterministic stall detection: tools ran, open leaves remain, graph
    // fingerprint unchanged for > KEEPER_STALL_SECS.
    if mission.phase == "execute" || mission.phase == "prepare" {
        if let Some(stall) = detect_stall(&conn) {
            report.inject = Some(stall);
            return finalize_dedupe_conn(&conn, report);
        }
    }

    // 5. Soft nudge: all open nodes sealed, no false-done left.
    if mission.phase == "execute" {
        let open = graph::list_open(&conn).unwrap_or_default();
        let sealed = graph::list_sealed(&conn).unwrap_or_default();
        if open.is_empty() && !sealed.is_empty() {
            let inject = "[SDLC keeper]\n\
                All open nodes are sealed. If acceptance criteria are green and \
                verify evidence is in place, call mission_integrate to merge the \
                mission branch."
                .to_string();
            report.inject = Some(inject);
        }
    }

    finalize_dedupe_conn(&conn, report)
}

fn detect_stall(conn: &rusqlite::Connection) -> Option<String> {
    let open_leaves = graph::list_open_leaves(conn).ok()?;
    if open_leaves.is_empty() {
        return None;
    }
    let last_tool = graph::get_mission_meta(conn, META_LAST_TOOL_ROUND_AT)
        .ok()
        .flatten()?
        .parse::<i64>()
        .ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if now.saturating_sub(last_tool) < KEEPER_STALL_SECS {
        return None;
    }
    let prev_fp = graph::get_mission_meta(conn, META_LAST_GRAPH_FINGERPRINT)
        .ok()
        .flatten()?;
    let cur_fp = graph::graph_fingerprint(conn).ok()?;
    // Stall = fingerprint unchanged since last tool-round stamp (no graph progress).
    if prev_fp != cur_fp {
        // Graph moved since stamp — refresh fingerprint baseline, no inject.
        let _ = graph::set_mission_meta(conn, META_LAST_GRAPH_FINGERPRINT, &cur_fp);
        return None;
    }
    let lines: Vec<String> = open_leaves
        .iter()
        .take(8)
        .map(|n| format!("- {}: {} [{}]", n.id, n.title, n.status))
        .collect();
    Some(format!(
        "[SDLC keeper]\n\
         Stall detected: tools ran but the graph has not advanced for >{KEEPER_STALL_SECS}s \
         with open leaves remaining.\n\
         Open leaves:\n{}\n\
         Resume the next OPEN leaf (task.node_id required) or record verify evidence.",
        lines.join("\n")
    ))
}

fn finalize_dedupe(session_dir: &Path, report: KeeperReport) -> KeeperReport {
    let conn = match crate::model::msglog::open(session_dir) {
        Ok(c) => c,
        Err(_) => return report,
    };
    let _ = graph::ensure_tables(&conn);
    finalize_dedupe_conn(&conn, report)
}

fn finalize_dedupe_conn(conn: &rusqlite::Connection, mut report: KeeperReport) -> KeeperReport {
    if let Some(ref inject) = report.inject {
        let inject_hash = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(inject.as_bytes());
            let result = hasher.finalize();
            result[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        if let Ok(Some(prev)) = graph::get_mission_meta(conn, META_KEY_LAST_INJECT) {
            if prev == inject_hash {
                report.inject = None;
                return report;
            }
        }
        let _ = graph::set_mission_meta(conn, META_KEY_LAST_INJECT, &inject_hash);
    }
    report
}

/// Build the user+system messages for an optional Safeguard oneshot that looks
/// for stalled / dishonest progress the deterministic keeper missed.
pub fn llm_keeper_prompt(
    mission: &Mission,
    open: &[super::graph::GraphTask],
    sealed: &[super::graph::GraphTask],
) -> Vec<crate::dto::chat::ChatMessage> {
    use crate::dto::chat::{ChatMessage, Role};
    let system = ChatMessage::new(
        Role::System,
        "You are the SDLC keeper. You review an in-progress SDLC mission for stalled or \
         dishonest progress.\n\
         Reply ONLY with valid JSON: {\"allow\": bool, \"reason\": string}\n\
         - allow=true means healthy progress, NO inject needed.\n\
         - allow=false means the model needs a nudge; reason is the inject body.\n\
         Be concise. Only flag genuine issues: stalled (no progress across sealed tasks), \
         dishonest (tasks marked done without evidence), or off-track (not working toward the goal).",
    );
    let open_titles: Vec<String> = open
        .iter()
        .map(|n| format!("- {} [{}] ({})", n.title, n.status, n.id))
        .collect();
    let sealed_titles: Vec<String> = sealed
        .iter()
        .map(|n| {
            format!(
                "- {} [done, verify={}] ({})",
                n.title,
                if n.verify_bit { "1" } else { "0" },
                n.id
            )
        })
        .collect();
    let user_body = format!(
        "Mission goal: {}\nPhase: {}\nAcceptance: {}\n\nOPEN tasks:\n{}\n\nSEALED tasks:\n{}\n\n\
         Assess whether progress is honest and on-track. Reply JSON only.",
        mission.goal,
        mission.phase,
        mission.acceptance.join(", "),
        if open_titles.is_empty() {
            "(none)".to_string()
        } else {
            open_titles.join("\n")
        },
        if sealed_titles.is_empty() {
            "(none)".to_string()
        } else {
            sealed_titles.join("\n")
        },
    );
    vec![system, ChatMessage::new(Role::User, user_body)]
}

/// Parse a classify reply JSON into an optional inject body.
pub fn llm_verdict_to_inject(reply: &str) -> Option<String> {
    let v: serde_json::Value = match serde_json::from_str(reply) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let allow = v.get("allow").and_then(|a| a.as_bool()).unwrap_or(true);
    if allow {
        None
    } else {
        let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("");
        if reason.is_empty() {
            None
        } else {
            Some(format!("[SDLC keeper — review]\n{reason}"))
        }
    }
}

#[cfg(test)]
#[path = "keeper_test.rs"]
mod tests;
