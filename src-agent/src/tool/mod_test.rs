use super::*;

#[test]
fn plan_allowlist_blocks_mutating_and_offensive_tools() {
    assert!(!tool_allowed_in_plan("write"));
    assert!(!tool_allowed_in_plan("edit"));
    assert!(!tool_allowed_in_plan("delete"));
    assert!(!tool_allowed_in_plan("bash"));
    assert!(!tool_allowed_in_plan("web_download"));
    assert!(!tool_allowed_in_plan("remember"));
    assert!(!tool_allowed_in_plan("todo_write"));
    assert!(!tool_allowed_in_plan("git_worktree"));
}

#[test]
fn plan_allowlist_allows_read_only_and_reasoning_tools() {
    assert!(tool_allowed_in_plan("read"));
    assert!(tool_allowed_in_plan("grep"));
    assert!(tool_allowed_in_plan("git_operator"));
    assert!(tool_allowed_in_plan("task"));
    assert!(tool_allowed_in_plan("seqthink"));
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
