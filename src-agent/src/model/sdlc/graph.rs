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

/// Derive a stable graph-node id from a title: `n-{slug}-{hash6}` where slug
/// is the first 4 hyphenated ASCII-alphanumeric word groups (max 32 chars) and
/// hash6 is the first 3 bytes of the SHA-256 hex digest.
fn stable_id_for_title(title: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    let hex: String = hasher.finalize()[..3]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "task".to_string()
    } else {
        slug.chars().take(32).collect()
    };
    format!("n-{slug}-{hex}")
}

/// Replace the open node set from a checklist of titles by TITLE identity.
///
/// - Match existing nodes by exact title; reuse id and preserve parent_id/phase/notes/verify_bit.
/// - New titles get a stable id: `n-{slug}-{hash6}` from title (not positional t1..tn).
/// - Open nodes whose titles are absent from `items` are cancelled.
/// - Done/cancelled nodes not in items are left untouched (sealed history).
/// - When an existing done+verified node is still listed as done, keep verify_bit=1.
/// - When status moves away from done, clear verify_bit to 0.
pub fn replace_nodes_from_checklist(
    conn: &Connection,
    items: &[(String, String)], // (title, status)
) -> Result<()> {
    let now = now_secs();

    // 1. Load all existing nodes so we can match by title.
    let all = list_all(conn)?;

    // 2. Build title → most-recently-updated non-cancelled node for reuse.
    use std::collections::HashMap;
    let mut by_title: HashMap<String, GraphTask> = HashMap::new();
    for node in &all {
        if node.status == "cancelled" {
            continue;
        }
        match by_title.get(&node.title) {
            Some(existing) if existing.updated_at >= node.updated_at => {}
            _ => {
                by_title.insert(node.title.clone(), node.clone());
            }
        }
    }

    // 3. Process each item: reuse existing or insert stable-id node.
    let mut kept_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (title, status) in items {
        if let Some(ex) = by_title.get(title) {
            // Preserve id, parent_id, phase, notes. Preserve verify_bit only if still done.
            let verify_bit = if status == "done" {
                ex.verify_bit
            } else {
                false
            };
            conn.execute(
                "UPDATE sdlc_nodes SET status = ?1, verify_bit = ?2, updated_at = ?3 \
                 WHERE id = ?4",
                rusqlite::params![status, verify_bit as i64, now, ex.id],
            )?;
            kept_ids.insert(ex.id.clone());
        } else {
            let id = stable_id_for_title(title);
            conn.execute(
                "INSERT OR REPLACE INTO sdlc_nodes \
                 (id, parent_id, title, status, phase, notes, verify_bit, updated_at) \
                 VALUES (?1, NULL, ?2, ?3, NULL, '', 0, ?4)",
                rusqlite::params![id, title, status, now],
            )?;
            kept_ids.insert(id);
        }
    }

    // 4. Cancel open nodes whose titles are absent from items.
    for node in &all {
        if node.status == "done" || node.status == "cancelled" {
            continue;
        }
        if !kept_ids.contains(&node.id) {
            conn.execute(
                "UPDATE sdlc_nodes SET status = 'cancelled', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, node.id],
            )?;
        }
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
        // "a" done without verify → false done (look up by title since ids are now stable)
        let false_done = list_false_done(&conn).unwrap();
        assert_eq!(false_done.len(), 1);
        assert_eq!(false_done[0].title, "a");

        set_verify_bit(&conn, &false_done[0].id, true).unwrap();
        mark_status(&conn, &false_done[0].id, "done").unwrap();
        assert!(list_false_done(&conn).unwrap().is_empty());

        // fail verify reopens
        set_verify_bit(&conn, &false_done[0].id, false).unwrap();
        let open = list_open(&conn).unwrap();
        assert!(open.iter().any(|n| n.id == false_done[0].id && n.status == "active"));
    }

    #[test]
    fn stable_id_and_verify_preserve() {
        let conn = mem();
        // First checklist: "x" done + "y" pending
        replace_nodes_from_checklist(
            &conn,
            &[("x".into(), "done".into()), ("y".into(), "pending".into())],
        )
        .unwrap();
        // Verify "x"
        let sealed = list_sealed(&conn).unwrap();
        let x_node = sealed.iter().find(|n| n.title == "x").unwrap();
        set_verify_bit(&conn, &x_node.id, true).unwrap();
        mark_status(&conn, &x_node.id, "done").unwrap();
        let x_id_before = x_node.id.clone();

        // Second checklist: same titles — ids should be preserved
        replace_nodes_from_checklist(
            &conn,
            &[("x".into(), "done".into()), ("y".into(), "pending".into())],
        )
        .unwrap();
        let sealed = list_sealed(&conn).unwrap();
        let x_node2 = sealed.iter().find(|n| n.title == "x").unwrap();
        assert_eq!(x_node2.id, x_id_before, "stable id preserved across replace");
        assert!(x_node2.verify_bit, "verify_bit preserved when done→done");

        // Third checklist: "x" moved to pending → verify_bit cleared
        replace_nodes_from_checklist(
            &conn,
            &[("x".into(), "pending".into()), ("y".into(), "done".into())],
        )
        .unwrap();
        let open = list_open(&conn).unwrap();
        let x_open = open.iter().find(|n| n.title == "x").unwrap();
        assert_eq!(x_open.id, x_id_before, "stable id still preserved");
        assert!(!x_open.verify_bit, "verify_bit cleared when done→pending");
    }

    #[test]
    fn mission_meta_roundtrip() {
        let conn = mem();
        set_mission_meta(&conn, "k", "v").unwrap();
        assert_eq!(get_mission_meta(&conn, "k").unwrap().as_deref(), Some("v"));
        assert_eq!(get_mission_meta(&conn, "missing").unwrap(), None);
    }
}
