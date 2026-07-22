#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn status_hints_gone_counts_right() {
    let raw = "\
On branch main
Your branch is ahead of 'origin/main' by 2 commits.
  (use \"git push\" to publish your local commits)

Changes to be committed:
  (use \"git restore --staged <file>...\" to unstage)
        modified:   src/main.rs
        new file:   src/new.rs

Changes not staged for commit:
  (use \"git add <file>...\" to update what will be committed)
  (use \"git restore <file>...\" to discard changes in working directory)
        modified:   README.md

Untracked files:
  (use \"git add <file>...\" to include in what will be committed)
        scratch.txt

no changes added to commit (use \"git add\" and/or \"git commit -a\")
";
    let outcome = try_filter("git status", raw, Some(0)).expect("should filter");
    assert!(!outcome.text.contains("(use \"git"));
    assert!(outcome.text.contains("staged (2):"));
    assert!(outcome.text.contains("modified (1):"));
    assert!(outcome.text.contains("untracked (1):"));
    assert!(outcome.text.contains("src/main.rs"));
    assert!(outcome.text.contains("scratch.txt"));
}

#[test]
fn status_porcelain_returns_none() {
    assert!(try_filter("git status --porcelain", "## main\n", Some(0)).is_none());
    assert!(try_filter("git status -s", "M  foo\n", Some(0)).is_none());
}

#[test]
fn log_default_three_commits() {
    let raw = "\
commit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
Author: Alice <alice@example.com>
Date:   Mon Jan 5 10:00:00 2026 +0000

    First commit message

commit bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
Author: Bob <bob@example.com>
Date:   Sun Jan 4 09:00:00 2026 +0000

    Second commit message

commit cccccccccccccccccccccccccccccccccccccccc
Author: Carol <carol@example.com>
Date:   Sat Jan 3 08:00:00 2026 +0000

    Third commit message
";
    let outcome = try_filter("git log", raw, Some(0)).expect("should filter");
    assert_eq!(outcome.text.lines().count(), 3);
    assert!(outcome.text.contains("First commit message"));
    assert!(outcome.text.contains("Alice"));
}

#[test]
fn log_with_pretty_flag_returns_none() {
    assert!(try_filter("git log --oneline", "abc123 msg\n", Some(0)).is_none());
}

#[test]
fn log_fatal_error_with_nonzero_exit_returns_none() {
    let raw = "fatal: your current branch 'main' does not have any commits yet\n";
    assert!(try_filter("git log", raw, Some(128)).is_none());
}

#[test]
fn log_commit_free_raw_with_zero_exit_returns_none() {
    // Defense in depth: even if exit_code somehow reports success, never
    // collapse output that contains zero parsed commit blocks.
    let raw = "fatal: your current branch 'main' does not have any commits yet\n";
    assert!(try_filter("git log", raw, Some(0)).is_none());
}

#[test]
fn status_nonzero_exit_returns_none() {
    let raw = "fatal: not a git repository\n";
    assert!(try_filter("git status", raw, Some(128)).is_none());
}

#[test]
fn diff_under_threshold_returns_none() {
    let raw = "diff --git a/foo b/foo\n@@ -1,2 +1,2 @@\n-old\n+new\n";
    assert!(try_filter("git diff", raw, Some(0)).is_none());
}

#[test]
fn diff_over_threshold_trims_long_context_keeps_changes() {
    let mut raw = String::new();
    for f in 0..50 {
        raw.push_str(&format!("diff --git a/file{f}.rs b/file{f}.rs\n"));
        raw.push_str("index 1111111..2222222 100644\n");
        raw.push_str(&format!("--- a/file{f}.rs\n"));
        raw.push_str(&format!("+++ b/file{f}.rs\n"));
        raw.push_str("@@ -1,12 +1,12 @@\n");
        for i in 0..10 {
            raw.push_str(&format!(" context line number {i} in file {f}\n"));
        }
        raw.push_str("-old value here\n");
        raw.push_str("+new value here\n");
    }
    assert!(
        raw.len() > 20_000,
        "fixture must exceed threshold, was {}",
        raw.len()
    );

    let outcome = try_filter("git diff", &raw, Some(0)).expect("should filter");
    assert!(outcome.text.contains("... [4 context lines trimmed]"));
    assert!(outcome.text.contains("-old value here"));
    assert!(outcome.text.contains("+new value here"));
    assert!(outcome.text.contains("@@ -1,12 +1,12 @@"));
    assert!(!outcome.text.contains("context line number 5 in file 0"));
    assert!(outcome.text.contains("context line number 0 in file 0"));
    assert!(outcome.text.contains("context line number 9 in file 0"));
}
