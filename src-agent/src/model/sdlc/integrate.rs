//! SDLC integrate: merge the mission branch back to the frozen target worktree.
//!
//! Called by `mission_integrate` interception. Never force-pushes. Dirty target
//! → leave branch with instructions. Branch-only cannot bypass evidence gates
//! (those are enforced by the caller before this runs). Destination is always
//! `mission.target_worktree_path` + `mission.target_branch` — never inferred
//! from live session workdir / workdir_saved.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::mission::Mission;

/// Result of an integrate attempt.
#[derive(Debug, Clone)]
pub struct IntegrateResult {
    /// Human-readable status message.
    pub message: String,
    /// Whether the merge succeeded (phase → done).
    pub success: bool,
}

/// Resolve the frozen integrate destination from the mission contract.
/// Fails closed when target fields are missing (legacy contracts).
pub fn frozen_target_workdir(mission: &Mission) -> Result<PathBuf, String> {
    let path = mission
        .target_worktree_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "error: mission missing frozen target_worktree_path — re-approve required".to_string()
        })?;
    let p = PathBuf::from(path);
    if !p.is_dir() {
        return Err(format!(
            "error: frozen target_worktree_path is missing or not a directory: {path}"
        ));
    }
    Ok(p)
}

/// Re-validate frozen target path + branch immediately before merge.
pub fn validate_target_immediately_before_merge(mission: &Mission) -> Result<(), String> {
    mission
        .validate_target_destination()
        .map_err(|e| format!("error: target validation failed before merge: {e}"))
}

/// Try to integrate the mission branch into the frozen target worktree.
///
/// Logic:
/// 1. Require frozen `target_worktree_path` + `target_branch` (never workdir_saved).
/// 2. Re-validate target path+branch immediately before merge.
/// 3. Read `mission.branch` (mission feature branch).
/// 4. Check `git status --porcelain` for dirty on the frozen target.
/// 5. If dirty OR `force_branch_only` → do NOT merge; return instructions.
/// 6. If clean: try `git merge --ff-only <branch>`; on conflict abort and
///    leave the branch ready for manual resolution.
/// 7. On success set mission.phase = "done" (caller).
pub fn try_integrate(mission: &Mission, force_branch_only: bool) -> IntegrateResult {
    let target_branch = match mission.target_branch.as_deref().filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => {
            return IntegrateResult {
                message: "error: mission missing frozen target_branch — re-approve required"
                    .to_string(),
                success: false,
            };
        }
    };

    let branch = match &mission.branch {
        Some(b) => b.clone(),
        None => {
            return IntegrateResult {
                message: "error: mission has no branch set".to_string(),
                success: false,
            };
        }
    };

    // force_branch_only never merges; still requires frozen target_branch name
    // for the status string, but does not touch the worktree.
    if force_branch_only {
        return IntegrateResult {
            message: format!(
                "Branch `{branch}` left ready for manual integration into \
                 `{target_branch}` (force_branch_only)."
            ),
            success: false,
        };
    }

    let primary_workdir = match frozen_target_workdir(mission) {
        Ok(p) => p,
        Err(message) => {
            return IntegrateResult {
                message,
                success: false,
            };
        }
    };

    if let Err(message) = validate_target_immediately_before_merge(mission) {
        return IntegrateResult {
            message,
            success: false,
        };
    }

    // Check for dirty working tree on frozen target.
    let dirty = is_dirty(&primary_workdir);
    if dirty {
        return IntegrateResult {
            message: format!(
                "Target worktree (`{target_branch}` at {}) is dirty — cannot merge cleanly. \
                 Mission branch `{branch}` is ready; merge it manually when the \
                 working tree is clean, or push it and open a PR.",
                primary_workdir.display()
            ),
            success: false,
        };
    }

    // Try fast-forward merge.
    let output = Command::new("git")
        .args(["merge", "--ff-only", &branch])
        .current_dir(&primary_workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            IntegrateResult {
                message: format!(
                    "Integrated `{branch}` into `{target_branch}` via fast-forward.\n{stdout}"
                ),
                success: true,
            }
        }
        Ok(o) => {
            // Non-ff: try a regular merge.
            let output2 = Command::new("git")
                .args(["merge", &branch])
                .current_dir(&primary_workdir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();
            match output2 {
                Ok(o2) if o2.status.success() => {
                    let stdout = String::from_utf8_lossy(&o2.stdout);
                    IntegrateResult {
                        message: format!(
                            "Integrated `{branch}` into `{target_branch}` via merge.\n{stdout}"
                        ),
                        success: true,
                    }
                }
                _ => {
                    // Conflict — abort merge.
                    let _ = Command::new("git")
                        .args(["merge", "--abort"])
                        .current_dir(&primary_workdir)
                        .output();
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    IntegrateResult {
                        message: format!(
                            "Merge conflict — aborted. Branch `{branch}` is ready for \
                             `{target_branch}`; resolve conflicts manually.\n{stderr}"
                        ),
                        success: false,
                    }
                }
            }
        }
        Err(e) => IntegrateResult {
            message: format!("error: git failed: {e}"),
            success: false,
        },
    }
}

/// Check if the git working tree has uncommitted changes.
fn is_dirty(dir: &Path) -> bool {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            !stdout.trim().is_empty()
        }
        Err(_) => true, // Assume dirty if git fails.
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::model::sdlc::mission;
    use crate::model::sdlc::Mission;
    use std::process::Command;

    fn sample(branch: &str, target_branch: &str) -> Mission {
        let gh = Some("g".into());
        let hash =
            Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
                goal: "g",
                acceptance: &["a".into()],
                non_goals: &[],
                lane: "express",
                verify_plan: &[],
                human_gates: &[],
                risks: &[],
                rationale: "",
                graph_hash: gh.as_deref(),
                worktree_name: Some("wt"),
                branch: Some(branch),
                worktree_path: Some("/tmp/x"),
                target_worktree_path: Some("/tmp/primary"),
                target_branch: Some(target_branch),
                target_head: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            });
        Mission {
            contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
            id: "m".into(),
            goal: "g".into(),
            non_goals: vec![],
            acceptance: vec!["a".into()],
            lane: "express".into(),
            verify_plan: vec![],
            human_gates: vec![],
            human_gates_approved: vec![],
            risks: vec![],
            worktree_name: Some("wt".into()),
            branch: Some(branch.into()),
            worktree_path: Some("/tmp/x".into()),
            target_worktree_path: Some("/tmp/primary".into()),
            target_branch: Some(target_branch.into()),
            target_head: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            rationale: String::new(),
            phase: "integrate".into(),
            approved: true,
            hash,
            graph_hash: gh,
            needs_reapproval: false,
            amendment_note: None,
        }
    }

    #[test]
    fn force_branch_only_is_not_success_leaves_branch_ready_message() {
        // force_branch_only short-circuits before path existence checks.
        let m = sample("feat/x", "develop");
        let r = try_integrate(&m, true);
        assert!(!r.success);
        assert!(
            r.message.contains("left ready")
                && r.message.contains("force_branch_only")
                && r.message.contains("develop"),
            "unexpected: {}",
            r.message
        );
    }

    #[test]
    fn missing_branch_fails() {
        let mut m = sample("x", "main");
        // Need a real dir so frozen target path check passes before branch check.
        let dir = std::env::temp_dir().join(format!(
            "koma-int-missing-br-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let o = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        m.target_worktree_path = Some(dir.to_string_lossy().into_owned());
        m.target_branch = Some("main".into());
        m.target_head = mission::current_git_head(&dir);
        m.branch = None;
        m.hash = m.recompute_hash();
        let r = try_integrate(&m, false);
        assert!(!r.success);
        assert!(r.message.contains("no branch"), "{}", r.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_frozen_target_fails_closed() {
        let mut m = sample("feat/x", "main");
        m.target_worktree_path = None;
        m.target_branch = None;
        m.target_head = None;
        m.hash = m.recompute_hash();
        let r = try_integrate(&m, false);
        assert!(!r.success);
        assert!(
            r.message.contains("target_worktree_path") || r.message.contains("target_branch"),
            "{}",
            r.message
        );
    }

    #[test]
    fn target_branch_drift_rejects_before_merge() {
        let root = std::env::temp_dir().join(format!(
            "koma-int-drift-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
                "{args:?} {:?}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        run(&["init", "-b", "develop"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        let head = mission::current_git_head(&root).unwrap();
        // Freeze target as develop, then switch to main → drift.
        run(&["checkout", "-b", "main"]);

        let mut m = sample("sdlc/feat", "develop");
        m.target_worktree_path = Some(root.to_string_lossy().into_owned());
        m.target_branch = Some("develop".into());
        m.target_head = Some(head);
        m.hash = m.recompute_hash();
        let r = try_integrate(&m, false);
        assert!(!r.success, "drift must reject");
        assert!(
            r.message.contains("target") || r.message.contains("develop"),
            "unexpected: {}",
            r.message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn temp_git_ff_integrate_into_non_main_target() {
        let root = std::env::temp_dir().join(format!(
            "koma-integrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
                "{args:?} {:?}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        // Non-main target branch.
        run(&["init", "-b", "develop"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        let head = mission::current_git_head(&root).unwrap();
        run(&["checkout", "-b", "sdlc/feat"]);
        std::fs::write(root.join("b.txt"), "b").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "feat"]);
        run(&["checkout", "develop"]);

        let mut m = sample("sdlc/feat", "develop");
        m.target_worktree_path = Some(root.to_string_lossy().into_owned());
        m.target_branch = Some("develop".into());
        m.target_head = Some(head);
        m.hash = m.recompute_hash();
        let r = try_integrate(&m, false);
        assert!(r.success, "{}", r.message);
        assert!(
            r.message.contains("develop") && !r.message.contains("into main via"),
            "status must name target branch, got: {}",
            r.message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn never_infers_destination_from_caller_path() {
        // API no longer accepts a primary_workdir argument — destination is
        // exclusively mission.target_worktree_path.
        let m = sample("feat/x", "release");
        assert_eq!(m.target_branch.as_deref(), Some("release"));
        let err = frozen_target_workdir(&m).unwrap_err();
        assert!(err.contains("target_worktree_path") || err.contains("not a directory"));
    }
}
