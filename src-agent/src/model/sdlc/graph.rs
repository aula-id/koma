//! SDLC graph: nodes + events in `messages.sqlite`.
//!
//! The graph provides the live checklist projection for SDLC missions.
//! Nodes represent tasks; events track transitions.

use anyhow::Result;
use rusqlite::Connection;

/// A single graph node (task) in the SDLC graph.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields reserved for keeper / richer UI
pub struct GraphTask {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub status: String, // pending | active | blocked | done | cancelled
    pub phase: Option<String>,
    pub notes: String,
    pub verify_bit: bool,
    pub updated_at: i64,
}

/// Current unix timestamp in seconds.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Ensure the SDLc tables exist in the given connection.
pub fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sdlc_nodes (
            id TEXT PRIMARY KEY,
            parent_id TEXT,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            phase TEXT,
            notes TEXT NOT NULL DEFAULT '',
            verify_bit INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sdlc_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id TEXT,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS mission_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Replace the open node set from a checklist of titles. Each title becomes a
/// new pending node; existing done/cancelled nodes are preserved.
pub fn replace_nodes_from_checklist(
    conn: &Connection,
    items: &[(String, String)], // (title, status)
) -> Result<()> {
    let now = now_secs();
    // Cancel any existing non-done nodes first
    conn.execute(
        "UPDATE sdlc_nodes SET status = 'cancelled', updated_at = ?1 WHERE status NOT IN ('done', 'cancelled')",
        [now],
    )?;
    // Upsert each item
    for (i, (title, status)) in items.iter().enumerate() {
        let display_id = format!("t{}", i + 1);
        conn.execute(
            "INSERT OR REPLACE INTO sdlc_nodes (id, title, status, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![display_id, title, status, now],
        )?;
    }
    Ok(())
}

/// List all nodes that are NOT done or cancelled (open tasks).
pub fn list_open(conn: &Connection) -> Result<Vec<GraphTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE status NOT IN ('done', 'cancelled') ORDER BY updated_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphTask {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            phase: row.get(4)?,
            notes: row.get(5)?,
            verify_bit: row.get::<_, i64>(6)? != 0,
            updated_at: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// List all nodes that are done (sealed tasks).
pub fn list_sealed(conn: &Connection) -> Result<Vec<GraphTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE status = 'done' ORDER BY updated_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphTask {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            phase: row.get(4)?,
            notes: row.get(5)?,
            verify_bit: row.get::<_, i64>(6)? != 0,
            updated_at: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Mark a node's status and append a transition event.
pub fn mark_status(conn: &Connection, node_id: &str, status: &str) -> Result<()> {
    let now = now_secs();
    conn.execute(
        "UPDATE sdlc_nodes SET status = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![status, now, node_id],
    )?;
    conn.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'status_change', ?2, ?3)",
        rusqlite::params![node_id, status, now],
    )?;
    Ok(())
}

/// Append an event to a node.
pub fn append_event(conn: &Connection, node_id: &str, kind: &str, detail: &str) -> Result<()> {
    let now = now_secs();
    conn.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![node_id, kind, detail, now],
    )?;
    Ok(())
}

/// Set the `verify_bit` on a graph node and append a `verify` event.
/// When `verified` is false, the node is also reopened to `active` status.
pub fn set_verify_bit(conn: &Connection, node_id: &str, verified: bool) -> Result<()> {
    let now = now_secs();
    let bit: i64 = if verified { 1 } else { 0 };
    conn.execute(
        "UPDATE sdlc_nodes SET verify_bit = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![bit, now, node_id],
    )?;
    let detail = if verified {
        "verified".to_string()
    } else {
        "verify_failed".to_string()
    };
    append_event(conn, node_id, "verify", &detail)?;
    // When verify fails, reopen to active so keeper can track it.
    if !verified {
        mark_status(conn, node_id, "active")?;
    }
    Ok(())
}

/// List all nodes that are `done` but have `verify_bit = 0` (false-done).
pub fn list_false_done(conn: &Connection) -> Result<Vec<GraphTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE status = 'done' AND verify_bit = 0 ORDER BY updated_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphTask {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            phase: row.get(4)?,
            notes: row.get(5)?,
            verify_bit: row.get::<_, i64>(6)? != 0,
            updated_at: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Reopen all false-done nodes (done without verify evidence) to active.
/// Returns the list of reopened tasks.
pub fn reopen_false_done(conn: &Connection) -> Result<Vec<GraphTask>> {
    let false_done = list_false_done(conn)?;
    let mut reopened = Vec::new();
    for node in &false_done {
        mark_status(conn, &node.id, "active")?;
        append_event(
            conn,
            &node.id,
            "false_done_reopen",
            "reopened: done without verify evidence",
        )?;
        reopened.push(node.clone());
    }
    Ok(reopened)
}

/// List all graph nodes (open, sealed, cancelled — everything).
#[allow(dead_code)] // reserved for richer keeper/UI projections
pub fn list_all(conn: &Connection) -> Result<Vec<GraphTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes ORDER BY updated_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GraphTask {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            phase: row.get(4)?,
            notes: row.get(5)?,
            verify_bit: row.get::<_, i64>(6)? != 0,
            updated_at: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Set a key-value pair in the `mission_meta` table.
pub fn set_mission_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO mission_meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Get a value from the `mission_meta` table by key.
pub fn get_mission_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM mission_meta WHERE key = ?1")?;
    let mut rows = stmt.query_map([key], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}


#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use rusqlite::Connection;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn verify_bit_and_false_done_reopen() {
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[("a".into(), "done".into()), ("b".into(), "pending".into())],
        )
        .unwrap();
        // t1 done without verify → false done
        let false_done = list_false_done(&conn).unwrap();
        assert_eq!(false_done.len(), 1);
        assert_eq!(false_done[0].id, "t1");

        set_verify_bit(&conn, "t1", true).unwrap();
        mark_status(&conn, "t1", "done").unwrap();
        assert!(list_false_done(&conn).unwrap().is_empty());

        // fail verify reopens
        set_verify_bit(&conn, "t1", false).unwrap();
        let open = list_open(&conn).unwrap();
        assert!(open.iter().any(|n| n.id == "t1" && n.status == "active"));
    }

    #[test]
    fn mission_meta_roundtrip() {
        let conn = mem();
        set_mission_meta(&conn, "k", "v").unwrap();
        assert_eq!(get_mission_meta(&conn, "k").unwrap().as_deref(), Some("v"));
        assert_eq!(get_mission_meta(&conn, "missing").unwrap(), None);
    }
}
