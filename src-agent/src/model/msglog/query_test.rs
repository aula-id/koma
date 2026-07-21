use super::*;
use crate::dto::chat::Role;
use crate::model::msglog::{clear_rolling_summary, read_summary, write_summary};

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

#[test]
fn clear_rolling_summary_freezes_watermark_and_empties_text() {
    let dir = TempDir::new("clear-summary");
    append(dir.path(), Role::User, "hi", None).unwrap();
    append(dir.path(), Role::Assistant, "hello", Some((10, 5, 0.0))).unwrap();
    let tip = max_message_id(dir.path());
    assert!(tip >= 2);
    // Seed a non-empty summary covering the archive.
    write_summary(dir.path(), "old summary of prior chat", tip, tip + 1).unwrap();
    clear_rolling_summary(dir.path()).unwrap();

    let sum = read_summary(dir.path()).expect("summary row kept");
    assert!(sum.text.is_empty(), "body wiped so shape skips injection");
    assert_eq!(sum.covers_up_to, tip, "watermark frozen at archive tip");
    // Archive rows must remain.
    assert_eq!(message_count(dir.path()), Some(2));
}

#[test]
fn clear_rolling_summary_on_empty_archive_deletes_row() {
    let dir = TempDir::new("clear-empty");
    // Open schema by writing then... actually empty dir has no DB. Seed then
    // delete via clear when max_id is 0 after truncate is awkward; write a
    // summary against an empty DB first (open creates schema).
    write_summary(dir.path(), "stale", 0, 1).unwrap();
    clear_rolling_summary(dir.path()).unwrap();
    assert!(read_summary(dir.path()).is_none());
}

#[test]
fn search_messages_single_term_finds_match() {
    let dir = TempDir::new("fts-single");
    append(dir.path(), Role::User, "hello world", None).unwrap();
    append(dir.path(), Role::Assistant, "goodbye", Some((10, 5, 0.0))).unwrap();

    let hits = search_messages(dir.path(), "hello", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].role, "user");
    assert!(hits[0].snippet.contains("hello"), "snippet should contain the match: {}", hits[0].snippet);
}

#[test]
fn search_messages_multi_term_or_finds_any_match() {
    let dir = TempDir::new("fts-multi");
    append(dir.path(), Role::User, "the quick brown fox", None).unwrap();
    append(dir.path(), Role::Assistant, "lazy dog sleeps", Some((10, 5, 0.0))).unwrap();
    append(dir.path(), Role::User, "unrelated text here", None).unwrap();

    // "fox zebra" -> fox* OR zebra* -> should match message 1 but not 2 or 3.
    let hits = search_messages(dir.path(), "fox zebra", 10);
    assert_eq!(hits.len(), 1, "only fox matches");
    assert_eq!(hits[0].role, "user");
}

#[test]
fn search_messages_or_ranks_multiple_hits() {
    let dir = TempDir::new("fts-rank");
    append(dir.path(), Role::User, "security vulnerability found in parser", None).unwrap();
    append(dir.path(), Role::Assistant, "security is important always", Some((10, 5, 0.0))).unwrap();
    append(dir.path(), Role::User, "nothing to see here", None).unwrap();

    let hits = search_messages(dir.path(), "security", 10);
    assert_eq!(hits.len(), 2);
    for h in &hits {
        assert!(h.snippet.contains("security"), "every hit must contain the term");
    }
}

#[test]
fn search_messages_no_match_returns_empty() {
    let dir = TempDir::new("fts-nomatch");
    append(dir.path(), Role::User, "hello", None).unwrap();

    let hits = search_messages(dir.path(), "zzznotexist", 10);
    assert!(hits.is_empty());
}

#[test]
fn search_messages_strips_fts5_syntax_chars() {
    let dir = TempDir::new("fts-syntax");
    append(dir.path(), Role::User, "the FOO constant is defined in config.rs", None).unwrap();

    // Parentheses and asterisks should be stripped so the query doesn't error.
    let hits = search_messages(dir.path(), "FOO*", 10);
    assert!(!hits.is_empty());
    assert!(hits[0].snippet.contains("FOO"));
}
