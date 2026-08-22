#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// A well-formed reverse-DNS id passes; anything that could escape `extensions_dir()`
/// as a directory name (empty, `.`/`..`, embedded `/`) is rejected.
#[test]
fn safe_ext_id_rejects_path_escapes() {
    assert_eq!(
        safe_ext_id("run.koma.example.fleet-board-daemon"),
        Some("run.koma.example.fleet-board-daemon")
    );
    assert_eq!(safe_ext_id(""), None);
    assert_eq!(safe_ext_id("."), None);
    assert_eq!(safe_ext_id(".."), None);
    assert_eq!(safe_ext_id("../etc"), None);
    assert_eq!(safe_ext_id("a/b"), None);
    assert_eq!(safe_ext_id(".hidden"), None);
}

/// A plain relative asset path resolves cleanly; `..` (any position), an absolute
/// path, or an embedded backslash is rejected outright — the zip-slip-style guard
/// applied to protocol request paths instead of zip entries.
#[test]
fn safe_ext_rel_path_rejects_escapes() {
    assert_eq!(
        safe_ext_rel_path("index.html"),
        Some(PathBuf::from("index.html"))
    );
    assert_eq!(
        safe_ext_rel_path("assets/index-abc123.js"),
        Some(PathBuf::from("assets/index-abc123.js"))
    );
    assert_eq!(safe_ext_rel_path(""), Some(PathBuf::new()));
    assert_eq!(safe_ext_rel_path(".."), None);
    assert_eq!(safe_ext_rel_path("../../etc/passwd"), None);
    assert_eq!(safe_ext_rel_path("assets/../../escape"), None);
    assert_eq!(safe_ext_rel_path("/etc/passwd"), None);
    assert_eq!(safe_ext_rel_path("a\\b"), None);
}
