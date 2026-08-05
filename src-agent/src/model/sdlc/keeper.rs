//! SDLC Keeper: deterministic post-turn check that catches false-done nodes
//! (marked done without verify evidence) and nudges the model to verify or
//! integrate. No second LLM required.

use std::path::Path;

use super::graph;
use super::Mission;

/// Report produced by the keeper evaluation.
#[derive(Debug, Clone, Default)]
pub struct KeeperReport {
    /// Tasks reopened from false-done: "id: title"
    pub reopened: Vec<String>,
    /// Full inject body if action is needed, to be pushed as a user turn.
    pub inject: Option<String>,
    /// Current mission phase hint.
    pub phase_hint: Option<String>,
}

/// Content-hash of the last inject we sent, stored in mission_meta so we
/// dedupe identical nudges within one idle window.
const META_KEY_LAST_INJECT: &str = "keeper_last_inject";

/// Run keeper against session dir. Only meaningful if mission is approved and
/// phase is execute|integrate. Returns a `KeeperReport` with optional inject.
pub fn evaluate(session_dir: &Path) -> KeeperReport {
    let mut report = KeeperReport::default();

    // 1. Load mission; bail if none or not approved.
    let mission = match Mission::load(session_dir) {
        Some(m) if m.approved => m,
        _ => return report,
    };
    report.phase_hint = Some(mission.phase.clone());

    // Only meaningful during execute or integrate phases.
    if mission.phase != "execute" && mission.phase != "integrate" {
        return report;
    }

    // 2. Open msglog, ensure tables.
    let conn = match crate::model::msglog::open(session_dir) {
        Ok(c) => c,
        Err(_) => return report,
    };
    let _ = graph::ensure_tables(&conn);

    // 3. Reopen false-done nodes.
    let reopened = match graph::reopen_false_done(&conn) {
        Ok(r) => r,
        Err(_) => return report,
    };

    if !reopened.is_empty() {
        // Build inject body for false-done reopen.
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
             Run the verify_plan steps, then call mission_verify for each node \
             (or mark verify via mission_verify tool) before sealing done again.\n\
             OPEN/SEALED law still applies.",
            reopened_lines.join("\n")
        );
        report.inject = Some(inject);
    } else if mission.phase == "execute" {
        // 4. Soft nudge: all open nodes are sealed, no false-done left.
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

    // 5. Dedupe: skip if this exact inject was already sent.
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
        if let Ok(Some(prev)) = graph::get_mission_meta(&conn, META_KEY_LAST_INJECT) {
            if prev == inject_hash {
                report.inject = None;
                return report;
            }
        }
        let _ = graph::set_mission_meta(&conn, META_KEY_LAST_INJECT, &inject_hash);
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
    let open_titles: Vec<String> = open.iter().map(|n| format!("- {} [{}]", n.title, n.status)).collect();
    let sealed_titles: Vec<String> = sealed
        .iter()
        .map(|n| {
            format!(
                "- {} [done, verify={}]",
                n.title,
                if n.verify_bit { "1" } else { "0" }
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
/// `allow=true` → None (healthy). `allow=false` → Some(reason wrapped as keeper nudge).
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
    use crate::model::sdlc::graph;
    use crate::model::sdlc::Mission;
    use std::path::PathBuf;

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

    fn write_mission(dir: &std::path::Path, phase: &str, approved: bool) {
        let m = Mission {
            id: "m-k".into(),
            goal: "g".into(),
            non_goals: vec![],
            acceptance: vec!["a".into()],
            lane: "express".into(),
            verify_plan: vec!["cargo test".into()],
            human_gates: vec![],
            risks: vec![],
            worktree_name: Some("wt".into()),
            branch: Some("sdlc/g".into()),
            rationale: String::new(),
            phase: phase.into(),
            approved,
            hash: Mission::compute_hash("g", &["a".into()], &[]),
        };
        m.save(dir).unwrap();
    }

    #[test]
    fn keeper_reopens_false_done() {
        let dir = tmp_session();
        write_mission(&dir, "execute", true);
        // open messages.sqlite via graph ensure on msglog open
        let conn = crate::model::msglog::open(&dir).unwrap();
        graph::ensure_tables(&conn).unwrap();
        graph::replace_nodes_from_checklist(
            &conn,
            &[("ship it".into(), "done".into())],
        )
        .unwrap();
        drop(conn);

        let report = evaluate(&dir);
        assert_eq!(report.reopened.len(), 1);
        assert!(report.inject.as_ref().unwrap().contains("False-done"));
        // second call dedupes
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
}
