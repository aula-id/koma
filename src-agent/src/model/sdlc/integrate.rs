//! SDLC integrate: merge the mission branch back to the primary workdir.
//!
//! Called by `mission_integrate` interception. Never force-pushes. Dirty main
//! → leave branch with instructions.

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
///    return branch_ready.
/// 5. On success set mission.phase = "done".
pub fn try_integrate(primary_workdir: &Path, mission: &Mission) -> IntegrateResult {
    let branch = match &mission.branch {
        Some(b) => b.clone(),
        None => {
            return IntegrateResult {
                message: "error: mission has no branch set".to_string(),
                success: false,
            };
        }
    };

    // Check for dirty working tree.
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
                message: format!(
                    "Integrated `{branch}` into main via fast-forward.\n{stdout}"
                ),
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
                        message: format!(
                            "Integrated `{branch}` into main via merge.\n{stdout}"
                        ),
                        success: true,
                    }
                }
                _ => {
                    // Conflict — abort merge.
                    let _ = Command::new("git")
                        .args(["merge", "--abort"])
                        .current_dir(primary_workdir)
                        .output();
                    let stderr = String::from_utf8_lossy(
                        &o.stderr,
                    );
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
