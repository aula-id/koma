#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn plan_allowlist_blocks_mutating_and_offensive_tools() {
    assert!(!tool_allowed_in_plan("write"));
    assert!(!tool_allowed_in_plan("edit"));
    assert!(!tool_allowed_in_plan("delete"));
    assert!(!tool_allowed_in_plan("bash"));
    assert!(!tool_allowed_in_plan("web_download"));
    assert!(!tool_allowed_in_plan("remember"));
    assert!(!tool_allowed_in_plan("git_worktree"));
}

#[test]
fn sdlc_assess_allowlist_blocks_workspace_mutations() {
    assert!(!tool_allowed_in_sdlc_assess("write"));
    assert!(!tool_allowed_in_sdlc_assess("edit"));
    assert!(!tool_allowed_in_sdlc_assess("delete"));
    assert!(!tool_allowed_in_sdlc_assess("bash"));
    assert!(!tool_allowed_in_sdlc_assess("web_download"));
    assert!(!tool_allowed_in_sdlc_assess("git_worktree"));
    assert!(!tool_allowed_in_sdlc_assess("remember"));
}

#[test]
fn sdlc_assess_allowlist_keeps_mission_prep_tools() {
    assert!(tool_allowed_in_sdlc_assess("read"));
    assert!(tool_allowed_in_sdlc_assess("grep"));
    assert!(tool_allowed_in_sdlc_assess("glob"));
    assert!(tool_allowed_in_sdlc_assess("checklist"));
    assert!(tool_allowed_in_sdlc_assess("mission_ready"));
    assert!(tool_allowed_in_sdlc_assess("web_search"));
    assert!(tool_allowed_in_sdlc_assess("seqthink"));
}

#[test]
fn plan_allowlist_allows_read_only_and_reasoning_tools() {
    assert!(tool_allowed_in_plan("read"));
    assert!(tool_allowed_in_plan("grep"));
    assert!(tool_allowed_in_plan("git_operator"));
    assert!(tool_allowed_in_plan("task"));
    assert!(tool_allowed_in_plan("seqthink"));
    // The real tool name is `checklist` — Plan mode manages
    // the checklist through it (fully intercepted in `process_tools`, see
    // `approval.rs`), so it's allowed here at the tool-name level.
    assert!(tool_allowed_in_plan("checklist"));
    assert!(tool_allowed_in_plan("skill"));
}

#[test]
fn plan_git_subcommand_allows_read_only() {
    assert!(plan_git_subcommand_allowed("log"));
    assert!(plan_git_subcommand_allowed("status"));
    assert!(plan_git_subcommand_allowed("diff"));
}

#[test]
fn plan_git_subcommand_denies_mutating() {
    assert!(!plan_git_subcommand_allowed("commit"));
    assert!(!plan_git_subcommand_allowed("push"));
    assert!(!plan_git_subcommand_allowed("checkout"));
}

// --- SDLC assess git-form guards (gap 1) ---

#[test]
fn sdlc_assess_git_allows_safe_read_forms() {
    assert!(sdlc_assess_git_args_allowed(&["status"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["log", "--oneline", "-5"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "-vv"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "--show-current"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "--list", "feat/*"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "-v"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "show", "origin"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "get-url", "origin"]).is_ok());
    // Assess may create/checkout local branches (no force/discard).
    assert!(sdlc_assess_git_args_allowed(&["branch", "new-feature"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["checkout", "main"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["switch", "main"]).is_ok());
}

#[test]
fn sdlc_assess_git_rejects_mutating_branch_forms() {
    for args in [
        &["branch", "-d", "old"][..],
        &["branch", "-D", "old"][..],
        &["branch", "-m", "renamed"][..],
        &["branch", "--set-upstream-to=origin/main"][..],
        &["branch", "-u", "origin/main", "main"][..],
        &["branch", "--force", "x", "y"][..],
    ] {
        let err = sdlc_assess_git_args_allowed(args).unwrap_err();
        assert!(
            err.contains("branch") || err.contains("mutating"),
            "args={args:?} err={err}"
        );
    }
}

#[test]
fn sdlc_assess_git_rejects_mutating_remote_forms() {
    for args in [
        &["remote", "add", "origin", "https://example.com/r.git"][..],
        &["remote", "remove", "origin"][..],
        &["remote", "rm", "origin"][..],
        &["remote", "set-url", "origin", "https://example.com/n.git"][..],
        &["remote", "rename", "origin", "upstream"][..],
    ] {
        let err = sdlc_assess_git_args_allowed(args).unwrap_err();
        assert!(
            err.contains("remote") || err.contains("mutating"),
            "args={args:?} err={err}"
        );
    }
}

#[test]
fn sdlc_assess_git_rejects_non_readonly_subcommands() {
    assert!(sdlc_assess_git_args_allowed(&["commit", "-m", "x"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["push"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["checkout", "-f", "main"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["merge", "main"]).is_err());
}

// --- SDLC execute git confinement helpers (gap 2) ---

#[test]
fn sdlc_execute_git_rejects_cwd_override_and_branch_ops() {
    assert!(sdlc_execute_git_args_allowed(&["status"], None, true, "", Some("feat")).is_ok());
    assert!(sdlc_execute_git_args_allowed(&["add", "."], None, true, "", Some("feat")).is_ok());
    assert!(
        sdlc_execute_git_args_allowed(&["commit", "-m", "x"], None, true, "", Some("feat")).is_ok()
    );

    let err =
        sdlc_execute_git_args_allowed(&["status"], Some("/tmp/other"), true, "", Some("feat"))
            .unwrap_err();
    assert!(err.contains("cwd"), "{err}");

    let err = sdlc_execute_git_args_allowed(&["checkout", "main"], None, true, "", Some("feat"))
        .unwrap_err();
    assert!(err.contains("checkout"), "{err}");

    let err = sdlc_execute_git_args_allowed(&["switch", "main"], None, true, "", Some("feat"))
        .unwrap_err();
    assert!(err.contains("switch"), "{err}");

    let err =
        sdlc_execute_git_args_allowed(&["status"], None, false, "worktree mismatch", Some("feat"))
            .unwrap_err();
    assert!(err.contains("not live") || err.contains("binding"), "{err}");
}

#[test]
fn sdlc_git_force_push_denied_matrix() {
    assert!(sdlc_git_force_push_denied(&["push", "origin", "main"]).is_none());
    assert!(sdlc_git_force_push_denied(&["status"]).is_none());
    for args in [
        &["push", "--force"][..],
        &["push", "-f", "origin", "main"][..],
        &["push", "-uf", "origin", "main"][..],
        &["push", "--force-with-lease"][..],
        &["push", "--force-with-lease=origin/main"][..],
        &["push", "--delete", "origin", "old"][..],
        &["push", "-d", "origin", "old"][..],
        &["push", "origin", ":old-branch"][..],
    ] {
        let reason = sdlc_git_force_push_denied(args).expect("should deny");
        assert!(!reason.is_empty(), "{args:?}");
    }
    let err = sdlc_execute_git_args_allowed(
        &["push", "--force", "origin", "main"],
        None,
        true,
        "",
        Some("main"),
    )
    .unwrap_err();
    assert!(err.contains("Never force-push"), "{err}");
}

#[test]
fn sdlc_execute_git_push_mission_branch_matrix() {
    let mb = Some("feat/x");
    // force still denied
    assert!(sdlc_execute_git_args_allowed(
        &["push", "--force", "origin", "feat/x"],
        None,
        true,
        "",
        mb
    )
    .is_err());
    // wrong branch deny
    let err =
        sdlc_execute_git_args_allowed(&["push", "origin", "main"], None, true, "", mb).unwrap_err();
    assert!(
        err.contains("mission branch") || err.contains("feat/x"),
        "{err}"
    );
    // correct branch ok
    assert!(
        sdlc_execute_git_args_allowed(&["push", "origin", "feat/x"], None, true, "", mb).is_ok()
    );
    assert!(sdlc_execute_git_args_allowed(
        &["push", "origin", "refs/heads/feat/x"],
        None,
        true,
        "",
        mb
    )
    .is_ok());
    assert!(
        sdlc_execute_git_args_allowed(&["push", "origin", "HEAD:feat/x"], None, true, "", mb)
            .is_ok()
    );
    // bare push deny
    let err = sdlc_execute_git_args_allowed(&["push"], None, true, "", mb).unwrap_err();
    assert!(err.contains("bare") || err.contains("push"), "{err}");
    let err = sdlc_execute_git_args_allowed(&["push", "origin"], None, true, "", mb).unwrap_err();
    assert!(err.contains("bare") || err.contains("push"), "{err}");
    // missing branch deny
    let err = sdlc_execute_git_args_allowed(&["push", "origin", "feat/x"], None, true, "", None)
        .unwrap_err();
    assert!(
        err.contains("no bound branch") || err.contains("branch"),
        "{err}"
    );
}

// --- resolve_read skill-dir exemption tests ---

#[test]
fn resolve_read_allows_abs_path_under_active_skill_dir() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-allow-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("helper.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![tmp.clone()];
    let path = resolve_read(
        &workspaces,
        tmp.join("helper.md").to_str().unwrap(),
        None,
        &skill_dirs,
    );
    assert!(path.is_ok(), "should allow read under active skill dir");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_denies_abs_path_under_inactive_skill_dir() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-deny-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    // Empty skill_dirs — skill is NOT active.
    let skill_dirs: Vec<PathBuf> = vec![];
    let result = resolve_read(
        &workspaces,
        tmp.join("secret.md").to_str().unwrap(),
        None,
        &skill_dirs,
    );
    assert!(
        result.is_err(),
        "should deny read when skill dir is not active"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_skill_dir_escape_via_dotdot_is_denied() {
    let base = std::env::temp_dir().join(format!("koma-skill-test-escape-{}", std::process::id()));
    let skill_dir = base.join("skill");
    let outside = base.join("outside");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![skill_dir.clone()];
    // Attempt to read outside/secret.md via skill_dir/../outside/secret.md.
    let escaped = skill_dir.join("..").join("outside").join("secret.md");
    let result = resolve_read(&workspaces, escaped.to_str().unwrap(), None, &skill_dirs);
    assert!(
        result.is_err(),
        "dotdot escape from skill dir must be denied"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resolve_write_still_denies_skill_dir() {
    // resolve() (used by write/edit/delete) must NOT have the skill dir exemption.
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-write-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let result = resolve(&workspaces, tmp.join("file.md").to_str().unwrap());
    assert!(
        result.is_err(),
        "resolve() must deny paths outside workspaces"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_allows_scratch_by_default_non_sdlc() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-scratch-ok-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("note.txt");
    // Path need not exist yet — resolve accepts absolute under scratch.
    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace-x")];
    let ok = resolve(&workspaces, target.to_str().expect("utf8 path"));
    assert!(
        ok.is_ok(),
        "default resolve must keep scratch exemption: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_read_allows_canonicalized_scratch_path() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-read-scratch-ok-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("note.txt");
    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace-read")];
    let ok = resolve_read(&workspaces, target.to_str().expect("utf8 path"), None, &[]);
    assert!(
        ok.is_ok(),
        "resolve_read must keep scratch exemption after canonicalization: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_in_rejects_scratch_when_bypass_disabled() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-scratch-deny-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("escape.txt");
    let workspaces = vec![std::env::temp_dir().join("koma-bound-worktree-fake")];
    let denied = resolve_in(&workspaces, target.to_str().expect("utf8 path"), false);
    assert!(
        denied.is_err(),
        "SDLC execute/integrate must not write via scratch root: {denied:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_in_allows_path_inside_bound_worktree_without_scratch() {
    let bound = std::env::temp_dir().join(format!("koma-bound-wt-{}", std::process::id()));
    std::fs::create_dir_all(&bound).unwrap();
    let target = bound.join("src").join("lib.rs");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "fn main() {}").unwrap();
    let workspaces = vec![bound.clone()];
    let ok = resolve_in(&workspaces, target.to_str().expect("utf8 path"), false);
    assert!(
        ok.is_ok(),
        "bound worktree absolute path must still resolve: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&bound);
}
