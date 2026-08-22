use super::*;

#[test]
fn normalization_preserves_relative_parents_and_clamps_roots() {
    assert_eq!(normalize_lexical("../foo"), "../foo");
    assert_eq!(normalize_lexical("a/../../b"), "../b");
    assert_eq!(normalize_lexical("/../../x"), "/x");
    assert_eq!(normalize_lexical("foo/./bar/../baz"), "foo/baz");
}

#[test]
fn normalization_preserves_windows_roots_on_unix() {
    assert_eq!(normalize_lexical(r"C:\a\..\b"), "C:/b");
    assert_eq!(normalize_lexical(r"C:\"), "C:/");
    assert_eq!(
        normalize_lexical(r"\\server\share\a\..\b"),
        "//server/share/b"
    );
    assert_eq!(
        normalize_lexical(r"\\server\share\..\x"),
        "//server/share/x"
    );
}

#[test]
fn owner_is_longest_component_prefix() {
    let roots = vec!["/project".into(), "/project/sub".into()];
    assert_eq!(
        owner_root("/project/sub/a.rs", &roots),
        Some("/project/sub")
    );
    assert_eq!(owner_root("/projectile/a.rs", &roots), None);
    assert_eq!(owner_root("relative.rs", &roots), None);
}

#[test]
fn contained_join_rejects_escapes_and_missing_base() {
    let roots = vec!["/project".into(), "/project/nested".into()];
    assert_eq!(
        join_contained(Some("/project/src"), "../lib.rs", &roots),
        Some(("/project/lib.rs".into(), "/project".into()))
    );
    assert_eq!(
        join_contained(Some("/project/nested/src"), "a.rs", &roots),
        Some(("/project/nested/src/a.rs".into(), "/project/nested".into()))
    );
    assert!(join_contained(Some("/project"), "../../escape.rs", &roots).is_none());
    assert!(join_contained(None, "src/a.rs", &roots).is_none());
    assert!(join_contained(Some("relative"), "a.rs", &roots).is_none());
}
