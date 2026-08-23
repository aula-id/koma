use super::parse;

#[test]
fn bare_remote_opens_saved_host_picker() {
    let opts = parse(["koma", "remote"].into_iter().map(String::from));
    assert!(opts.remote_picker);
    assert!(opts.remote_target.is_none());
}

#[test]
fn remote_target_uses_direct_remote_entry() {
    let opts = parse(
        ["koma", "remote", "alice@example.test"]
            .into_iter()
            .map(String::from),
    );
    assert!(!opts.remote_picker);
    assert_eq!(opts.remote_target.as_deref(), Some("alice@example.test"));
}

#[test]
fn lsp_status_subcommand() {
    let opts = parse(["koma", "lsp", "status"].into_iter().map(String::from));
    match opts.lsp {
        Some(crate::lsp::LspCli::Status) => {}
        other => panic!("expected Status, got {other:?}"),
    }
}

#[test]
fn lsp_install_all_force() {
    let opts = parse(
        ["koma", "lsp", "install", "--all", "--force"]
            .into_iter()
            .map(String::from),
    );
    match opts.lsp {
        Some(crate::lsp::LspCli::Install {
            id: None,
            all: true,
            force: true,
        }) => {}
        other => panic!("expected install --all --force, got {other:?}"),
    }
}
