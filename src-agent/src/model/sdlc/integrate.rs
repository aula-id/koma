//! SDLC integrate: merge the mission branch back to the primary workdir.
//!
//! Called by `mission_integrate` interception. Never force-pushes. Dirty main
//! → leave branch with instructions. Branch-only cannot bypass evidence gates
//! (those are enforced by the caller before this runs).

use std::path::Path;
use std::process::Command;

use super::Mission;

/// Result of an integrate attempt.
#[derive(Debug, Clone)]
pub struct IntegrateResult {
    /// Human-readable status message.
    pub message: String,
    /// Whether the merge succeeded (phase → done).
    pub success: bool,
}

/// Try to integrate the mission branch into the primary workdir.
///
/// Logic:
/// 1. Read `mission.branch`.
/// 2. Check `git status --porcelain` for dirty.
/// 3. If dirty OR `force_branch_only` → do NOT merge; return instructions.
/// 4. If clean: try `git merge --ff-only <branch>`; on conflict abort and
///    leave the branch ready for manual resolution.
/// 5. On success set mission.phase = "done" (caller).
pub fn try_integrate(
    primary_workdir: &Path,
    mission: &Mission,
    force_branch_only: bool,
) -> IntegrateResult {
    let branch = match &mission.branch {
        Some(b) => b.clone(),
        None => {
            return IntegrateResult {
                message: "error: mission has no branch set".to_string(),
                success: false,
            };
        }
    };

    if force_branch_only {
        return IntegrateResult {
            message: format!(
                "Branch `{branch}` left ready for manual integration (force_branch_only)."
            ),
            success: false,
        };
    }

    // Check for dirty working tree on primary.
    let dirty = is_dirty(primary_workdir);
    if dirty {
        return IntegrateResult {
            message: format!(
                "Main worktree is dirty — cannot merge cleanly. \
                 Mission branch `{branch}` is ready; merge it manually when the \
                 working tree is clean, or push it and open a PR."
            ),
            success: false,
        };
    }

    // Try fast-forward merge.
    let output = Command::new("git")
        .args(["merge", "--ff-only", &branch])
        .current_dir(primary_workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            IntegrateResult {
                message: format!("Integrated `{branch}` into main via fast-forward.\n{stdout}"),
                success: true,
            }
        }
        Ok(o) => {
            // Non-ff: try a regular merge.
            let output2 = Command::new("git")
                .args(["merge", &branch])
                .current_dir(primary_workdir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();
            match output2 {
                Ok(o2) if o2.status.success() => {
                    let stdout = String::from_utf8_lossy(&o2.stdout);
                    IntegrateResult {
                        message: format!("Integrated `{branch}` into main via merge.\n{stdout}"),
                        success: true,
                    }
                }
                _ => {
                    // Conflict — abort merge.
                    let _ = Command::new("git")
                        .args(["merge", "--abort"])
                        .current_dir(primary_workdir)
                        .output();
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    IntegrateResult {
                        message: format!(
                            "Merge conflict — aborted. Branch `{branch}` is ready; \
                             resolve conflicts manually.\n{stderr}"
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
    use crate::model::sdlc::Mission;
    use std::process::Command;

    fn sample(branch: &str) -> Mission {
        let gh = Some("g".into());
        let hash = Mission::compute_contract_hash_full(
            "g",
            &["a".into()],
            &[],
            "express",
            &[],
            &[],
            &[],
            "",
            gh.as_deref(),
            Some("wt"),
            Some(branch),
            Some("/tmp/x"),
        );
        Mission {
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
        let m = sample("feat/x");
        let r = try_integrate(std::path::Path::new("."), &m, true);
        assert!(!r.success);
        assert!(
            r.message.contains("left ready") && r.message.contains("force_branch_only"),
            "unexpected: {}",
            r.message
        );
    }

    #[test]
    fn missing_branch_fails() {
        let mut m = sample("x");
        m.branch = None;
        let r = try_integrate(std::path::Path::new("."), &m, false);
        assert!(!r.success);
        assert!(r.message.contains("no branch"));
    }

    #[test]
    fn temp_git_ff_integrate() {
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
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        run(&["checkout", "-b", "sdlc/feat"]);
        std::fs::write(root.join("b.txt"), "b").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "feat"]);
        run(&["checkout", "main"]);

        let m = sample("sdlc/feat");
        let r = try_integrate(&root, &m, false);
        assert!(r.success, "{}", r.message);
        let _ = std::fs::remove_dir_all(&root);
    }
}
