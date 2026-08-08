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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::model::sdlc::graph::{self, ChecklistNode};
    use crate::model::sdlc::Mission;
    use std::path::PathBuf;
    use std::process::Command;

    fn tmp_session() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "koma-keeper-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn create_bound_worktree(dir: &std::path::Path) -> PathBuf {
        let worktree = dir.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        run_git(&worktree, &["init", "-b", "sdlc/g"]);
        run_git(
            &worktree,
            &["config", "user.email", "keeper-test@example.invalid"],
        );
        run_git(&worktree, &["config", "user.name", "Keeper Test"]);
        std::fs::write(worktree.join("README.md"), "test\n").unwrap();
        run_git(&worktree, &["add", "README.md"]);
        run_git(&worktree, &["commit", "-m", "initial"]);
        worktree
    }

    fn write_mission(dir: &std::path::Path, phase: &str, approved: bool) {
        let worktree = create_bound_worktree(dir);
        let graph_hash = Some("deadbeefdeadbeefdeadbeefdeadbeef".into());
        let worktree_name = Some("wt".into());
        let branch = Some("sdlc/g".into());
        let worktree_path = Some(worktree.to_string_lossy().into_owned());
        let target_worktree_path = Some("/tmp/primary".into());
        let target_branch = Some("main".into());
        let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let hash =
            Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
                goal: "g",
                acceptance: &["a".into()],
                non_goals: &[],
                lane: "express",
                verify_plan: &["cargo test".into()],
                human_gates: &[],
                risks: &[],
                rationale: "",
                graph_hash: graph_hash.as_deref(),
                worktree_name: worktree_name.as_deref(),
                branch: branch.as_deref(),
                worktree_path: worktree_path.as_deref(),
                target_worktree_path: target_worktree_path.as_deref(),
                target_branch: target_branch.as_deref(),
                target_head: target_head.as_deref(),
            });
        let m = Mission {
            contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
            id: "m-k".into(),
            goal: "g".into(),
            non_goals: vec![],
            acceptance: vec!["a".into()],
            lane: "express".into(),
            verify_plan: vec!["cargo test".into()],
            human_gates: vec![],
            human_gates_approved: vec![],
            risks: vec![],
            worktree_name,
            branch,
            worktree_path,
            target_worktree_path,
            target_branch,
            target_head,
            rationale: String::new(),
            phase: phase.into(),
            approved,
            hash,
            graph_hash,
            needs_reapproval: false,
            amendment_note: None,
        };
        m.save(dir).unwrap();
    }

    // --- Prepare-phase keeper tests ---

    #[test]
    fn keeper_evaluates_during_prepare_phase() {
        // Keeper does NOT early-exit when phase is prepare.
        let dir = tmp_session();
        write_mission(&dir, "prepare", true);
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: "setup task".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            }],
        )
        .unwrap();
        drop(conn);

        let report = evaluate(&dir);
        // Keeper ran (phase_hint should be prepare).
        assert_eq!(report.phase_hint.as_deref(), Some("prepare"));
        // No inject needed — single pending node is fine.
        assert!(report.inject.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_stall_detection_works_during_prepare() {
        let dir = tmp_session();
        write_mission(&dir, "prepare", true);
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: "pending setup".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            }],
        )
        .unwrap();
        // Stamp tool round in the past.
        let past = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            - KEEPER_STALL_SECS
            - 5;
        graph::set_mission_meta(&conn, META_LAST_TOOL_ROUND_AT, &past.to_string()).unwrap();
        let fp = graph::graph_fingerprint(&conn).unwrap();
        graph::set_mission_meta(&conn, META_LAST_GRAPH_FINGERPRINT, &fp).unwrap();
        drop(conn);

        let report = evaluate(&dir);
        assert!(
            report
                .inject
                .as_ref()
                .is_some_and(|s| s.contains("Stall detected")),
            "prepare phase should trigger stall detection: {:?}",
            report.inject
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reopens_false_done_in_prepare() {
        let dir = tmp_session();
        write_mission(&dir, "prepare", true);
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: "setup item".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            }],
        )
        .unwrap();
        let id = graph::list_all(&conn).unwrap()[0].id.clone();
        // Fake false-done row.
        conn.execute(
            "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
        drop(conn);

        let report = evaluate(&dir);
        assert_eq!(report.reopened.len(), 1);
        assert!(report.inject.as_ref().unwrap().contains("False-done"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reopens_false_done() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: "ship it".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,

                owned_paths: vec![],
            }],
        )
        .unwrap();
        let id = graph::list_all(&conn).unwrap()[0].id.clone();
        // Legacy false-done row for keeper reopen path.
        conn.execute(
            "UPDATE sdlc_nodes SET status = 'done', verify_bit = 0 WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
        drop(conn);

        let report = evaluate(&dir);
        assert_eq!(report.reopened.len(), 1);
        assert!(report.inject.as_ref().unwrap().contains("False-done"));
        let report2 = evaluate(&dir);
        assert!(report2.inject.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_skips_unapproved() {
        let dir = tmp_session();
        write_mission(&dir, "execute", false);
        let report = evaluate(&dir);
        assert!(report.inject.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_stall_when_graph_frozen() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
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
        // Stamp tool round in the past.
        let past = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            - KEEPER_STALL_SECS
            - 5;
        graph::set_mission_meta(&conn, META_LAST_TOOL_ROUND_AT, &past.to_string()).unwrap();
        let fp = graph::graph_fingerprint(&conn).unwrap();
        graph::set_mission_meta(&conn, META_LAST_GRAPH_FINGERPRINT, &fp).unwrap();
        drop(conn);

        let report = evaluate(&dir);
        assert!(
            report
                .inject
                .as_ref()
                .is_some_and(|s| s.contains("Stall detected")),
            "got {:?}",
            report.inject
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn llm_verdict_allow_true_returns_none() {
        let json = r#"{"allow": true, "reason": ""}"#;
        assert!(super::llm_verdict_to_inject(json).is_none());
    }

    #[test]
    fn llm_verdict_allow_false_returns_inject() {
        let json = r#"{"allow": false, "reason": "Tasks stalled with no verify evidence"}"#;
        let inject = super::llm_verdict_to_inject(json).unwrap();
        assert!(inject.contains("[SDLC keeper — review]"));
        assert!(inject.contains("Tasks stalled"));
    }

    #[test]
    fn llm_verdict_allow_false_empty_reason_returns_none() {
        let json = r#"{"allow": false, "reason": ""}"#;
        assert!(super::llm_verdict_to_inject(json).is_none());
    }

    #[test]
    fn llm_verdict_malformed_returns_none() {
        assert!(super::llm_verdict_to_inject("not json").is_none());
        assert!(super::llm_verdict_to_inject("{}").is_none());
    }

    // --- Stage 2: reassessment rail tests ---

    #[test]
    fn keeper_reassessment_on_invalid_hash() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        // Invalidate the stored hash so hash_valid() fails.
        let mut m = Mission::load(&dir).unwrap();
        m.hash = "wrong".to_string();
        m.save(&dir).unwrap();

        let report = evaluate(&dir);
        assert!(
            report.action.is_some(),
            "should produce RequireReassessment"
        );
        match report.action.as_ref().unwrap() {
            super::KeeperAction::RequireReassessment { reason } => {
                assert!(
                    reason.contains("contract hash invalid"),
                    "unexpected reason: {reason}"
                );
            }
        }
        assert!(report.inject.is_some(), "should have inject text");

        // Simulate deferred action handling: mark disk mission reassess.
        let mut m = Mission::load(&dir).unwrap();
        assert!(
            m.approved,
            "mission still approved before deferred mutation"
        );
        m.approved = false;
        m.needs_reapproval = true;
        m.amendment_note = Some("keeper reassessment: contract hash invalid".into());
        let _ = m.try_transition("assess");
        m.save(&dir).unwrap();

        // Disk state: mission is in assess, unapproved, needs reapproval.
        let m2 = Mission::load(&dir).unwrap();
        assert!(!m2.approved);
        assert!(m2.needs_reapproval);
        assert_eq!(m2.phase, "assess");
        assert!(m2.amendment_note.is_some());
        // validate_active fails → tools blocked.
        assert!(m2.validate_active().is_err());
        let (ws, roots) = m2.tool_sandbox_roots(std::path::Path::new("/tmp"));
        assert!(ws.as_os_str().is_empty());
        assert!(roots.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reassessment_on_missing_graph_hash() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        // Remove graph hash but keep contract hash valid.
        let mut m = Mission::load(&dir).unwrap();
        m.graph_hash = None;
        m.hash = m.recompute_hash();
        m.save(&dir).unwrap();

        let report = evaluate(&dir);
        assert!(report.action.is_some());
        match report.action.as_ref().unwrap() {
            super::KeeperAction::RequireReassessment { reason } => {
                assert!(
                    reason.contains("graph hash missing"),
                    "unexpected reason: {reason}"
                );
            }
        }
        assert!(report.inject.is_some());

        // Simulate deferred: mark disk mission reassess.
        let mut m = Mission::load(&dir).unwrap();
        m.approved = false;
        m.needs_reapproval = true;
        let _ = m.try_transition("assess");
        m.save(&dir).unwrap();

        let m2 = Mission::load(&dir).unwrap();
        assert!(!m2.approved);
        assert!(m2.needs_reapproval);
        assert!(m2.validate_active().is_err());
        let (ws, roots) = m2.tool_sandbox_roots(std::path::Path::new("/tmp"));
        assert!(ws.as_os_str().is_empty());
        assert!(roots.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reassessment_on_lost_binding() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        // Lose the mission binding (worktree_path + branch cleared).
        let mut m = Mission::load(&dir).unwrap();
        m.worktree_path = None;
        m.branch = None;
        m.hash = m.recompute_hash();
        m.save(&dir).unwrap();

        let report = evaluate(&dir);
        assert!(report.action.is_some());
        match report.action.as_ref().unwrap() {
            super::KeeperAction::RequireReassessment { reason } => {
                assert!(
                    reason.contains("mission binding lost"),
                    "unexpected reason: {reason}"
                );
            }
        }
        assert!(report.inject.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reassessment_on_missing_worktree() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let worktree = Mission::load(&dir)
            .unwrap()
            .worktree_path
            .map(PathBuf::from)
            .unwrap();
        std::fs::remove_dir_all(&worktree).unwrap();

        let report = evaluate(&dir);
        assert!(matches!(
            report.action,
            Some(super::KeeperAction::RequireReassessment { ref reason })
                if reason.contains("mission binding lost")
        ));
        assert!(report.inject.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reassessment_on_live_branch_mismatch() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let worktree = Mission::load(&dir)
            .unwrap()
            .worktree_path
            .map(PathBuf::from)
            .unwrap();
        run_git(&worktree, &["checkout", "-b", "sdlc/other"]);

        let report = evaluate(&dir);
        assert!(matches!(
            report.action,
            Some(super::KeeperAction::RequireReassessment { ref reason })
                if reason.contains("mission binding lost")
        ));
        assert!(report.inject.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_reassessment_dedupe_on_repeated_eval() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let mut m = Mission::load(&dir).unwrap();
        m.hash = "wrong".to_string();
        m.save(&dir).unwrap();

        // First evaluation: inject present.
        let report1 = evaluate(&dir);
        assert!(report1.inject.is_some(), "first eval should inject");
        assert!(report1.action.is_some());
        // Repeated evaluation: inject deduped (same hash), action still detected.
        let report2 = evaluate(&dir);
        assert!(report2.inject.is_none(), "second eval should be deduped");
        assert!(
            report2.action.is_some(),
            "action should still detect invalid state"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeper_repeated_action_does_not_reinject_after_disk_mutation() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        let mut m = Mission::load(&dir).unwrap();
        m.hash = "wrong".to_string();
        m.save(&dir).unwrap();

        // First evaluation: action + inject.
        let report1 = evaluate(&dir);
        assert!(report1.action.is_some());
        assert!(report1.inject.is_some());

        // Simulate deferred action handling (disk mutation).
        let mut m = Mission::load(&dir).unwrap();
        m.approved = false;
        m.needs_reapproval = true;
        m.amendment_note = Some("keeper reassessment: contract hash invalid".into());
        let _ = m.try_transition("assess");
        m.save(&dir).unwrap();

        // Second evaluation: mission no longer approved → no action, no inject.
        let report2 = evaluate(&dir);
        assert!(report2.action.is_none());
        assert!(report2.inject.is_none());
        assert!(report2.reopened.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
