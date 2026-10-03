#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{events_after_on, max_usage_id_on};
use crate::model::usage::ledger::ensure_schema;

fn mem() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    ensure_schema(&conn).unwrap();
    conn
}

fn insert(conn: &rusqlite::Connection, ts: i64, model: &str, tin: i64, tout: i64) {
    conn.execute(
        "INSERT INTO usage (ts, model_id, role, session_uuid, pwd_hash,
            tokens_in, tokens_cached, tokens_out, cost)
         VALUES (?1, ?2, 'main', '', '', ?3, 0, ?4, 0.0)",
        rusqlite::params![ts, model, tin, tout],
    )
    .unwrap();
}

#[test]
fn empty_ledger_is_zero_and_empty() {
    let conn = mem();
    assert_eq!(max_usage_id_on(&conn), 0);
    assert!(events_after_on(&conn, 0, 50).is_empty());
}

#[test]
fn catchup_walks_ids_in_order_and_respects_watermark() {
    let conn = mem();
    insert(&conn, 100, "koma/apple", 10, 2);
    insert(&conn, 200, "anthropic/claude-sonnet-4", 20, 4);
    insert(&conn, 300, "koma/apple", 30, 6);
    assert_eq!(max_usage_id_on(&conn), 3);

    let first = events_after_on(&conn, 0, 2);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].id, 1);
    assert_eq!(first[0].model_id, "koma/apple");
    assert_eq!(first[1].id, 2);

    let rest = events_after_on(&conn, first[1].id, 50);
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].id, 3);
    assert_eq!(rest[0].tokens_in, 30);
    assert!(events_after_on(&conn, 3, 50).is_empty());
}
