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

// --- resolve_read skill-dir exemption tests ---

#[test]
fn resolve_read_allows_abs_path_under_active_skill_dir() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-test-allow-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("helper.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![tmp.clone()];
    let path = resolve_read(&workspaces, tmp.join("helper.md").to_str().unwrap(), None, &skill_dirs);
    assert!(path.is_ok(), "should allow read under active skill dir");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_denies_abs_path_under_inactive_skill_dir() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-test-deny-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    // Empty skill_dirs — skill is NOT active.
    let skill_dirs: Vec<PathBuf> = vec![];
    let result = resolve_read(&workspaces, tmp.join("secret.md").to_str().unwrap(), None, &skill_dirs);
    assert!(result.is_err(), "should deny read when skill dir is not active");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_skill_dir_escape_via_dotdot_is_denied() {
    let base = std::env::temp_dir().join(format!(
        "koma-skill-test-escape-{}",
        std::process::id()
    ));
    let skill_dir = base.join("skill");
    let outside = base.join("outside");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![skill_dir.clone()];
    // Attempt to read outside/secret.md via skill_dir/../outside/secret.md.
    let escaped = skill_dir
        .join("..")
        .join("outside")
        .join("secret.md");
    let result = resolve_read(
        &workspaces,
        escaped.to_str().unwrap(),
        None,
        &skill_dirs,
    );
    assert!(result.is_err(), "dotdot escape from skill dir must be denied");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resolve_write_still_denies_skill_dir() {
    // resolve() (used by write/edit/delete) must NOT have the skill dir exemption.
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-test-write-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let result = resolve(&workspaces, tmp.join("file.md").to_str().unwrap());
    assert!(result.is_err(), "resolve() must deny paths outside workspaces");

    let _ = std::fs::remove_dir_all(&tmp);
}
