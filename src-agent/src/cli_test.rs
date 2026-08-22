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
