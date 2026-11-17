use super::*;
use std::path::PathBuf;

#[test]
fn workspaces_block_names_current_single_root() {
    let dirs = vec![PathBuf::from("/home/user/project")];
    assert!(Session::format_workspaces_block(&dirs)
        .unwrap()
        .contains("[0]"));
}

#[test]
fn workspaces_block_none_for_empty() {
    assert!(Session::format_workspaces_block(&[]).is_none());
}

#[test]
fn workspaces_block_two_roots() {
    let dirs = vec![
        PathBuf::from("/home/user/project-a"),
        PathBuf::from("/home/user/project-b"),
    ];
    let block = Session::format_workspaces_block(&dirs).unwrap();
    assert!(block.contains("# Workspaces"), "missing header");
    assert!(
        block.contains("[0] /home/user/project-a  (primary)"),
        "primary wrong: {block}"
    );
    assert!(
        block.contains("[1] /home/user/project-b"),
        "second wrong: {block}"
    );
    assert!(
        block.contains("Bare relative tool paths target [0]"),
        "missing guidance"
    );
}

#[test]
fn workspaces_block_normalizes_backslashes_on_windows_only() {
    let dirs = vec![
        PathBuf::from("C:\\Users\\dev\\a"),
        PathBuf::from("C:\\Users\\dev\\b"),
    ];
    let block = Session::format_workspaces_block(&dirs).unwrap();
    if cfg!(windows) {
        assert!(block.contains("[0] C:/Users/dev/a  (primary)"));
        assert!(block.contains("[1] C:/Users/dev/b"));
    } else {
        // Backslash is a legal file-name character on macOS/Linux: kept verbatim.
        assert!(block.contains("[0] C:\\Users\\dev\\a  (primary)"));
        assert!(block.contains("[1] C:\\Users\\dev\\b"));
    }
}

#[test]
fn workspaces_block_three_roots() {
    let dirs = vec![
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/c"),
    ];
    let block = Session::format_workspaces_block(&dirs).unwrap();
    assert!(block.contains("[0] /a  (primary)"));
    assert!(block.contains("[1] /b"));
    assert!(block.contains("[2] /c"));
}

#[test]
fn unattended_defaults_explicit_checkpoint_and_restart_recovery() {
    use crate::ipc::proto::turn::{ApprovalPolicy, PlanPolicy};
    let dir = std::env::temp_dir().join(format!("koma-unattended-{}", uuid::Uuid::new_v4()));
    let _cleanup = scopeguard::guard(dir.clone(), |path| { let _ = std::fs::remove_dir_all(path); });
    let session = Session::new("unattended-test".into(), dir.join("session"),
        "test".into(), Settings::default(), Conversation::new("test"));
    let initial = session.load_unattended().unwrap();
    assert_eq!(initial.policy, SessionPolicy::default());
    assert!(initial.last_turn.is_none());
    assert!(!session.path.join("unattended.json").exists());

    let policy = SessionPolicy { plan: PlanPolicy::Forbid, approval: ApprovalPolicy::Reject };
    let turn = TurnRecord { id: "turn-1".into(), state: TurnState::Running, error: None };
    session.save_unattended_policy(&policy, Some(&turn)).unwrap();
    let checkpoint = std::fs::read(session.path.join("unattended.json")).unwrap();
    session.save().unwrap();
    assert_eq!(checkpoint, std::fs::read(session.path.join("unattended.json")).unwrap(),
        "ordinary saves must not change policy or the captured turn");
    let loaded = session.load_unattended().unwrap();
    assert_eq!(loaded.policy, policy);
    let recovered = loaded.last_turn.unwrap();
    assert_eq!(recovered.id, "turn-1");
    assert_eq!(recovered.state, TurnState::Failed);
    assert_eq!(recovered.error.as_deref(), Some("daemon restarted"));
    assert_eq!(session.load_unattended().unwrap().last_turn, Some(recovered));

    std::fs::write(session.path.join("unattended.json"), "{}").unwrap();
    assert_eq!(session.load_unattended().unwrap().policy, SessionPolicy::default());
    std::fs::write(session.path.join("unattended.json"), "invalid").unwrap();
    assert!(session.load_unattended().is_err(), "corrupt policy must not silently permit execution");
}
