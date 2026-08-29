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
    append(dir.path(), Role::User, "hi", None, None).unwrap();
    append(
        dir.path(),
        Role::Assistant,
        "hello",
        None,
        Some((10, 5, 0.001)),
    )
    .unwrap();
    append(dir.path(), Role::Tool, "tool output", None, None).unwrap();

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
    append(dir.path(), Role::System, "system prompt", None, None).unwrap();
    append(dir.path(), Role::User, "hi", None, None).unwrap();

    assert_eq!(message_count(dir.path()), Some(2));
}

#[test]
fn clear_rolling_summary_freezes_watermark_and_empties_text() {
    let dir = TempDir::new("clear-summary");
    append(dir.path(), Role::User, "hi", None, None).unwrap();
    append(
        dir.path(),
        Role::Assistant,
        "hello",
        None,
        Some((10, 5, 0.0)),
    )
    .unwrap();
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
    append(dir.path(), Role::User, "hello world", None, None).unwrap();
    append(
        dir.path(),
        Role::Assistant,
        "goodbye",
        None,
        Some((10, 5, 0.0)),
    )
    .unwrap();

    let hits = search_messages(dir.path(), "hello", 10, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].role, "user");
    assert!(
        hits[0].snippet.contains("hello"),
        "snippet should contain the match: {}",
        hits[0].snippet
    );
}

#[test]
fn search_messages_multi_term_or_finds_any_match() {
    let dir = TempDir::new("fts-multi");
    append(dir.path(), Role::User, "the quick brown fox", None, None).unwrap();
    append(
        dir.path(),
        Role::Assistant,
        "lazy dog sleeps",
        None,
        Some((10, 5, 0.0)),
    )
    .unwrap();
    append(dir.path(), Role::User, "unrelated text here", None, None).unwrap();

    // "fox zebra" -> "fox"* OR "zebra"* -> should match message 1 but not 2 or 3.
    let hits = search_messages(dir.path(), "fox zebra", 10, None).unwrap();
    assert_eq!(hits.len(), 1, "only fox matches");
    assert_eq!(hits[0].role, "user");
}

#[test]
fn search_messages_or_ranks_multiple_hits() {
    let dir = TempDir::new("fts-rank");
    append(
        dir.path(),
        Role::User,
        "security vulnerability found in parser",
        None,
        None,
    )
    .unwrap();
    append(
        dir.path(),
        Role::Assistant,
        "security is important always",
        None,
        Some((10, 5, 0.0)),
    )
    .unwrap();
    append(dir.path(), Role::User, "nothing to see here", None, None).unwrap();

    let hits = search_messages(dir.path(), "security", 10, None).unwrap();
    assert_eq!(hits.len(), 2);
    for h in &hits {
        assert!(
            h.snippet.contains("security"),
            "every hit must contain the term"
        );
    }
}

#[test]
fn search_messages_no_match_returns_empty() {
    let dir = TempDir::new("fts-nomatch");
    append(dir.path(), Role::User, "hello", None, None).unwrap();

    let hits = search_messages(dir.path(), "zzznotexist", 10, None).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn search_messages_strips_fts5_syntax_chars() {
    let dir = TempDir::new("fts-syntax");
    append(
        dir.path(),
        Role::User,
        "the FOO constant is defined in config.rs",
        None,
        None,
    )
    .unwrap();

    // Parentheses and asterisks should be stripped so the query doesn't error.
    let hits = search_messages(dir.path(), "FOO*", 10, None).unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].snippet.contains("FOO"));
}

#[test]
fn append_stores_and_fetch_reasoning() {
    let dir = TempDir::new("reasoning-store");
    append(
        dir.path(),
        Role::Assistant,
        "The answer is 42",
        Some("I considered many possibilities before settling on this"),
        Some((10, 5, 0.001)),
    )
    .unwrap();
    append(dir.path(), Role::User, "thanks", None, None).unwrap();

    // Reasoning is returned by fetch_messages_since.
    let msgs = fetch_messages_since(dir.path(), 0, 10);
    assert_eq!(msgs.len(), 2);
    assert_eq!(
        msgs[0].reasoning.as_deref(),
        Some("I considered many possibilities before settling on this")
    );
    // User message has no reasoning.
    assert!(msgs[1].reasoning.is_none());

    // FTS search still matches on content (not reasoning).
    let hits = search_messages(dir.path(), "answer", 10, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].role, "assistant");
    // The reasoning snippet is populated in search results.
    assert_eq!(
        hits[0].reasoning.as_deref(),
        Some("I considered many possibilities before settling on this")
    );
}

#[test]
fn append_empty_reasoning_stores_null() {
    let dir = TempDir::new("reasoning-empty");
    append(
        dir.path(),
        Role::Assistant,
        "hello",
        Some("   "),
        Some((10, 5, 0.0)),
    )
    .unwrap();

    let msgs = fetch_messages_since(dir.path(), 0, 10);
    assert_eq!(msgs.len(), 1);
    // Whitespace-only reasoning should be stored as NULL.
    assert!(msgs[0].reasoning.is_none());
}

#[test]
fn schema_meta_skips_fts_recount() {
    let dir = TempDir::new("schema-meta");
    // First append triggers ensure_schema + backfill.
    append(dir.path(), Role::User, "first message", None, None).unwrap();

    // Verify schema_meta has fts_backfilled=1.
    let conn = crate::model::msglog::schema::open(dir.path()).unwrap();
    let val: i64 = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'fts_backfilled'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0);
    assert_eq!(val, 1, "fts_backfilled should be set after first append");

    // Second open should succeed without re-running backfill (schema ready).
    let conn2 = crate::model::msglog::schema::open(dir.path()).unwrap();
    let val2: i64 = conn2
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'fts_backfilled'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0);
    assert_eq!(val2, 1, "meta unchanged after second open");
}

#[test]
fn search_still_works_after_reasoning_column() {
    let dir = TempDir::new("fts-after-reasoning");
    append(
        dir.path(),
        Role::Assistant,
        "we need to fix the parser bug",
        Some("I analyzed the stack trace and found the root cause"),
        Some((10, 5, 0.0)),
    )
    .unwrap();
    append(dir.path(), Role::User, "great, go ahead", None, None).unwrap();

    // FTS matches on content, not reasoning.
    let hits = search_messages(dir.path(), "parser", 10, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("parser"));
    assert_eq!(
        hits[0].reasoning.as_deref(),
        Some("I analyzed the stack trace and found the root cause")
    );

    // Reasoning-only terms don't match via FTS.
    let hits = search_messages(dir.path(), "stack trace", 10, None).unwrap();
    assert!(hits.is_empty(), "FTS should not index reasoning");
}

#[test]
fn search_messages_repairs_empty_fts_after_false_backfill_flag() {
    let dir = TempDir::new("fts-repair");
    append(dir.path(), Role::User, "unique_repair_token_xyz", None, None).unwrap();

    // Simulate a failed prior backfill: flag set, FTS wiped.
    {
        let conn = crate::model::msglog::schema::open(dir.path()).unwrap();
        conn.execute("DELETE FROM messages_fts", []).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('fts_backfilled', 1)",
            [],
        )
        .unwrap();
    }
    // Drop process-level ready cache by using a fresh path identity — open()
    // still re-runs ensure_schema only when the path is not in SCHEMA_READY.
    // Force re-open after clearing the in-process cache is not public; instead
    // call ensure path via search which opens the same path. If the path is
    // already marked ready from append, repair won't run until next process.
    // Clear by writing a sentinel through open after deleting ready is hard;
    // so invoke the public repair path by re-opening in a way that hits schema:
    // the SCHEMA_READY set still contains this path from append. Manually
    // trigger repair by calling open after removing the path is not exposed.
    // Workaround: create a *new* connection via open after we also clear the
    // meta+empty FTS — but SCHEMA_READY skips ensure_schema. Verify via a
    // subprocess-equivalent: call ensure through a duplicated dir is overkill.
    //
    // Direct unit coverage of the SQL repair: open a brand-new session dir
    // where we seed messages without going through append's FTS insert.
    let dir2 = TempDir::new("fts-repair2");
    {
        let conn = crate::model::msglog::schema::open(dir2.path()).unwrap();
        conn.execute(
            "INSERT INTO messages(role, content, created_at) VALUES ('user', 'repair_me_token_abc', 1)",
            [],
        )
        .unwrap();
        // Flag set + empty FTS (messages row present) — next open must rebuild.
        conn.execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('fts_backfilled', 1)",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM messages_fts", []).unwrap();
    }
    // SCHEMA_READY already has dir2 from the open above. Clear it by searching
    // after a process-local reopen isn't possible; instead delete from the
    // static set by using open on a path that re-runs only if not ready.
    // For this test, call search_messages which opens again — if ready-skip
    // prevents repair, hits stay empty. Force by removing ready entry via
    // another open of a copy is messy; call the public API after renaming.
    //
    // Practical approach: reopen under a path clone by copying the sqlite file
    // to a fresh dir so SCHEMA_READY misses it.
    let dir3 = TempDir::new("fts-repair3");
    std::fs::copy(
        dir2.path().join("messages.sqlite"),
        dir3.path().join("messages.sqlite"),
    )
    .unwrap();
    // Copy WAL/SHM if present.
    let _ = std::fs::copy(
        dir2.path().join("messages.sqlite-wal"),
        dir3.path().join("messages.sqlite-wal"),
    );
    let _ = std::fs::copy(
        dir2.path().join("messages.sqlite-shm"),
        dir3.path().join("messages.sqlite-shm"),
    );

    let hits = search_messages(dir3.path(), "repair_me_token_abc", 10, None).unwrap();
    assert_eq!(hits.len(), 1, "repair backfill should reindex messages");
    assert!(hits[0].snippet.contains("repair_me_token_abc"));
}
