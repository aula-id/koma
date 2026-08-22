#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn porcelain_parser_maps_branched_worktrees_and_preserves_spaces() {
    let worktrees = parse_worktree_output(
        b"worktree /repo with spaces\0HEAD aaaa\0branch refs/heads/main\0\0\
          worktree /linked/topic tree\0HEAD bbbb\0branch refs/heads/topic\0locked reason with spaces\0\0\
          worktree /detached tree\0HEAD cccc\0detached\0\0\
          worktree /stale\n tree\0HEAD dddd\0branch refs/heads/stale\0prunable gitdir file points to non-existent location\0",
    );

    assert_eq!(worktrees.len(), 3);
    assert_eq!(
        worktrees.get("main").map(String::as_str),
        Some("/repo with spaces")
    );
    assert_eq!(
        worktrees.get("topic").map(String::as_str),
        Some("/linked/topic tree")
    );
    assert_eq!(
        worktrees.get("stale").map(String::as_str),
        Some("/stale\n tree")
    );
    assert!(!worktrees.contains_key("detached"));
}

#[test]
fn branch_output_uses_porcelain_occupancy_and_skips_remote_head() {
    let worktrees = parse_worktree_output(
        b"worktree /repo\0branch refs/heads/main\0\0worktree /other\0branch refs/heads/topic\0",
    );
    let refs = parse_branch_output(
        b"refs/heads/main\t*\nrefs/heads/topic\t \nrefs/heads/free\t \nrefs/remotes/origin/HEAD\t \nrefs/remotes/origin/main\t \nrefs/tags/v1\t \n",
        &worktrees,
    );
    assert_eq!(refs.len(), 5);
    assert!(refs[0].is_current);
    assert_eq!(refs[0].worktree_path.as_deref(), Some("/repo"));
    assert_eq!(refs[1].worktree_path.as_deref(), Some("/other"));
    assert_eq!(refs[2].worktree_path, None);
    assert_eq!(refs[3].kind, "remote");
    assert_eq!(refs[4].kind, "tag");
}

#[test]
fn local_branch_is_free_here_but_occupied_in_another_worktree() {
    let worktrees = parse_worktree_output(
        b"worktree /repo root\0branch refs/heads/main\0\0worktree /other tree\0branch refs/heads/topic\0",
    );

    assert_eq!(
        occupied_worktree_path(&worktrees, "main", std::path::Path::new("/repo root")),
        None
    );
    assert_eq!(
        occupied_worktree_path(&worktrees, "free", std::path::Path::new("/repo root")),
        None
    );
    assert_eq!(
        occupied_worktree_path(&worktrees, "topic", std::path::Path::new("/repo root")),
        Some("/other tree")
    );
}

#[test]
fn malformed_and_unknown_ref_records_are_ignored_safely() {
    let refs = parse_branch_output(
        b"garbage\nrefs/notes/x\t*\nrefs/heads/ok\n",
        &std::collections::HashMap::new(),
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "ok");
    assert!(!refs[0].is_current);
    assert_eq!(refs[0].worktree_path, None);
}
