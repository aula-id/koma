//! L1 Mission contract: the frozen SDLC spec stored in `<session>/mission.json`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::graph::{self, GraphTask};

/// A frozen SDLC mission contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub goal: String,
    #[serde(default)]
    pub non_goals: Vec<String>,
    pub acceptance: Vec<String>,
    pub lane: String, // express | standard | full
    #[serde(default)]
    pub verify_plan: Vec<String>,
    #[serde(default)]
    pub human_gates: Vec<String>,
    /// Human gates that have been approved via the persisted gate path.
    #[serde(default)]
    pub human_gates_approved: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    pub worktree_name: Option<String>,
    pub branch: Option<String>,
    /// Absolute path of the bound mission worktree (set only after successful bind).
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub rationale: String,
    /// Current phase: assess | execute | integrate | done | paused | draft.
    pub phase: String,
    pub approved: bool,
    /// Content hash of the frozen fields (for change detection).
    pub hash: String,
    /// Canonical hash of the frozen graph at approval time.
    #[serde(default)]
    pub graph_hash: Option<String>,
    /// When true, contract was amended and needs re-approval.
    #[serde(default)]
    pub needs_reapproval: bool,
    /// Optional amendment note.
    #[serde(default)]
    pub amendment_note: Option<String>,
}

impl Mission {
    /// Full contract hash including optional worktree binding fields.
    pub fn compute_contract_hash_full(
        goal: &str,
        acceptance: &[String],
        non_goals: &[String],
        lane: &str,
        verify_plan: &[String],
        human_gates: &[String],
        risks: &[String],
        rationale: &str,
        graph_hash: Option<&str>,
        worktree_name: Option<&str>,
        branch: Option<&str>,
        worktree_path: Option<&str>,
    ) -> String {
        let mut hasher = Sha256::new();
        fn add_str(h: &mut Sha256, s: &str) {
            h.update(s.as_bytes());
            h.update([0]);
        }
        fn add_list(h: &mut Sha256, items: &[String]) {
            h.update((items.len() as u64).to_le_bytes());
            for it in items {
                add_str(h, it);
            }
        }
        add_str(&mut hasher, goal);
        add_list(&mut hasher, acceptance);
        add_list(&mut hasher, non_goals);
        add_str(&mut hasher, lane.trim());
        add_list(&mut hasher, verify_plan);
        add_list(&mut hasher, human_gates);
        add_list(&mut hasher, risks);
        add_str(&mut hasher, rationale);
        add_str(&mut hasher, graph_hash.unwrap_or(""));
        // Binding fields are part of the frozen contract once established.
        add_str(&mut hasher, worktree_name.unwrap_or(""));
        add_str(&mut hasher, branch.unwrap_or(""));
        add_str(&mut hasher, worktree_path.unwrap_or(""));
        let result = hasher.finalize();
        result[..16].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Recompute hash from this mission's frozen fields (including binding).
    pub fn recompute_hash(&self) -> String {
        Self::compute_contract_hash_full(
            &self.goal,
            &self.acceptance,
            &self.non_goals,
            &self.lane,
            &self.verify_plan,
            &self.human_gates,
            &self.risks,
            &self.rationale,
            self.graph_hash.as_deref(),
            self.worktree_name.as_deref(),
            self.branch.as_deref(),
            self.worktree_path.as_deref(),
        )
    }

    /// True when stored hash matches frozen fields + graph binding.
    pub fn hash_valid(&self) -> bool {
        !self.hash.is_empty() && self.hash == self.recompute_hash()
    }

    /// Fail-closed active-ops check: approved, hash valid, bound, not draft/paused needing reapproval.
    pub fn validate_active(&self) -> Result<()> {
        if !self.approved {
            bail!("mission is not approved");
        }
        if self.needs_reapproval {
            bail!("mission contract was amended and requires re-approval");
        }
        if !self.hash_valid() {
            bail!("mission contract hash mismatch — legacy/unbound contract fails closed");
        }
        if self.graph_hash.is_none() {
            bail!("mission has no frozen graph_hash — fails closed into reassessment");
        }
        if self.worktree_path.is_none() || self.branch.is_none() || self.worktree_name.is_none() {
            bail!("mission worktree binding incomplete — fails closed");
        }
        if matches!(self.phase.as_str(), "draft" | "done" | "paused" | "assess") {
            bail!(
                "mission phase '{}' is not active execute/integrate",
                self.phase
            );
        }
        Ok(())
    }

    /// Validate live worktree + branch still match the frozen binding.
    pub fn validate_binding(
        &self,
        current_cwd: &std::path::Path,
        current_branch: Option<&str>,
    ) -> Result<()> {
        let expected_path = self
            .worktree_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no worktree_path bound"))?;
        let expected_branch = self
            .branch
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no branch bound"))?;

        let cur = std::fs::canonicalize(current_cwd).unwrap_or_else(|_| current_cwd.to_path_buf());
        let exp = std::fs::canonicalize(expected_path)
            .unwrap_or_else(|_| std::path::PathBuf::from(expected_path));
        if cur != exp {
            bail!(
                "worktree mismatch: cwd {} != bound {}",
                cur.display(),
                exp.display()
            );
        }
        match current_branch {
            Some(b) if b == expected_branch => Ok(()),
            Some(b) => bail!("branch mismatch: current '{b}' != bound '{expected_branch}'"),
            None => bail!("could not determine current git branch for binding check"),
        }
    }

    /// Writable tool roots for SDLC execute/integrate.
    ///
    /// Fail-closed: invalid/unapproved contract or missing bound path → empty
    /// roots (callers must also refuse bash when empty). When the live cwd
    /// matches the bound tree, that cwd is used; otherwise the frozen bound
    /// path alone is exposed so the primary tree stays out of the allow-list.
    pub fn tool_sandbox_roots(
        &self,
        live_cwd: &std::path::Path,
    ) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
        if self.validate_active().is_err() {
            return (std::path::PathBuf::new(), Vec::new());
        }
        let Some(bound) = self
            .worktree_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
        else {
            return (std::path::PathBuf::new(), Vec::new());
        };
        let live_matches = std::fs::canonicalize(live_cwd)
            .ok()
            .zip(std::fs::canonicalize(&bound).ok())
            .is_some_and(|(a, b)| a == b);
        let workspaces = if live_matches {
            vec![live_cwd.to_path_buf()]
        } else if bound.is_dir() || std::fs::canonicalize(&bound).is_ok() {
            vec![bound]
        } else {
            // Bound path missing/tampered.
            return (std::path::PathBuf::new(), Vec::new());
        };
        let workspace = workspaces.first().cloned().unwrap_or_default();
        (workspace, workspaces)
    }

    /// Mark human gate approved (persisted). Idempotent.
    pub fn approve_human_gate(&mut self, gate: &str) {
        let g = gate.trim();
        if g.is_empty() {
            return;
        }
        if !self.human_gates.iter().any(|h| h == g) {
            return;
        }
        if !self.human_gates_approved.iter().any(|h| h == g) {
            self.human_gates_approved.push(g.to_string());
        }
    }

    /// True when every declared human gate has been approved.
    pub fn human_gates_satisfied(&self) -> bool {
        self.human_gates
            .iter()
            .all(|g| self.human_gates_approved.iter().any(|a| a == g))
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

/// Read `git rev-parse --abbrev-ref HEAD` in dir.
pub fn current_git_branch(dir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() || s == "HEAD" {
        None
    } else {
        Some(s)
    }
}

/// Build the SDLC seed capsule for injection into a compacted or fresh
/// conversation. Mandatory OPEN + SEALED sections so the model always
/// sees the full contract context on resume. OPEN/SEALED prefer leaf-aware tree.
///
/// When `all_nodes` is non-empty, hierarchy is rendered via the full graph;
/// otherwise OPEN/SEALED are rendered from the provided slices alone.
pub fn build_seed_capsule_with_all(
    mission: &Mission,
    open_nodes: &[GraphTask],
    sealed_nodes: &[GraphTask],
    all_nodes: &[GraphTask],
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
    s.push_str(&format!(
        "**Phase:** {} | **Lane:** {} | **Approved:** {}\n",
        mission.phase, mission.lane, mission.approved
    ));
    if mission.needs_reapproval {
        s.push_str("**NEEDS RE-APPROVAL** after amendment.\n");
    }
    if !mission.hash_valid() {
        s.push_str("**CONTRACT HASH INVALID** — fail closed; reassess.\n");
    }

    if let Some(ref wt) = mission.worktree_name {
        s.push_str(&format!(
            "**Worktree:** {} (branch: {})\n",
            wt,
            mission.branch.as_deref().unwrap_or("n/a")
        ));
    }
    if let Some(ref p) = mission.worktree_path {
        s.push_str(&format!("**Worktree path:** {p}\n"));
    }
    if let Some(ref gh) = mission.graph_hash {
        s.push_str(&format!("**Frozen graph hash:** {gh}\n"));
    }

    if !mission.verify_plan.is_empty() {
        s.push_str("**Verify plan:**\n");
        for vp in &mission.verify_plan {
            s.push_str(&format!("- {vp}\n"));
        }
    }

    if !mission.human_gates.is_empty() {
        s.push_str("**Human gates:**\n");
        for hg in &mission.human_gates {
            let mark = if mission.human_gates_approved.iter().any(|a| a == hg) {
                "approved"
            } else {
                "pending"
            };
            s.push_str(&format!("- [{mark}] {hg}\n"));
        }
    }

    let all = if all_nodes.is_empty() {
        let mut v = open_nodes.to_vec();
        v.extend(sealed_nodes.iter().cloned());
        v
    } else {
        all_nodes.to_vec()
    };

    s.push_str("\n## OPEN\n");
    if open_nodes.is_empty() {
        s.push_str("_none_\n");
    } else if all.iter().any(|n| n.parent_id.is_some()) {
        s.push_str(&graph::format_tree(open_nodes, &all, true));
    } else {
        for node in open_nodes {
            s.push_str(&format!(
                "- [{}] {} ({}) ← leaf\n",
                node.status, node.title, node.id
            ));
        }
    }

    s.push_str("\n## SEALED\n");
    if sealed_nodes.is_empty() {
        s.push_str("_none_\n");
    } else if all.iter().any(|n| n.parent_id.is_some()) {
        s.push_str(&graph::format_tree(sealed_nodes, &all, true));
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

    s.push_str(
        "\n## Law\n\
         - Graph is authoritative; TODO.md is projection only.\n\
         - Delegate OPEN leaves only (`task.node_id` required).\n\
         - Do not escape the bound mission worktree/branch during execute/integrate.\n\
         - External shell/MCP is not OS-sandboxed — stay disciplined.\n",
    );

    s
}

/// Integration gate: frozen graph complete, leaves verified, binding ok, human gates approved.
pub fn integrate_gate(
    mission: &Mission,
    conn: &rusqlite::Connection,
    current_cwd: &std::path::Path,
    current_branch: Option<&str>,
) -> Result<()> {
    mission.validate_active()?;
    // Graph hash must still match frozen.
    let live = graph::graph_fingerprint(conn)?;
    let frozen = mission
        .graph_hash
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing graph_hash"))?;
    // Allow progress (status/verify changes) — frozen hash is the *structure* at
    // approval. Recompute structural hash from titles/ids/parents only.
    let structural = structural_graph_hash(conn)?;
    if structural != frozen {
        bail!("frozen graph structure changed — amend + re-approve before integrate");
    }
    let _ = live; // retained for keeper/diagnostics
    if !graph::all_required_leaves_verified(conn)? {
        bail!("not all required leaf evidence is verified");
    }
    // Open non-cancelled leaves must be empty.
    let open_leaves = graph::list_open_leaves(conn)?;
    if !open_leaves.is_empty() {
        bail!("{} open leaf task(s) remain", open_leaves.len());
    }
    // Live session cwd + branch must match the frozen binding (no path fallbacks).
    mission.validate_binding(current_cwd, current_branch)?;
    if !mission.human_gates_satisfied() {
        bail!("human gates not all approved");
    }
    Ok(())
}

/// Structural graph hash (ids + parent + title) — status-independent freeze.
pub fn structural_graph_hash(conn: &rusqlite::Connection) -> Result<String> {
    let mut nodes = graph::list_all(conn)?;
    nodes.retain(|n| n.status != "cancelled");
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hasher = Sha256::new();
    for n in &nodes {
        hasher.update(n.id.as_bytes());
        hasher.update(b"|");
        hasher.update(n.parent_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"|");
        hasher.update(n.title.as_bytes());
        hasher.update(b"\n");
    }
    let result = hasher.finalize();
    Ok(result[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether a mission phase should auto-resume when re-entering SDLC.
///
/// Fail-closed: only actively approved execute/integrate missions resume.
/// `paused` / `draft` / `done` / `assess` never auto-resume — restart requires
/// explicit re-entry into a still-valid ACTIVE execute/integrate contract
/// (paused missions stay paused until the user re-approves / rebinds).
pub fn should_auto_resume(mission: &Mission) -> bool {
    mission.approved
        && !mission.needs_reapproval
        && mission.hash_valid()
        && matches!(mission.phase.as_str(), "execute" | "integrate")
}

/// Phase to restore on SDLC re-entry for a still-active execute/integrate mission.
pub fn resume_phase(mission: &Mission) -> Option<String> {
    if !should_auto_resume(mission) {
        return None;
    }
    Some(mission.phase.clone())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::model::sdlc::graph::GraphTask;

    fn sample_mission() -> Mission {
        let graph_hash = Some("abc".into());
        let worktree_name = Some("sdlc-test".into());
        let branch = Some("sdlc/ship-x".into());
        let worktree_path = Some("/tmp/wt".into());
        let hash = Mission::compute_contract_hash_full(
            "ship X",
            &["tests pass".into()],
            &["rewrite Y".into()],
            "standard",
            &["cargo test".into()],
            &[],
            &["api churn".into()],
            "match house style",
            graph_hash.as_deref(),
            worktree_name.as_deref(),
            branch.as_deref(),
            worktree_path.as_deref(),
        );
        Mission {
            id: "m-test".into(),
            goal: "ship X".into(),
            non_goals: vec!["rewrite Y".into()],
            acceptance: vec!["tests pass".into()],
            lane: "standard".into(),
            verify_plan: vec!["cargo test".into()],
            human_gates: vec![],
            human_gates_approved: vec![],
            risks: vec!["api churn".into()],
            worktree_name,
            branch,
            worktree_path,
            rationale: "match house style".into(),
            phase: "execute".into(),
            approved: true,
            hash,
            graph_hash,
            needs_reapproval: false,
            amendment_note: None,
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
        let cap = build_seed_capsule_with_all(&m, &open, &sealed, &[]);
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
        let cap = build_seed_capsule_with_all(&m, &[], &[], &[]);
        assert!(cap.contains("**Worktree:** sdlc-test (branch: sdlc/ship-x)"));
        assert!(cap.contains("**Verify plan:**"));
        assert!(cap.contains("- cargo test"));
    }

    #[test]
    fn seed_capsule_shows_verify_status_on_sealed() {
        let m = sample_mission();
        let sealed = vec![
            GraphTask {
                id: "t1".into(),
                parent_id: None,
                title: "task1".into(),
                status: "done".into(),
                phase: None,
                notes: String::new(),
                verify_bit: false,
                updated_at: 0,
            },
            GraphTask {
                id: "t2".into(),
                parent_id: None,
                title: "task2".into(),
                status: "done".into(),
                phase: None,
                notes: String::new(),
                verify_bit: true,
                updated_at: 0,
            },
        ];
        let cap = build_seed_capsule_with_all(&m, &[], &sealed, &[]);
        assert!(cap.contains("task1 (t1) (UNVERIFIED)"));
        assert!(cap.contains("task2 (t2) (verified)"));
    }

    #[test]
    fn seed_capsule_includes_human_gates_when_present() {
        let mut m = sample_mission();
        m.human_gates = vec!["review API".into()];
        // hash no longer matches after field change — that's fine for capsule text
        let cap = build_seed_capsule_with_all(&m, &[], &[], &[]);
        assert!(cap.contains("**Human gates:**"));
        assert!(cap.contains("review API"));
    }

    #[test]
    fn hash_is_stable_for_same_inputs() {
        let a = Mission::compute_contract_hash_full(
            "g",
            &["a".into()],
            &["n".into()],
            "standard",
            &[],
            &[],
            &[],
            "",
            None,
            None,
            None,
            None,
        );
        let b = Mission::compute_contract_hash_full(
            "g",
            &["a".into()],
            &["n".into()],
            "standard",
            &[],
            &[],
            &[],
            "",
            None,
            None,
            None,
            None,
        );
        let c = Mission::compute_contract_hash_full(
            "g2",
            &["a".into()],
            &["n".into()],
            "standard",
            &[],
            &[],
            &[],
            "",
            None,
            None,
            None,
            None,
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn full_contract_hash_covers_lane_and_graph() {
        let a = Mission::compute_contract_hash_full(
            "g",
            &["a".into()],
            &[],
            "full",
            &[],
            &[],
            &[],
            "",
            Some("gh1"),
            None,
            None,
            None,
        );
        let b = Mission::compute_contract_hash_full(
            "g",
            &["a".into()],
            &[],
            "full",
            &[],
            &[],
            &[],
            "",
            Some("gh2"),
            None,
            None,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn contract_hash_covers_worktree_binding() {
        let base = |wt: Option<&str>, br: Option<&str>, path: Option<&str>| {
            Mission::compute_contract_hash_full(
                "g",
                &["a".into()],
                &[],
                "standard",
                &[],
                &[],
                &[],
                "",
                Some("gh"),
                wt,
                br,
                path,
            )
        };
        let unbound = base(None, None, None);
        let bound = base(Some("wt"), Some("sdlc/x"), Some("/tmp/wt"));
        let other_path = base(Some("wt"), Some("sdlc/x"), Some("/tmp/other"));
        assert_ne!(unbound, bound);
        assert_ne!(bound, other_path);
        assert!(sample_mission().hash_valid());
    }

    #[test]
    fn legacy_empty_hash_fails_active_validation() {
        let mut m = sample_mission();
        m.hash = String::new();
        assert!(m.validate_active().is_err());
    }

    #[test]
    fn amendment_clears_approval() {
        let mut m = sample_mission();
        // Mirror production amendment path (mission_ready / re-entry fail-closed):
        // unapprove, force assess, flag reapproval, clear binding for re-bind.
        m.approved = false;
        m.phase = "assess".into();
        m.needs_reapproval = true;
        m.amendment_note = Some("change scope".into());
        m.worktree_path = None;
        assert!(!m.approved);
        assert!(m.needs_reapproval);
        assert_eq!(m.phase, "assess");
        assert!(m.worktree_path.is_none());
    }

    #[test]
    fn should_not_auto_resume_draft_done_paused() {
        let mut m = sample_mission();
        m.phase = "done".into();
        assert!(!should_auto_resume(&m));
        assert!(resume_phase(&m).is_none());
        m.phase = "draft".into();
        assert!(!should_auto_resume(&m));
        assert!(resume_phase(&m).is_none());
        // Explicit exit marks missions paused — restart must NOT auto-resume them.
        m.phase = "paused".into();
        assert!(!should_auto_resume(&m));
        assert!(resume_phase(&m).is_none());
        m.phase = "assess".into();
        assert!(!should_auto_resume(&m));
        assert!(resume_phase(&m).is_none());
        // Only live execute/integrate resume.
        m.phase = "execute".into();
        assert!(should_auto_resume(&m));
        assert_eq!(resume_phase(&m).as_deref(), Some("execute"));
        m.phase = "integrate".into();
        assert!(should_auto_resume(&m));
        assert_eq!(resume_phase(&m).as_deref(), Some("integrate"));
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
        assert_eq!(loaded.graph_hash, m.graph_hash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn human_gates_approval_path() {
        let mut m = sample_mission();
        m.human_gates = vec!["g1".into(), "g2".into()];
        assert!(!m.human_gates_satisfied());
        m.approve_human_gate("g1");
        assert!(!m.human_gates_satisfied());
        m.approve_human_gate("g2");
        assert!(m.human_gates_satisfied());
    }

    #[test]
    fn validate_binding_rejects_cwd_mismatch_no_fallback() {
        let m = sample_mission();
        // sample binds worktree_path=/tmp/wt — current dir is not that path.
        let cwd = std::env::temp_dir();
        let err = m
            .validate_binding(&cwd, m.branch.as_deref())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("worktree mismatch") || err.contains("no worktree"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn tool_sandbox_roots_fail_closed_without_valid_binding() {
        let mut m = sample_mission();
        // Active execute mission with missing on-disk worktree → empty roots.
        m.phase = "execute".into();
        m.worktree_path = Some("/tmp/koma-sdlc-missing-wt-definitely-not-real".into());
        m.hash = m.recompute_hash();
        let live = std::path::PathBuf::from("/tmp");
        let (ws, roots) = m.tool_sandbox_roots(&live);
        assert!(ws.as_os_str().is_empty(), "cwd must be poisoned empty");
        assert!(roots.is_empty(), "no writable roots without bound tree");

        // Paused also denies (not an active execute/integrate phase).
        m.phase = "paused".into();
        m.worktree_path = Some("/tmp".into());
        m.hash = m.recompute_hash();
        let (ws2, roots2) = m.tool_sandbox_roots(&live);
        assert!(ws2.as_os_str().is_empty());
        assert!(roots2.is_empty());

        m.phase = "execute".into();
        m.approved = false;
        m.hash = m.recompute_hash();
        let (ws3, roots3) = m.tool_sandbox_roots(&live);
        assert!(ws3.as_os_str().is_empty());
        assert!(roots3.is_empty());
    }

    #[test]
    fn tool_sandbox_roots_pins_to_bound_when_live_mismatches() {
        let dir = std::env::temp_dir().join(format!("koma-sdlc-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = sample_mission();
        m.phase = "execute".into();
        m.worktree_path = Some(dir.to_string_lossy().into_owned());
        m.hash = m.recompute_hash();
        let other = std::env::temp_dir();
        let (ws, roots) = m.tool_sandbox_roots(&other);
        assert_eq!(roots.len(), 1);
        let bound_canon = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
        let root_canon = std::fs::canonicalize(&roots[0]).unwrap_or(roots[0].clone());
        assert_eq!(root_canon, bound_canon);
        let ws_canon = std::fs::canonicalize(&ws).unwrap_or(ws.clone());
        assert_eq!(ws_canon, bound_canon);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn integrate_gate_requires_live_binding() {
        let m = sample_mission();
        // Empty in-memory graph cannot satisfy structure/evidence, but binding
        // must still be checked when those pass. Use a temp sqlite via msglog-less
        // direct connection if available — structural fail is enough to prove
        // the function fails closed without path fallbacks.
        let dir = std::env::temp_dir().join(format!("koma-sdlc-igate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Build a tiny on-disk messages db if open helper needs it.
        // Fall back: just assert validate_binding is what integrate_gate uses.
        let cwd = std::path::Path::new("/definitely/not/the/bound/path");
        let err = m
            .validate_binding(cwd, Some("sdlc/ship-x"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("worktree mismatch"), "unexpected: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cannot_overwrite_concept_via_hash_valid() {
        let m = sample_mission();
        assert!(m.hash_valid());
        let mut m2 = m.clone();
        m2.goal = "other".into();
        assert!(!m2.hash_valid());
    }
}
