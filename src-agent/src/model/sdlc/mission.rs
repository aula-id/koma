//! L1 Mission contract: the frozen SDLC spec stored in `<session>/mission.json`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::graph::GraphTask;

/// A frozen SDLC mission contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub acceptance: Vec<String>,
    pub lane: String, // express | standard | full
    pub verify_plan: Vec<String>,
    pub human_gates: Vec<String>,
    pub risks: Vec<String>,
    pub worktree_name: Option<String>,
    pub branch: Option<String>,
    pub rationale: String,
    /// Current phase: assess | execute | integrate | done.
    pub phase: String,
    pub approved: bool,
    /// Content hash of the frozen fields (for change detection).
    pub hash: String,
}

impl Mission {
    /// Compute the content hash of the frozen (pre-approval) fields.
    pub fn compute_hash(goal: &str, acceptance: &[String], non_goals: &[String]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(goal.as_bytes());
        for item in acceptance {
            hasher.update(item.as_bytes());
        }
        for item in non_goals {
            hasher.update(item.as_bytes());
        }
        let result = hasher.finalize();
        // Encode as hex string without `hex` crate — just use format! on each byte.
        result[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Load mission from `<session>/mission.json`.
    pub fn load(session_dir: &std::path::Path) -> Option<Self> {
        let path = session_dir.join("mission.json");
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Save mission to `<session>/mission.json` (atomic write).
    pub fn save(&self, session_dir: &std::path::Path) -> Result<()> {
        let path = session_dir.join("mission.json");
        let json = serde_json::to_string_pretty(self)?;
        crate::model::memory::atomic_write(&path, json.as_bytes())?;
        Ok(())
    }
}

/// Build the SDLC seed capsule for injection into a compacted or fresh
/// conversation. Mandatory OPEN + SEALED sections so the model always
/// sees the full contract context on resume.
pub fn build_seed_capsule(
    mission: &Mission,
    open_nodes: &[GraphTask],
    sealed_nodes: &[GraphTask],
) -> String {
    let mut s = String::from("# SDLC mission capsule\n\n");
    s.push_str(&format!("**Goal:** {}\n", mission.goal));
    if !mission.non_goals.is_empty() {
        s.push_str("**Non-goals:**\n");
        for ng in &mission.non_goals {
            s.push_str(&format!("- {ng}\n"));
        }
    }
    if !mission.acceptance.is_empty() {
        s.push_str("**Acceptance criteria:**\n");
        for ac in &mission.acceptance {
            s.push_str(&format!("- {ac}\n"));
        }
    }
    s.push_str(&format!("**Phase:** {} | **Lane:** {}\n", mission.phase, mission.lane));

    // Worktree / branch info.
    if let Some(ref wt) = mission.worktree_name {
        s.push_str(&format!(
            "**Worktree:** {} (branch: {})\n",
            wt,
            mission.branch.as_deref().unwrap_or("n/a")
        ));
    }

    // Verify plan bullets.
    if !mission.verify_plan.is_empty() {
        s.push_str("**Verify plan:**\n");
        for vp in &mission.verify_plan {
            s.push_str(&format!("- {vp}\n"));
        }
    }

    // Human gates.
    if !mission.human_gates.is_empty() {
        s.push_str("**Human gates:**\n");
        for hg in &mission.human_gates {
            s.push_str(&format!("- {hg}\n"));
        }
    }

    // OPEN tasks
    s.push_str("\n## OPEN\n");
    if open_nodes.is_empty() {
        s.push_str("_none_\n");
    } else {
        for node in open_nodes {
            s.push_str(&format!(
                "- [{}] {} ({})\n",
                node.status, node.title, node.id
            ));
        }
    }

    // SEALED (done) tasks
    s.push_str("\n## SEALED\n");
    if sealed_nodes.is_empty() {
        s.push_str("_none_\n");
    } else {
        for node in sealed_nodes {
            let verify_mark = if node.verify_bit {
                "(verified)"
            } else {
                "(UNVERIFIED)"
            };
            s.push_str(&format!(
                "- [done] {} ({}) {}\n",
                node.title, node.id, verify_mark
            ));
        }
    }

    s
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::model::sdlc::graph::GraphTask;

    fn sample_mission() -> Mission {
        Mission {
            id: "m-test".into(),
            goal: "ship X".into(),
            non_goals: vec!["rewrite Y".into()],
            acceptance: vec!["tests pass".into()],
            lane: "standard".into(),
            verify_plan: vec!["cargo test".into()],
            human_gates: vec![],
            risks: vec!["api churn".into()],
            worktree_name: Some("sdlc-test".into()),
            branch: Some("sdlc/ship-x".into()),
            rationale: "match house style".into(),
            phase: "execute".into(),
            approved: true,
            hash: Mission::compute_hash("ship X", &["tests pass".into()], &["rewrite Y".into()]),
        }
    }

    #[test]
    fn seed_capsule_includes_open_and_sealed() {
        let m = sample_mission();
        let open = vec![GraphTask {
            id: "t1".into(),
            parent_id: None,
            title: "implement".into(),
            status: "active".into(),
            phase: None,
            notes: String::new(),
            verify_bit: false,
            updated_at: 0,
        }];
        let sealed = vec![GraphTask {
            id: "t0".into(),
            parent_id: None,
            title: "assess done".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
        }];
        let cap = build_seed_capsule(&m, &open, &sealed);
        assert!(cap.contains("# SDLC mission capsule"));
        assert!(cap.contains("## OPEN"));
        assert!(cap.contains("## SEALED"));
        assert!(cap.contains("implement"));
        assert!(cap.contains("assess done"));
        assert!(cap.contains("ship X"));
        assert!(cap.contains("tests pass"));
    }

    #[test]
    fn seed_capsule_includes_worktree_and_verify_plan() {
        let m = sample_mission();
        let cap = build_seed_capsule(&m, &[], &[]);
        assert!(cap.contains("**Worktree:** sdlc-test (branch: sdlc/ship-x)"));
        assert!(cap.contains("**Verify plan:**"));
        assert!(cap.contains("- cargo test"));
    }

    #[test]
    fn seed_capsule_shows_verify_status_on_sealed() {
        let m = sample_mission();
        let sealed = vec![GraphTask {
            id: "t1".into(),
            parent_id: None,
            title: "task1".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: false,
            updated_at: 0,
        }, GraphTask {
            id: "t2".into(),
            parent_id: None,
            title: "task2".into(),
            status: "done".into(),
            phase: None,
            notes: String::new(),
            verify_bit: true,
            updated_at: 0,
        }];
        let cap = build_seed_capsule(&m, &[], &sealed);
        assert!(cap.contains("task1 (t1) (UNVERIFIED)"));
        assert!(cap.contains("task2 (t2) (verified)"));
    }

    #[test]
    fn seed_capsule_includes_human_gates_when_present() {
        let mut m = sample_mission();
        m.human_gates = vec!["review API".into()];
        let cap = build_seed_capsule(&m, &[], &[]);
        assert!(cap.contains("**Human gates:**"));
        assert!(cap.contains("- review API"));
    }

    #[test]
    fn hash_is_stable_for_same_inputs() {
        let a = Mission::compute_hash("g", &["a".into()], &["n".into()]);
        let b = Mission::compute_hash("g", &["a".into()], &["n".into()]);
        let c = Mission::compute_hash("g2", &["a".into()], &["n".into()]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32); // 16 bytes hex
    }

    #[test]
    fn mission_roundtrip_json() {
        let dir = std::env::temp_dir().join(format!("koma-sdlc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = sample_mission();
        m.save(&dir).unwrap();
        let loaded = Mission::load(&dir).unwrap();
        assert_eq!(loaded.goal, m.goal);
        assert!(loaded.approved);
        assert_eq!(loaded.hash, m.hash);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
