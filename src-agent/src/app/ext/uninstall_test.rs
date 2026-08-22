#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// The id-safety guard mirrors `install::validate_id`: reverse-DNS ids pass; path-escape
/// / pure-punctuation ids are rejected (so the uninstall `remove_dir_all` can never
/// escape `extensions/`).
#[test]
fn safe_ext_id_rejects_path_escapes() {
    assert!(is_safe_ext_id("run.koma.gateway"));
    assert!(is_safe_ext_id("run.koma.example.echo-tool_daemon"));
    assert!(!is_safe_ext_id(""));
    assert!(!is_safe_ext_id("."));
    assert!(!is_safe_ext_id(".."));
    assert!(!is_safe_ext_id("../etc"));
    assert!(!is_safe_ext_id("a/b"));
    assert!(!is_safe_ext_id(".hidden"));
}
