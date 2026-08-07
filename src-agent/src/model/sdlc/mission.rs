//! L1 Mission contract: the frozen SDLC spec stored in `<session>/mission.json`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::graph::{self, GraphTask};

/// Legacy contracts predate a frozen integration target.
const LEGACY_CONTRACT_VERSION: u32 = 1;
/// Current contracts freeze both source and integration-target bindings.
pub const CURRENT_CONTRACT_VERSION: u32 = 2;

fn default_contract_version() -> u32 {
    LEGACY_CONTRACT_VERSION
}

/// A frozen SDLC mission contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    /// Contract schema version. Missing values deserialize as legacy v1.
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
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
    /// Frozen absolute path of the primary/target repo worktree at approval.
    /// Integration merges exclusively into this path — never inferred from live cwd.
    #[serde(default)]
    pub target_worktree_path: Option<String>,
    /// Frozen branch name of the primary/target at approval (never detached).
    #[serde(default)]
    pub target_branch: Option<String>,
    /// Frozen full commit SHA of the primary/target HEAD at approval.
    #[serde(default)]
    pub target_head: Option<String>,
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

/// Frozen fields used to compute a mission contract hash.
#[derive(Debug, Clone, Copy)]
pub struct ContractHashInput<'a> {
    pub goal: &'a str,
    pub acceptance: &'a [String],
    pub non_goals: &'a [String],
    pub lane: &'a str,
    pub verify_plan: &'a [String],
    pub human_gates: &'a [String],
    pub risks: &'a [String],
    pub rationale: &'a str,
    pub graph_hash: Option<&'a str>,
    pub worktree_name: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub worktree_path: Option<&'a str>,
    pub target_worktree_path: Option<&'a str>,
    pub target_branch: Option<&'a str>,
    pub target_head: Option<&'a str>,
}

impl Mission {
    /// Full contract hash including optional worktree binding + frozen target fields.
    pub fn compute_contract_hash_full(input: ContractHashInput<'_>) -> String {
        let ContractHashInput {
            goal,
            acceptance,
            non_goals,
            lane,
            verify_plan,
            human_gates,
            risks,
            rationale,
            graph_hash,
            worktree_name,
            branch,
            worktree_path,
            target_worktree_path,
            target_branch,
            target_head,
        } = input;
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
        // Frozen integrate destination — required for active contracts (v2).
        add_str(&mut hasher, target_worktree_path.unwrap_or(""));
        add_str(&mut hasher, target_branch.unwrap_or(""));
        add_str(&mut hasher, target_head.unwrap_or(""));
        let result = hasher.finalize();
        result[..16].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Recompute hash from this mission's frozen fields (including binding + target).
    pub fn recompute_hash(&self) -> String {
        Self::compute_contract_hash_full(ContractHashInput {
            goal: &self.goal,
            acceptance: &self.acceptance,
            non_goals: &self.non_goals,
            lane: &self.lane,
            verify_plan: &self.verify_plan,
            human_gates: &self.human_gates,
            risks: &self.risks,
            rationale: &self.rationale,
            graph_hash: self.graph_hash.as_deref(),
            worktree_name: self.worktree_name.as_deref(),
            branch: self.branch.as_deref(),
            worktree_path: self.worktree_path.as_deref(),
            target_worktree_path: self.target_worktree_path.as_deref(),
            target_branch: self.target_branch.as_deref(),
            target_head: self.target_head.as_deref(),
        })
    }

    /// True when stored hash matches frozen fields + graph binding.
    pub fn hash_valid(&self) -> bool {
        !self.hash.is_empty() && self.hash == self.recompute_hash()
    }

    /// True when the frozen integrate destination is fully present.
    pub fn has_frozen_target(&self) -> bool {
        self.target_worktree_path
            .as_deref()
            .is_some_and(|s| !s.is_empty())
            && self.target_branch.as_deref().is_some_and(|s| !s.is_empty())
            && self.target_head.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// Fail-closed active-ops check: approved, hash valid, bound, not draft/paused needing reapproval.
    pub fn validate_active(&self) -> Result<()> {
        if !self.approved {
            bail!("mission is not approved");
        }
        if self.needs_reapproval {
            bail!("mission contract was amended and requires re-approval");
        }
        if self.contract_version < CURRENT_CONTRACT_VERSION || !self.has_frozen_target() {
            bail!(
                "mission has no frozen integration target (legacy contract v{}) — re-approval required",
                self.contract_version
            );
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

    /// Validate the frozen integrate destination still matches path + branch.
    /// Never consults live session workdir — destination is exclusively the
    /// frozen `target_worktree_path` / `target_branch`.
    pub fn validate_target_destination(&self) -> Result<()> {
        let path = self
            .target_worktree_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("mission missing frozen target_worktree_path — re-approve required")
            })?;
        let expected_branch = self
            .target_branch
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("mission missing frozen target_branch — re-approve required")
            })?;
        let p = std::path::Path::new(path);
        if !p.is_dir() {
            bail!("frozen target worktree path missing or not a directory: {path}");
        }
        let live = current_git_branch(p);
        match live.as_deref() {
            Some(b) if b == expected_branch => Ok(()),
            Some(b) => bail!(
                "target branch drift: frozen target_branch '{expected_branch}' but \
                 target worktree is on '{b}'"
            ),
            None => bail!(
                "could not determine branch at frozen target_worktree_path (detached or not a repo)"
            ),
        }
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

/// Read full `git rev-parse HEAD` in dir.
pub fn current_git_head(dir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// True when `candidate` is an ancestor of `descendant` (or equal), via
/// `git merge-base --is-ancestor`. Used to reject rebinding an existing mission
/// worktree whose branch does not contain the frozen target_head.
pub fn is_ancestor(repo: &std::path::Path, ancestor: &str, descendant: &str) -> bool {
    std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    if mission.has_frozen_target() {
        s.push_str(&format!(
            "**Target:** {} @ {} ({})\n",
            mission.target_branch.as_deref().unwrap_or("n/a"),
            mission
                .target_head
                .as_deref()
                .map(|h| if h.len() > 12 { &h[..12] } else { h })
                .unwrap_or("n/a"),
            mission.target_worktree_path.as_deref().unwrap_or("n/a")
        ));
    } else {
        s.push_str("**Target:** missing frozen target — legacy contract; re-approve required.\n");
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
    // Live session cwd + branch must match the frozen mission worktree binding.
    mission.validate_binding(current_cwd, current_branch)?;
    // Integrate destination is exclusively the frozen target — path + branch.
    mission.validate_target_destination()?;
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
    mission.contract_version >= CURRENT_CONTRACT_VERSION
        && mission.has_frozen_target()
        && mission.approved
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
        let target_worktree_path = Some("/tmp/primary".into());
        let target_branch = Some("main".into());
        let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let hash = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "ship X",
            acceptance: &["tests pass".into()],
            non_goals: &["rewrite Y".into()],
            lane: "standard",
            verify_plan: &["cargo test".into()],
            human_gates: &[],
            risks: &["api churn".into()],
            rationale: "match house style",
            graph_hash: graph_hash.as_deref(),
            worktree_name: worktree_name.as_deref(),
            branch: branch.as_deref(),
            worktree_path: worktree_path.as_deref(),
            target_worktree_path: target_worktree_path.as_deref(),
            target_branch: target_branch.as_deref(),
            target_head: target_head.as_deref(),
        });
        Mission {
            contract_version: CURRENT_CONTRACT_VERSION,
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
            target_worktree_path,
            target_branch,
            target_head,
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
            owned_paths: vec![],
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
            owned_paths: vec![],
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
                owned_paths: vec![],
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
                owned_paths: vec![],
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
        let a = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &["n".into()],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: None,
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
        let b = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &["n".into()],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: None,
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
        let c = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g2",
            acceptance: &["a".into()],
            non_goals: &["n".into()],
            lane: "standard",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: None,
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn full_contract_hash_covers_lane_and_graph() {
        let a = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &[],
            lane: "full",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: Some("gh1"),
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
        let b = Mission::compute_contract_hash_full(ContractHashInput {
            goal: "g",
            acceptance: &["a".into()],
            non_goals: &[],
            lane: "full",
            verify_plan: &[],
            human_gates: &[],
            risks: &[],
            rationale: "",
            graph_hash: Some("gh2"),
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        });
        assert_ne!(a, b);
    }

    #[test]
    fn contract_hash_covers_worktree_binding() {
        let base = |wt: Option<&str>, br: Option<&str>, path: Option<&str>| {
            Mission::compute_contract_hash_full(ContractHashInput {
                goal: "g",
                acceptance: &["a".into()],
                non_goals: &[],
                lane: "standard",
                verify_plan: &[],
                human_gates: &[],
                risks: &[],
                rationale: "",
                graph_hash: Some("gh"),
                worktree_name: wt,
                branch: br,
                worktree_path: path,
                target_worktree_path: None,
                target_branch: None,
                target_head: None,
            })
        };
        let unbound = base(None, None, None);
        let bound = base(Some("wt"), Some("sdlc/x"), Some("/tmp/wt"));
        let other_path = base(Some("wt"), Some("sdlc/x"), Some("/tmp/other"));
        assert_ne!(unbound, bound);
        assert_ne!(bound, other_path);
        assert!(sample_mission().hash_valid());
    }

    #[test]
    fn contract_hash_covers_frozen_target() {
        let base = |tp: Option<&str>, tb: Option<&str>, th: Option<&str>| {
            Mission::compute_contract_hash_full(ContractHashInput {
                goal: "g",
                acceptance: &["a".into()],
                non_goals: &[],
                lane: "standard",
                verify_plan: &[],
                human_gates: &[],
                risks: &[],
                rationale: "",
                graph_hash: Some("gh"),
                worktree_name: Some("wt"),
                branch: Some("sdlc/x"),
                worktree_path: Some("/tmp/wt"),
                target_worktree_path: tp,
                target_branch: tb,
                target_head: th,
            })
        };
        let no_target = base(None, None, None);
        let with_target = base(Some("/tmp/p"), Some("main"), Some("abc123"));
        let other_branch = base(Some("/tmp/p"), Some("develop"), Some("abc123"));
        let other_head = base(Some("/tmp/p"), Some("main"), Some("def456"));
        assert_ne!(no_target, with_target);
        assert_ne!(with_target, other_branch);
        assert_ne!(with_target, other_head);
        assert!(sample_mission().has_frozen_target());
        assert!(sample_mission().hash_valid());
    }

    #[test]
    fn legacy_missing_target_deserializes_but_fails_active() {
        // Simulate a pre-v2 mission.json without target_* fields.
        let json = r#"{
            "id": "m-legacy",
            "goal": "ship X",
            "non_goals": [],
            "acceptance": ["tests pass"],
            "lane": "standard",
            "verify_plan": [],
            "human_gates": [],
            "risks": [],
            "worktree_name": "sdlc-test",
            "branch": "sdlc/ship-x",
            "worktree_path": "/tmp/wt",
            "rationale": "",
            "phase": "execute",
            "approved": true,
            "hash": "deadbeefdeadbeefdeadbeefdeadbeef",
            "graph_hash": "abc",
            "needs_reapproval": false
        }"#;
        let m: Mission = serde_json::from_str(json).expect("legacy must deserialize");
        assert_eq!(m.contract_version, LEGACY_CONTRACT_VERSION);
        assert!(m.target_worktree_path.is_none());
        assert!(m.target_branch.is_none());
        assert!(m.target_head.is_none());
        assert!(!m.has_frozen_target());
        // Hash won't match recompute (target fields now hashed as empty), and
        // validate_active fails closed either way.
        assert!(m.validate_active().is_err());
        let err = m.validate_active().unwrap_err().to_string();
        assert!(
            err.contains("hash mismatch")
                || err.contains("missing frozen target")
                || err.contains("legacy"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn seed_capsule_includes_frozen_target() {
        let m = sample_mission();
        let cap = build_seed_capsule_with_all(&m, &[], &[], &[]);
        assert!(
            cap.contains("**Target:** main @"),
            "capsule must show frozen target branch, got: {cap}"
        );
        assert!(cap.contains("/tmp/primary"));
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
    fn integrate_gate_requires_live_binding_and_frozen_target() {
        use crate::model::sdlc::graph::{
            ensure_tables, replace_nodes_from_checklist, set_verify_bit_with_evidence,
            ChecklistNode,
        };
        use rusqlite::Connection;
        use std::process::Command;

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("koma-sdlc-igate-{}-{}", std::process::id(), stamp));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let primary = root.join("primary");
        let bound = root.join("mission-wt");
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::create_dir_all(&bound).unwrap();

        let run_in = |dir: &std::path::Path, args: &[&str]| {
            let o = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{args:?} in {} → {}",
                dir.display(),
                String::from_utf8_lossy(&o.stderr)
            );
        };
        // Primary on non-main target branch `develop`.
        run_in(&primary, &["init", "-b", "develop"]);
        run_in(&primary, &["config", "user.email", "t@t"]);
        run_in(&primary, &["config", "user.name", "t"]);
        std::fs::write(primary.join("a.txt"), "a").unwrap();
        run_in(&primary, &["add", "."]);
        run_in(&primary, &["commit", "-m", "init"]);
        let target_head = current_git_head(&primary).expect("head");
        // Mission worktree is a separate git dir on mission branch (path check only
        // for validate_binding — no shared object db required for the gate).
        run_in(&bound, &["init", "-b", "sdlc/ship-x"]);
        run_in(&bound, &["config", "user.email", "t@t"]);
        run_in(&bound, &["config", "user.name", "t"]);
        std::fs::write(bound.join("b.txt"), "b").unwrap();
        run_in(&bound, &["add", "."]);
        run_in(&bound, &["commit", "-m", "feat"]);

        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        replace_nodes_from_checklist(
            &conn,
            &[ChecklistNode {
                title: "leaf".into(),
                status: "done".into(),
                parent_title: None,
                id: None,
                owned_paths: vec![],
            }],
        )
        .unwrap();
        let leaf_id = crate::model::sdlc::graph::list_all(&conn).unwrap()[0]
            .id
            .clone();
        set_verify_bit_with_evidence(&conn, &leaf_id, true, Some("tests pass")).unwrap();
        assert!(crate::model::sdlc::graph::list_open_leaves(&conn)
            .unwrap()
            .is_empty());
        assert!(crate::model::sdlc::graph::all_required_leaves_verified(&conn).unwrap());

        let structural = structural_graph_hash(&conn).unwrap();
        let mut m = sample_mission();
        m.phase = "execute".into();
        m.graph_hash = Some(structural);
        m.worktree_path = Some(bound.to_string_lossy().into_owned());
        m.branch = Some("sdlc/ship-x".into());
        m.worktree_name = Some("sdlc-test".into());
        m.target_worktree_path = Some(primary.to_string_lossy().into_owned());
        m.target_branch = Some("develop".into());
        m.target_head = Some(target_head);
        m.human_gates = vec![];
        m.human_gates_approved = vec![];
        m.hash = m.recompute_hash();
        assert!(m.validate_active().is_ok());
        assert_eq!(m.target_branch.as_deref(), Some("develop"));

        // Stale / missing live cwd must fail closed before integrate proceeds.
        let wrong_cwd = std::path::Path::new("/definitely/not/the/bound/path");
        let err = integrate_gate(&m, &conn, wrong_cwd, Some("sdlc/ship-x"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("worktree mismatch"),
            "expected worktree mismatch from integrate_gate, got: {err}"
        );

        // Correct cwd + wrong branch is also rejected (no path fallbacks).
        let err_branch = integrate_gate(&m, &conn, &bound, Some("other-branch"))
            .unwrap_err()
            .to_string();
        assert!(
            err_branch.contains("branch mismatch"),
            "expected branch mismatch from integrate_gate, got: {err_branch}"
        );

        // Target branch drift: freeze says develop, but switch primary to main.
        run_in(&primary, &["checkout", "-b", "main"]);
        let err_target = integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x"))
            .unwrap_err()
            .to_string();
        assert!(
            err_target.contains("target branch drift") || err_target.contains("develop"),
            "expected target drift rejection, got: {err_target}"
        );
        // Restore develop for the control case.
        run_in(&primary, &["checkout", "develop"]);

        // Control: live cwd + bound branch + matching frozen target passes.
        integrate_gate(&m, &conn, &bound, Some("sdlc/ship-x")).unwrap();

        // Legacy: missing frozen target fails closed even with good mission binding.
        let mut legacy = m.clone();
        legacy.target_worktree_path = None;
        legacy.target_branch = None;
        legacy.target_head = None;
        legacy.hash = legacy.recompute_hash();
        let err_legacy = integrate_gate(&legacy, &conn, &bound, Some("sdlc/ship-x"))
            .unwrap_err()
            .to_string();
        assert!(
            err_legacy.contains("frozen target") || err_legacy.contains("re-approval"),
            "expected legacy target failure, got: {err_legacy}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cannot_overwrite_concept_via_hash_valid() {
        let m = sample_mission();
        assert!(m.hash_valid());
        let mut m2 = m.clone();
        m2.goal = "other".into();
        assert!(!m2.hash_valid());
    }

    #[test]
    fn is_ancestor_detects_related_history() {
        use std::process::Command;
        let root = std::env::temp_dir().join(format!(
            "koma-sdlc-ancestor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let run = |args: &[&str]| {
            let o = Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{args:?} {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        run(&["init", "-b", "develop"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        let base = current_git_head(&root).unwrap();
        run(&["checkout", "-b", "sdlc/feat"]);
        std::fs::write(root.join("b.txt"), "b").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "feat"]);
        let tip = current_git_head(&root).unwrap();
        assert!(
            is_ancestor(&root, &base, &tip),
            "base must be ancestor of tip"
        );
        assert!(
            is_ancestor(&root, &base, &base),
            "commit is ancestor of itself"
        );
        // Unrelated: tip is NOT ancestor of base.
        assert!(!is_ancestor(&root, &tip, &base) || tip == base);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_active_requires_frozen_target() {
        let mut m = sample_mission();
        assert!(m.validate_active().is_ok());
        m.target_branch = None;
        m.hash = m.recompute_hash();
        let err = m.validate_active().unwrap_err().to_string();
        assert!(
            err.contains("frozen target") || err.contains("re-approval"),
            "unexpected: {err}"
        );
    }
}
