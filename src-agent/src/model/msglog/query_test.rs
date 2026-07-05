use super::*;
use crate::dto::chat::Role;

/// A unique path under the OS temp root for a single test, removed
/// recursively on drop. Mirrors `app::bgbash::mod_test`'s `TempDir` helper —
/// no `tempfile` dep in this crate's Cargo.toml.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-msglog-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn message_count_none_when_no_sqlite_yet() {
    let dir = TempDir::new("none");
    // Never appended to — no `messages.sqlite` should exist.
    assert_eq!(message_count(dir.path()), None);
}

#[test]
fn message_count_counts_appended_rows() {
    let dir = TempDir::new("counts");
    append(dir.path(), Role::User, "hi", None).unwrap();
    append(dir.path(), Role::Assistant, "hello", Some((10, 5, 0.001))).unwrap();
    append(dir.path(), Role::Tool, "tool output", None).unwrap();

    assert_eq!(message_count(dir.path()), Some(3));
}

#[test]
fn message_count_is_a_plain_row_count() {
    // `msglog::append` is never called with `Role::System` at any real call
    // site (see `message_count`'s doc comment), so a plain `COUNT(*)` is
    // equivalent to "count of non-System messages" in practice. This test
    // documents that the query itself does not special-case any role — if a
    // System row were ever appended, it would be counted too.
    let dir = TempDir::new("plain-count");
    append(dir.path(), Role::System, "system prompt", None).unwrap();
    append(dir.path(), Role::User, "hi", None).unwrap();

    assert_eq!(message_count(dir.path()), Some(2));
}
