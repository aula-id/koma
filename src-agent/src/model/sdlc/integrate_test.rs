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
        draft_locks: Default::default(),
    }
}

fn tmp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "koma-int-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{args:?} {:?}",
        String::from_utf8_lossy(&o.stderr)
    );
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
    let mut m = sample("x", "develop");
    // Need a real dir so frozen target path check passes before branch check.
    let dir = tmp_root("missing-br");
    git(&dir, &["init", "-b", "develop"]);
    git(&dir, &["config", "user.email", "t@t"]);
    git(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "init"]);
    m.target_worktree_path = Some(dir.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
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
    let root = tmp_root("drift");
    git(&root, &["init", "-b", "develop"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init"]);
    let head = mission::current_git_head(&root).unwrap();
    // Freeze target as develop, then switch to main → drift.
    git(&root, &["checkout", "-b", "main"]);

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
fn nothing_to_land_when_zero_commits_ahead() {
    let root = tmp_root("zero-ahead");
    git(&root, &["init", "-b", "develop"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init"]);
    let head = mission::current_git_head(&root).unwrap();
    git(&root, &["branch", "feat/empty"]);

    let mut m = sample("feat/empty", "develop");
    m.target_worktree_path = Some(root.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(head);
    m.worktree_path = None;
    m.hash = m.recompute_hash();
    let r = try_integrate(&m, false);
    assert!(!r.success, "{}", r.message);
    assert!(
        r.message.contains("nothing to land"),
        "unexpected: {}",
        r.message
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dirty_mission_worktree_blocks_integrate() {
    let root = tmp_root("dirty-wt");
    let mission_wt = root.join("mission");
    git(&root, &["init", "-b", "develop"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init"]);
    let head = mission::current_git_head(&root).unwrap();
    git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            "feat/dirty",
            &mission_wt.to_string_lossy(),
        ],
    );
    std::fs::write(mission_wt.join("b.txt"), "b").unwrap();
    git(&mission_wt, &["add", "."]);
    git(&mission_wt, &["commit", "-m", "feat"]);
    // Dirty after commit.
    std::fs::write(mission_wt.join("dirty.txt"), "x").unwrap();

    let mut m = sample("feat/dirty", "develop");
    m.target_worktree_path = Some(root.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(head);
    m.worktree_path = Some(mission_wt.to_string_lossy().into_owned());
    m.hash = m.recompute_hash();
    let r = try_integrate(&m, false);
    assert!(!r.success, "{}", r.message);
    assert!(
        r.message.contains("mission worktree dirty"),
        "unexpected: {}",
        r.message
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn temp_git_ff_integrate_into_non_main_target() {
    let root = tmp_root("ff-happy");
    // Non-main target branch.
    git(&root, &["init", "-b", "develop"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init"]);
    let head = mission::current_git_head(&root).unwrap();
    git(&root, &["checkout", "-b", "sdlc/feat"]);
    std::fs::write(root.join("b.txt"), "b").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "feat"]);
    git(&root, &["checkout", "develop"]);

    let mut m = sample("sdlc/feat", "develop");
    m.target_worktree_path = Some(root.to_string_lossy().into_owned());
    m.target_branch = Some("develop".into());
    m.target_head = Some(head);
    m.worktree_path = None;
    m.hash = m.recompute_hash();
    let r = try_integrate(&m, false);
    assert!(r.success, "{}", r.message);
    assert!(
        r.message.contains("1 commit")
            && r.message.contains("develop")
            && r.message.contains("sdlc/feat")
            && !r.message.contains("into main via"),
        "status must be ship summary with count, got: {}",
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

#[test]
fn integrate_blocks_main_branch() {
    let mut m = sample("feat/x", "main");
    m.target_branch = Some("main".into());
    m.hash = m.recompute_hash();
    let r = try_integrate(&m, false);
    assert!(!r.success);
    assert!(
        r.message.contains("main/master") || r.message.contains("blocked"),
        "must block main: {}",
        r.message
    );
}

#[test]
fn integrate_blocks_master_branch() {
    let mut m = sample("feat/x", "master");
    m.target_branch = Some("master".into());
    m.hash = m.recompute_hash();
    let r = try_integrate(&m, false);
    assert!(!r.success);
    assert!(
        r.message.contains("main/master") || r.message.contains("blocked"),
        "must block master: {}",
        r.message
    );
}

#[test]
fn integrate_allows_non_main_branch() {
    let m = sample("feat/x", "develop");
    // force_branch_only short-circuits before path checks, so it tests
    // that the main/master guard fires after target_branch is resolved.
    let r = try_integrate(&m, true);
    assert!(!r.success); // force_branch_only is not success
    assert!(
        !r.message.contains("blocked"),
        "develop must not be blocked: {}",
        r.message
    );
}
