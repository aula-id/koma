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
    /// Current phase: assess | prepare | execute | integrate | done | paused | draft.
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
    /// Allow-listed SDLC phase transition. Idempotent when already `to`.
    pub fn try_transition(&mut self, to: &str) -> Result<()> {
        let from = self.phase.as_str();
        if from == to {
            return Ok(());
        }
        let allowed = matches!(
            (from, to),
            // Fail-closed rail: deny/amend/bind-fail/cleanup/re-entry → assess
            // (covers draft→assess and any other source)
            (_, "assess")
                | ("assess", "prepare")
                | ("prepare", "execute")
                | ("prepare", "paused")
                | ("paused", "prepare")
                | ("assess", "execute")
                | ("execute", "integrate")
                | ("integrate", "done")
                | ("execute", "paused")
                | ("integrate", "paused")
                | ("paused", "execute")
                | ("draft", "execute")
        );
        if !allowed {
            bail!("illegal SDLC phase transition: {from} → {to}");
        }
        self.phase = to.to_string();
        Ok(())
    }

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
        if matches!(
            self.phase.as_str(),
            "draft" | "done" | "paused" | "assess" | "prepare"
        ) {
            bail!(
                "mission phase '{}' is not active execute/integrate",
                self.phase
            );
        }
        Ok(())
    }

    /// Pre-transition check for `mission_prepare`: validates that the mission is
    /// approved, properly bound, and has a complete contract — without rejecting the
    /// `prepare` phase itself (unlike [`validate_active`](Self::validate_active),
    /// which is intentionally scoped to execute/integrate).
    pub fn validate_prepare_ready(&self) -> Result<()> {
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
        if !matches!(self.phase.as_str(), "prepare") {
            bail!(
                "mission phase '{}' is not prepare — cannot transition to execute",
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

/// Best-effort: capture the short (7-char) commit SHA from HEAD of `dir`.
/// Returns `None` when not a git repo or HEAD is unavailable.
pub fn capture_head_short_sha(dir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
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

/// Result of cleaning up a successfully integrated mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneCleanupOutcome {
    ResetToAssess,
}

/// Remove a completed mission's worktree and merged branch, then make its
/// persisted contract available for reassessment. This fails closed: cleanup
/// is permitted only for a valid `done` mission whose graph is fully verified
/// and whose mission branch is already contained in the frozen target branch.
pub fn cleanup_done_mission(session_dir: &std::path::Path) -> Result<DoneCleanupOutcome> {
    let mission =
        Mission::load(session_dir).ok_or_else(|| anyhow::anyhow!("no mission to clean up"))?;
    if mission.phase != "done" {
        bail!(
            "mission phase '{}' is not ready for done cleanup",
            mission.phase
        );
    }
    if !mission.approved || mission.needs_reapproval || !mission.hash_valid() {
        bail!("done mission contract is not valid for cleanup");
    }
    if mission.contract_version < CURRENT_CONTRACT_VERSION || !mission.has_frozen_target() {
        bail!("done mission has no frozen integration target");
    }

    let worktree_path = mission
        .worktree_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| anyhow::anyhow!("done mission has no bound worktree path"))?;
    let branch = mission
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .ok_or_else(|| anyhow::anyhow!("done mission has no bound branch"))?;
    let target_repo = mission
        .target_worktree_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("done mission has no frozen target path"))?;
    let target_branch = mission
        .target_branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .ok_or_else(|| anyhow::anyhow!("done mission has no frozen target branch"))?;

    let conn = crate::model::msglog::open(session_dir)?;
    graph::ensure_tables(&conn)?;
    if !graph::all_required_leaves_verified(&conn)? {
        bail!("cannot clean up done mission until every required leaf is verified");
    }
    if !is_ancestor(&target_repo, branch, target_branch) {
        bail!("cannot clean up done mission before branch '{branch}' is integrated into '{target_branch}'");
    }

    git_worktree_remove_checked(&target_repo, std::path::Path::new(worktree_path))?;
    git_branch_delete_checked(&target_repo, branch)?;
    reset_mission_to_assess_after_cleanup(session_dir, mission)?;
    Ok(DoneCleanupOutcome::ResetToAssess)
}

fn git_worktree_remove_checked(repo: &std::path::Path, worktree: &std::path::Path) -> Result<()> {
    // A prior interrupted cleanup may already have removed it; the branch and
    // mission reset checks below still make retrying safe.
    if !worktree.exists() {
        return Ok(());
    }
    let output = std::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            worktree.as_os_str().to_string_lossy().as_ref(),
        ])
        .current_dir(repo)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "could not remove mission worktree {}: {}",
        worktree.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn git_branch_delete_checked(repo: &std::path::Path, branch: &str) -> Result<()> {
    let exists = std::process::Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .current_dir(repo)
        .status()?
        .success();
    if !exists {
        return Ok(());
    }
    let output = std::process::Command::new("git")
        .args(["branch", "--delete", "--", branch])
        .current_dir(repo)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "could not delete merged mission branch '{branch}': {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn reset_mission_to_assess_after_cleanup(
    session_dir: &std::path::Path,
    mut mission: Mission,
) -> Result<()> {
    mission.try_transition("assess")?;
    mission.approved = false;
    mission.worktree_name = None;
    mission.branch = None;
    mission.worktree_path = None;
    mission.needs_reapproval = true;
    mission.hash = mission.recompute_hash();
    mission.save(session_dir)
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
    sealed_commit_shas: &std::collections::HashMap<String, Vec<String>>,
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
    } else if let Some(ref br) = mission.branch {
        s.push_str(&format!("**Branch intent:** {br}\n"));
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
        s.push_str(&graph::format_tree(open_nodes, &all, true, None));
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
        s.push_str(&graph::format_tree(
            sealed_nodes,
            &all,
            true,
            Some(sealed_commit_shas),
        ));
    } else {
        for node in sealed_nodes {
            let verify_mark = if node.verify_bit {
                let commit_suffix = sealed_commit_shas
                    .get(&node.id)
                    .and_then(|v| v.first())
                    .map(|sha| {
                        let short = if sha.len() > 7 { &sha[..7] } else { sha };
                        format!(" (commit: {short})")
                    })
                    .unwrap_or_default();
                format!("(verified){commit_suffix}")
            } else {
                "(UNVERIFIED)".to_string()
            };
            s.push_str(&format!(
                "- [done] {} ({}) {}\n",
                node.title, node.id, verify_mark
            ));
        }
    }

    s.push_str(
        "\n## Law\n\
         - Never force-push; plain push only the mission branch.\n\
         - One OPEN leaf claim at a time (main: checklist in_progress or task.node_id); seal only via mission_verify.\n\
         - Graph is authority; checklist cannot bare-done.\n\
         - Do not escape the bound mission worktree/branch during execute/integrate.\n\
         - No auto-commit; integrate needs clean mission WT + commits ahead.\n\
         - If the claimed leaf has owned_paths, stay inside them.\n\
         - Unsure: web_search → message_find → ask the user.\n",
    );

    s
}

/// Format the recent edit history section for the SDLC capsule.
pub fn format_edit_history_section(entries: &[graph::RecentEditEntry]) -> String {
    use super::graph::{EditAuditRecord, EditSummaryRecord};

    if entries.is_empty() {
        return String::new();
    }

    let mut s = String::from("\n## Recent activity (edit rail)\n");
    for entry in entries {
        match entry.kind.as_str() {
            "edit_summary" => {
                if let Ok(rec) = serde_json::from_str::<EditSummaryRecord>(&entry.detail) {
                    let paths_str = if rec.paths.len() <= 3 {
                        rec.paths.join(", ")
                    } else {
                        format!(
                            "{}, +{} more",
                            rec.paths[..2].join(", "),
                            rec.paths.len() - 2
                        )
                    };
                    let node_str = rec
                        .node_id
                        .as_deref()
                        .map(|n| format!(" [{n}]"))
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "-{node_str} {}\n  Files: {paths_str}\n",
                        rec.purpose
                    ));
                }
            }
            "edit_audit" => {
                if let Ok(rec) = serde_json::from_str::<EditAuditRecord>(&entry.detail) {
                    let node_str = rec
                        .node_id
                        .as_deref()
                        .map(|n| format!(" [{n}]"))
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "-{node_str} {} {} (summary pending)\n",
                        rec.tool, rec.path
                    ));
                }
            }
            _ => {}
        }
    }
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
    // Commit evidence gate: every sealed leaf must have at least one commit SHA
    // from its verify events, and each SHA must be reachable from the mission branch tip.
    // Only enforced when mission.branch is present and the worktree is a git repo.
    if let (Some(branch), Some(wt_path)) = (
        mission.branch.as_deref().filter(|s| !s.is_empty()),
        mission.worktree_path.as_deref().filter(|s| !s.is_empty()),
    ) {
        let wt = std::path::Path::new(wt_path);
        if wt.is_dir() && current_git_branch(wt).is_some() {
            let sealed = graph::list_sealed(conn)?;
            let mut leaves: Vec<&GraphTask> = Vec::new();
            for n in &sealed {
                if graph::is_leaf(conn, &n.id)? {
                    leaves.push(n);
                }
            }
            if !leaves.is_empty() {
                let node_ids: Vec<String> = leaves.iter().map(|n| n.id.clone()).collect();
                let commit_map = graph::latest_verified_commit_shas(conn, &node_ids)?;
                for leaf in &leaves {
                    let shas = commit_map
                        .get(&leaf.id)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    if shas.is_empty() {
                        bail!(
                            "sealed leaf '{}' ({}) has no commit evidence — \
                             run mission_verify with a commit",
                            leaf.title,
                            leaf.id
                        );
                    }
                    for sha in shas {
                        if !is_ancestor(wt, sha, branch) {
                            bail!(
                                "commit {sha} from sealed leaf '{}' ({}) is not reachable from \
                                 mission branch '{branch}' — re-verify with current branch state",
                                leaf.title,
                                leaf.id
                            );
                        }
                    }
                }
            }
        }
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
/// Fail-closed: only actively approved prepare/execute/integrate missions resume.
/// `paused` / `draft` / `done` / `assess` never auto-resume — restart requires
/// explicit re-entry into a still-valid ACTIVE prepare/execute/integrate contract
/// (paused missions stay paused until the user re-approves / rebinds).
pub fn should_auto_resume(mission: &Mission) -> bool {
    if mission.contract_version < CURRENT_CONTRACT_VERSION {
        return false;
    }
    if !mission.has_frozen_target() {
        return false;
    }
    if !mission.approved || mission.needs_reapproval || !mission.hash_valid() {
        return false;
    }
    match mission.phase.as_str() {
        // prepare doesn't need binding yet - it's established during prepare.
        "prepare" => true,
        // execute/integrate require a live binding (worktree_path + branch).
        "execute" | "integrate" => {
            mission
                .worktree_path
                .as_deref()
                .is_some_and(|s| !s.is_empty())
                && mission.branch.as_deref().is_some_and(|s| !s.is_empty())
        }
        _ => false,
    }
}

/// Phase to restore on SDLC re-entry for a still-active execute/integrate mission.
pub fn resume_phase(mission: &Mission) -> Option<String> {
    if !should_auto_resume(mission) {
        return None;
    }
    Some(mission.phase.clone())
}

#[cfg(test)]
#[path = "mission_test.rs"]
mod tests;
