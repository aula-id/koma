#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn parses_porcelain_branch_headers() {
    let (branch, upstream) = parse_status_headers(
        "# branch.oid abc\n# branch.head feature/x\n# branch.upstream origin/feature/x\n# branch.ab +2 -1\n",
    );
    assert_eq!(branch.as_deref(), Some("feature/x"));
    assert_eq!(upstream.as_deref(), Some("origin/feature/x"));
}

#[test]
fn detached_and_missing_upstream_are_distinct() {
    assert_eq!(
        parse_status_headers("# branch.head (detached)\n"),
        (None, None)
    );
    assert_eq!(
        parse_status_headers("# branch.head topic\n"),
        (Some("topic".to_string()), None)
    );
}

#[test]
fn remote_components_reject_option_and_whitespace_injection() {
    assert!(valid_remote_component("origin"));
    assert!(valid_remote_component("team/repo"));
    assert!(!valid_remote_component(""));
    assert!(!valid_remote_component("--upload-pack=evil"));
    assert!(!valid_remote_component("origin other"));
    assert!(!valid_remote_component("origin\nother"));
}

fn ok_output(text: &str) -> std::process::Output {
    std::process::Output {
        status: std::process::Command::new("true").status().unwrap(),
        stdout: text.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

#[test]
fn existing_upstream_honors_push_remote_precedence() {
    let git = |_root: &Path, args: &[&str], _extra: Option<(&str, &str)>| {
        let value = match args {
            ["status", "--porcelain=v2", "--branch"] => {
                "# branch.head topic\n# branch.upstream origin/topic\n"
            }
            ["config", "--get", "branch.topic.remote"] => "origin\n",
            ["config", "--get", "branch.topic.merge"] => "refs/heads/topic\n",
            ["config", "--get", "branch.topic.pushRemote"] => "publish\n",
            ["config", "--get", "remote.pushDefault"] => "fallback\n",
            ["rev-parse", "--verify", "origin/topic"] => "abc\n",
            _ => return None,
        };
        Some(ok_output(value))
    };
    let target = plan_target(&git, Path::new(".")).unwrap();
    assert_eq!(target.remote, "publish");
    assert_eq!(target.remote_branch, "topic");
}

#[test]
fn existing_upstream_refuses_ambiguous_override_destination() {
    let git = |_root: &Path, args: &[&str], _extra: Option<(&str, &str)>| {
        let value = match args {
            ["status", "--porcelain=v2", "--branch"] => {
                "# branch.head topic\n# branch.upstream origin/main\n"
            }
            ["config", "--get", "branch.topic.remote"] => "origin\n",
            ["config", "--get", "branch.topic.merge"] => "refs/heads/main\n",
            ["config", "--get", "branch.topic.pushRemote"] => "publish\n",
            _ => return None,
        };
        Some(ok_output(value))
    };
    assert!(plan_target(&git, Path::new(".")).is_err());
}

#[test]
fn push_modes_have_stable_wire_names() {
    assert_eq!(
        serde_json::to_string(&GitPushMode::Automatic).unwrap(),
        "\"automatic\""
    );
    assert_eq!(
        serde_json::to_string(&GitPushMode::SetUpstream).unwrap(),
        "\"set-upstream\""
    );
    assert_eq!(
        serde_json::to_string(&GitPushMode::ForceWithLease).unwrap(),
        "\"force-with-lease\""
    );
    assert_eq!(
        serde_json::from_str::<GitPushMode>("\"force-with-lease\"").unwrap(),
        GitPushMode::ForceWithLease
    );
}

#[test]
fn rebase_tracker_records_only_rewrites() {
    let root = Path::new("tracker-test-root");
    let key = (root.to_path_buf(), "topic".to_string());
    proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
    begin_rebase(root, "topic", "aaa");
    assert!(has_pending_rebase(root, "topic", "aaa"));
    assert!(!finish_rebase(root, "other", "bbb"));
    assert!(has_pending_rebase(root, "topic", "aaa"));
    record_rebase(root, "topic", "aaa", "aaa");
    assert!(proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .is_none());
    record_rebase(root, "topic", "aaa", "bbb");
    let proof = proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
        .unwrap();
    assert_eq!(proof.old_tip, "aaa");
    assert_eq!(proof.new_tip, "bbb");
    proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
}
