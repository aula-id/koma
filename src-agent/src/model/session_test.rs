use super::*;
use std::path::PathBuf;

#[test]
fn workspaces_block_none_for_single_root() {
    let dirs = vec![PathBuf::from("/home/user/project")];
    assert!(Session::format_workspaces_block(&dirs).is_none());
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
fn workspaces_block_normalizes_backslashes() {
    let dirs = vec![
        PathBuf::from("C:\\Users\\dev\\a"),
        PathBuf::from("C:\\Users\\dev\\b"),
    ];
    let block = Session::format_workspaces_block(&dirs).unwrap();
    assert!(block.contains("[0] C:/Users/dev/a  (primary)"));
    assert!(block.contains("[1] C:/Users/dev/b"));
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
