//! SDLC graph: nodes + events in `messages.sqlite`.
//!
//! The graph is the session-scoped authority for mission tasks. TODO.md is a
//! projection only and must never override graph membership or status.

use anyhow::{bail, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// A single graph node (task) in the SDLC graph.
#[derive(Debug, Clone)]
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

/// Input row for checklist / mission_ready graph upserts.
#[derive(Debug, Clone)]
pub struct ChecklistNode {
    pub title: String,
    pub status: String,
    /// Parent identified by title (resolved within the same batch / existing graph).
    pub parent_title: Option<String>,
    /// Optional stable id; when set, preferred over title-derived id.
    pub id: Option<String>,
}

/// Current unix timestamp in seconds.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Ensure the SDLC tables exist in the given connection.
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

/// Derive a stable graph-node id from a title: `n-{slug}-{hash6}`.
pub fn stable_id_for_title(title: &str) -> String {
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

/// Canonical fingerprint of the full graph (ids, parents, titles, status, verify).
pub fn graph_fingerprint(conn: &Connection) -> Result<String> {
    let mut nodes = list_all(conn)?;
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hasher = Sha256::new();
    for n in &nodes {
        hasher.update(n.id.as_bytes());
        hasher.update(b"|");
        hasher.update(n.parent_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"|");
        hasher.update(n.title.as_bytes());
        hasher.update(b"|");
        hasher.update(n.status.as_bytes());
        hasher.update(b"|");
        hasher.update([n.verify_bit as u8]);
        hasher.update(b"\n");
    }
    let result = hasher.finalize();
    Ok(result[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// Maximum parent-chain depth (epic → story → task).
pub const MAX_DEPTH: usize = 3;

/// Validate checklist nodes for cycles, unknown parents, and depth.
pub fn validate_checklist_nodes(items: &[ChecklistNode]) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    let mut titles: HashSet<&str> = HashSet::new();
    for it in items {
        let t = it.title.trim();
        if t.is_empty() {
            bail!("checklist item title must be non-empty");
        }
        if !titles.insert(t) {
            bail!("duplicate checklist title '{t}'");
        }
    }

    let title_set: HashSet<&str> = items.iter().map(|i| i.title.trim()).collect();
    let parent_of: HashMap<&str, Option<&str>> = items
        .iter()
        .map(|i| {
            (
                i.title.trim(),
                i.parent_title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty()),
            )
        })
        .collect();

    for it in items {
        let title = it.title.trim();
        if let Some(p) = it
            .parent_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !title_set.contains(p) {
                bail!("unknown parent_title '{p}' for '{title}'");
            }
            if p == title {
                bail!("node '{title}' cannot parent itself");
            }
        }
        // Depth + cycle via walk.
        let mut seen = HashSet::new();
        let mut cur = Some(title);
        let mut depth = 1usize;
        while let Some(c) = cur {
            if !seen.insert(c) {
                bail!("cycle detected involving '{title}'");
            }
            cur = parent_of.get(c).copied().flatten();
            if cur.is_some() {
                depth += 1;
                if depth > MAX_DEPTH {
                    bail!(
                        "hierarchy deeper than {MAX_DEPTH} levels (epic→story→task) at '{title}'"
                    );
                }
            }
        }
    }
    Ok(())
}

/// Replace the open node set from structured checklist nodes.
///
/// - Match existing nodes by explicit id, else exact title; preserve parent when possible.
/// - Open nodes whose titles are absent from `items` are cancelled (not deleted).
/// - Done/cancelled history not listed is left untouched.
/// - Sealed law: done+verified stays verified when still listed as done.
/// - Status leaving done clears verify_bit.
/// - After write, parents roll up from children.
pub fn replace_nodes_from_checklist(conn: &Connection, items: &[ChecklistNode]) -> Result<()> {
    validate_checklist_nodes(items)?;
    let now = now_secs();
    let all = list_all(conn)?;

    use std::collections::HashMap;
    let mut by_title: HashMap<String, GraphTask> = HashMap::new();
    let mut by_id: HashMap<String, GraphTask> = HashMap::new();
    for node in &all {
        by_id.insert(node.id.clone(), node.clone());
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

    // First pass: assign ids for every item (needed to resolve parent_id).
    let mut assigned: Vec<(ChecklistNode, String)> = Vec::with_capacity(items.len());
    let mut kept_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for it in items {
        let id = if let Some(ref explicit) = it.id {
            if !explicit.trim().is_empty() {
                explicit.trim().to_string()
            } else if let Some(ex) = by_title.get(&it.title) {
                ex.id.clone()
            } else {
                stable_id_for_title(&it.title)
            }
        } else if let Some(ex) = by_title.get(&it.title) {
            ex.id.clone()
        } else {
            stable_id_for_title(&it.title)
        };
        kept_ids.insert(id.clone());
        assigned.push((it.clone(), id));
    }

    let title_to_id: HashMap<String, String> = assigned
        .iter()
        .map(|(it, id)| (it.title.clone(), id.clone()))
        .collect();

    let tx = conn.unchecked_transaction()?;

    for (it, id) in &assigned {
        let parent_id = it
            .parent_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|pt| title_to_id.get(pt).cloned());

        let ex = by_id.get(id).or_else(|| by_title.get(&it.title));
        let verify_bit = if it.status == "done" {
            ex.map(|e| e.verify_bit).unwrap_or(false)
        } else {
            false
        };
        let phase = ex.and_then(|e| e.phase.clone());
        let notes = ex.map(|e| e.notes.clone()).unwrap_or_default();

        // Prefer preserving an existing unambiguous parent_id when parent_title omitted.
        let parent_id = match (&parent_id, ex) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(e)) if it.parent_title.is_none() => e.parent_id.clone(),
            _ => parent_id,
        };

        tx.execute(
            "INSERT INTO sdlc_nodes (id, parent_id, title, status, phase, notes, verify_bit, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               parent_id = excluded.parent_id,
               title = excluded.title,
               status = excluded.status,
               phase = excluded.phase,
               notes = excluded.notes,
               verify_bit = excluded.verify_bit,
               updated_at = excluded.updated_at",
            rusqlite::params![
                id,
                parent_id,
                it.title,
                it.status,
                phase,
                notes,
                verify_bit as i64,
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'upsert', ?2, ?3)",
            rusqlite::params![id, it.status, now],
        )?;
    }

    // Cancel open nodes whose titles/ids are absent.
    for node in &all {
        if node.status == "done" || node.status == "cancelled" {
            continue;
        }
        if !kept_ids.contains(&node.id) {
            let n = tx.execute(
                "UPDATE sdlc_nodes SET status = 'cancelled', updated_at = ?1 WHERE id = ?2 AND status NOT IN ('done','cancelled')",
                rusqlite::params![now, node.id],
            )?;
            if n > 0 {
                tx.execute(
                    "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'status_change', 'cancelled', ?2)",
                    rusqlite::params![node.id, now],
                )?;
            }
        }
    }

    // Parent rollup must share this transaction so checklist/cancel + parent
    // status/events never partially commit.
    rollup_parents_in_tx(&tx)?;
    tx.commit()?;
    Ok(())
}

/// True when the node has no non-cancelled children.
pub fn is_leaf(conn: &Connection, node_id: &str) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM sdlc_nodes WHERE parent_id = ?1 AND status != 'cancelled'",
    )?;
    let n: i64 = stmt.query_row([node_id], |r| r.get(0))?;
    Ok(n == 0)
}

/// List open leaf nodes only (not done/cancelled, no live children).
pub fn list_open_leaves(conn: &Connection) -> Result<Vec<GraphTask>> {
    let open = list_open(conn)?;
    let mut out = Vec::new();
    for n in open {
        if is_leaf(conn, &n.id)? {
            out.push(n);
        }
    }
    Ok(out)
}

/// List all nodes that are NOT done or cancelled (open tasks).
pub fn list_open(conn: &Connection) -> Result<Vec<GraphTask>> {
    query_nodes(
        conn,
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE status NOT IN ('done', 'cancelled') ORDER BY updated_at",
    )
}

/// List all nodes that are done (sealed tasks).
pub fn list_sealed(conn: &Connection) -> Result<Vec<GraphTask>> {
    query_nodes(
        conn,
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE status = 'done' ORDER BY updated_at",
    )
}

/// List all graph nodes.
pub fn list_all(conn: &Connection) -> Result<Vec<GraphTask>> {
    query_nodes(
        conn,
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes ORDER BY updated_at",
    )
}

fn query_nodes(conn: &Connection, sql: &str) -> Result<Vec<GraphTask>> {
    let mut stmt = conn.prepare(sql)?;
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

/// Get one node by id.
pub fn get_node(conn: &Connection, node_id: &str) -> Result<Option<GraphTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at
         FROM sdlc_nodes WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map([node_id], |row| {
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
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// Atomically claim a pending/blocked leaf → active.
///
/// Exclusive: a leaf that is already `active` cannot be claimed again (second
/// concurrent/deferred claim fails). Membership + leaf-ness are checked inside
/// the same transaction as the status CAS so the race window stays closed.
pub fn claim_leaf(conn: &Connection, node_id: &str) -> Result<()> {
    let now = now_secs();
    let tx = conn.unchecked_transaction()?;

    let status: Option<String> = {
        let mut stmt = tx.prepare("SELECT status FROM sdlc_nodes WHERE id = ?1")?;
        let mut rows = stmt.query(rusqlite::params![node_id])?;
        match rows.next()? {
            Some(r) => Some(r.get(0)?),
            None => None,
        }
    };
    let Some(status) = status else {
        bail!("unknown node_id '{node_id}'");
    };

    let child_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sdlc_nodes
         WHERE parent_id = ?1 AND status != 'cancelled'",
        rusqlite::params![node_id],
        |r| r.get(0),
    )?;
    if child_count > 0 {
        bail!("node '{node_id}' is not a leaf");
    }

    if status == "active" {
        bail!("leaf '{node_id}' already claimed (active)");
    }
    if status != "pending" && status != "blocked" {
        bail!("leaf '{node_id}' not claimable (status {status})");
    }

    // CAS: only pending/blocked → active. A concurrent claim loses here.
    let n = tx.execute(
        "UPDATE sdlc_nodes SET status = 'active', updated_at = ?1
         WHERE id = ?2 AND status IN ('pending', 'blocked')",
        rusqlite::params![now, node_id],
    )?;
    if n == 0 {
        bail!("leaf '{node_id}' not claimable (race or sealed)");
    }
    tx.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'claim', 'active', ?2)",
        rusqlite::params![node_id, now],
    )?;
    tx.commit()?;
    Ok(())
}

/// Set verify_bit on a LEAF only. Pass seals done; fail reopens leaf + ancestors.
/// Optional evidence is recorded in the SAME transaction as the verify mutation
/// so a failed verify never leaves a dangling evidence event (or vice versa).
pub fn set_verify_bit_with_evidence(
    conn: &Connection,
    node_id: &str,
    verified: bool,
    evidence: Option<&str>,
) -> Result<()> {
    let node =
        get_node(conn, node_id)?.ok_or_else(|| anyhow::anyhow!("unknown node '{node_id}'"))?;
    if node.status == "cancelled" {
        bail!("cannot verify cancelled node '{node_id}'");
    }
    if !is_leaf(conn, node_id)? {
        bail!("verify is leaf-only; '{node_id}' has children");
    }

    let now = now_secs();
    let tx = conn.unchecked_transaction()?;
    let bit: i64 = if verified { 1 } else { 0 };
    let n = tx.execute(
        "UPDATE sdlc_nodes SET verify_bit = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![bit, now, node_id],
    )?;
    if n == 0 {
        bail!("unknown node_id '{node_id}'");
    }
    if let Some(ev) = evidence {
        tx.execute(
            "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'verify_evidence', ?2, ?3)",
            rusqlite::params![node_id, ev, now],
        )?;
    }
    let detail = if verified {
        "verified"
    } else {
        "verify_failed"
    };
    tx.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'verify', ?2, ?3)",
        rusqlite::params![node_id, detail, now],
    )?;
    if verified {
        tx.execute(
            "UPDATE sdlc_nodes SET status = 'done', updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, node_id],
        )?;
        tx.execute(
            "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'status_change', 'done', ?2)",
            rusqlite::params![node_id, now],
        )?;
    } else {
        // Reopen leaf.
        tx.execute(
            "UPDATE sdlc_nodes SET status = 'active', updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, node_id],
        )?;
        tx.execute(
            "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'status_change', 'active', ?2)",
            rusqlite::params![node_id, now],
        )?;
        // Reopen ancestors (clear their done/verify).
        let mut parent = node.parent_id.clone();
        while let Some(pid) = parent {
            tx.execute(
                "UPDATE sdlc_nodes SET status = 'active', verify_bit = 0, updated_at = ?1
                 WHERE id = ?2 AND status != 'cancelled'",
                rusqlite::params![now, pid],
            )?;
            tx.execute(
                "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
                 VALUES (?1, 'ancestor_reopen', 'child_verify_failed', ?2)",
                rusqlite::params![pid, now],
            )?;
            parent = tx
                .query_row(
                    "SELECT parent_id FROM sdlc_nodes WHERE id = ?1",
                    [&pid],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None);
        }
    }
    // Verification + parent rollup share one transaction so a failed rollup
    // cannot leave a verified child without the matching parent state/events.
    if verified {
        rollup_parents_in_tx(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

/// Atomic status update + event for a known node. Clears verify when leaving done.
pub fn update_node_status(conn: &Connection, node_id: &str, status: &str) -> Result<()> {
    let node =
        get_node(conn, node_id)?.ok_or_else(|| anyhow::anyhow!("unknown node '{node_id}'"))?;
    if node.status == status {
        return Ok(());
    }
    let now = now_secs();
    let verify: i64 = if status == "done" {
        node.verify_bit as i64
    } else {
        0
    };
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE sdlc_nodes SET status = ?1, verify_bit = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![status, verify, now, node_id],
    )?;
    if n == 0 {
        bail!("unknown node_id '{node_id}'");
    }
    tx.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'status_change', ?2, ?3)",
        rusqlite::params![node_id, status, now],
    )?;
    // Status change + parent rollup are one transaction.
    rollup_parents_in_tx(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Snapshot non-cancelled nodes as checklist rows (for rollback after failed mission save).
pub fn snapshot_checklist(conn: &Connection) -> Result<Vec<ChecklistNode>> {
    let nodes = list_all(conn)?;
    let by_id: std::collections::HashMap<String, GraphTask> =
        nodes.iter().map(|n| (n.id.clone(), n.clone())).collect();
    let mut out = Vec::new();
    for n in nodes.iter().filter(|n| n.status != "cancelled") {
        let parent_title = n
            .parent_id
            .as_ref()
            .and_then(|pid| by_id.get(pid))
            .map(|p| p.title.clone());
        out.push(ChecklistNode {
            title: n.title.clone(),
            status: n.status.clone(),
            parent_title,
            id: Some(n.id.clone()),
        });
    }
    Ok(out)
}

/// Roll parent status up from children:
/// - all children done+verified → parent done+verified
/// - any child active/pending/blocked → parent active (and verify cleared)
/// - cancelled children ignored
///
/// Callers that mutate children (verify/status/checklist) MUST invoke this
/// before `commit` so child + parent graph state cannot partially land.
fn rollup_parents_in_tx(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    // `Transaction` derefs to `Connection`; reads see this tx's uncommitted rows.
    let all = list_all(tx)?;
    // Process bottom-up: deeper nodes first by walking parents repeatedly.
    let mut by_id: std::collections::HashMap<String, GraphTask> =
        all.iter().map(|n| (n.id.clone(), n.clone())).collect();

    // Collect parent ids that have children.
    let mut parents: std::collections::HashSet<String> = std::collections::HashSet::new();
    for n in &all {
        if let Some(ref p) = n.parent_id {
            parents.insert(p.clone());
        }
    }

    // Multiple passes for multi-level trees.
    for _ in 0..MAX_DEPTH {
        let now = now_secs();
        let mut changed = false;
        for pid in &parents {
            let children: Vec<&GraphTask> = by_id
                .values()
                .filter(|n| n.parent_id.as_deref() == Some(pid.as_str()) && n.status != "cancelled")
                .collect();
            if children.is_empty() {
                continue;
            }
            let all_verified_done = children.iter().all(|c| c.status == "done" && c.verify_bit);
            let any_open = children
                .iter()
                .any(|c| matches!(c.status.as_str(), "pending" | "active" | "blocked"));

            let Some(parent) = by_id.get(pid).cloned() else {
                continue;
            };
            if parent.status == "cancelled" {
                continue;
            }

            if all_verified_done {
                if parent.status != "done" || !parent.verify_bit {
                    tx.execute(
                        "UPDATE sdlc_nodes SET status = 'done', verify_bit = 1, updated_at = ?1 WHERE id = ?2",
                        rusqlite::params![now, pid],
                    )?;
                    tx.execute(
                        "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
                         VALUES (?1, 'rollup', 'all_children_verified', ?2)",
                        rusqlite::params![pid, now],
                    )?;
                    if let Some(p) = by_id.get_mut(pid) {
                        p.status = "done".into();
                        p.verify_bit = true;
                    }
                    changed = true;
                }
            } else if any_open {
                if parent.status == "done" || parent.verify_bit {
                    tx.execute(
                        "UPDATE sdlc_nodes SET status = 'active', verify_bit = 0, updated_at = ?1 WHERE id = ?2",
                        rusqlite::params![now, pid],
                    )?;
                    tx.execute(
                        "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
                         VALUES (?1, 'rollup', 'child_open', ?2)",
                        rusqlite::params![pid, now],
                    )?;
                    if let Some(p) = by_id.get_mut(pid) {
                        p.status = "active".into();
                        p.verify_bit = false;
                    }
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    Ok(())
}

/// List done nodes without verify evidence (false-done). Leaves only —
/// parents are rolled up, never independently verified.
pub fn list_false_done(conn: &Connection) -> Result<Vec<GraphTask>> {
    let sealed = list_sealed(conn)?;
    let mut out = Vec::new();
    for n in sealed {
        if n.verify_bit {
            continue;
        }
        if is_leaf(conn, &n.id)? {
            out.push(n);
        }
    }
    Ok(out)
}

/// Reopen false-done leaves (and their ancestors). Cancellation is never
/// treated as verification.
pub fn reopen_false_done(conn: &Connection) -> Result<Vec<GraphTask>> {
    let false_done = list_false_done(conn)?;
    let mut reopened = Vec::new();
    for node in &false_done {
        // Use set_verify_bit_with_evidence(false) path semantics without requiring prior verify.
        let now = now_secs();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE sdlc_nodes SET status = 'active', verify_bit = 0, updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, node.id],
        )?;
        tx.execute(
            "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
             VALUES (?1, 'false_done_reopen', 'reopened: done without verify evidence', ?2)",
            rusqlite::params![node.id, now],
        )?;
        let mut parent = node.parent_id.clone();
        while let Some(pid) = parent {
            tx.execute(
                "UPDATE sdlc_nodes SET status = 'active', verify_bit = 0, updated_at = ?1
                 WHERE id = ?2 AND status != 'cancelled'",
                rusqlite::params![now, pid],
            )?;
            tx.execute(
                "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
                 VALUES (?1, 'ancestor_reopen', 'child_false_done', ?2)",
                rusqlite::params![pid, now],
            )?;
            parent = tx
                .query_row(
                    "SELECT parent_id FROM sdlc_nodes WHERE id = ?1",
                    [&pid],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None);
        }
        tx.commit()?;
        reopened.push(node.clone());
    }
    Ok(reopened)
}

/// True when every non-cancelled leaf is done+verified.
pub fn all_required_leaves_verified(conn: &Connection) -> Result<bool> {
    let all = list_all(conn)?;
    let mut any_leaf = false;
    for n in &all {
        if n.status == "cancelled" {
            continue;
        }
        if !is_leaf(conn, &n.id)? {
            continue;
        }
        any_leaf = true;
        if n.status != "done" || !n.verify_bit {
            return Ok(false);
        }
    }
    Ok(any_leaf)
}

/// Format OPEN/SEALED tree lines for the capsule (marks leaves).
pub fn format_tree(nodes: &[GraphTask], all: &[GraphTask], leaf_mark: bool) -> String {
    use std::collections::HashMap;
    let by_parent: HashMap<Option<&str>, Vec<&GraphTask>> = {
        let mut m: HashMap<Option<&str>, Vec<&GraphTask>> = HashMap::new();
        for n in nodes {
            m.entry(n.parent_id.as_deref()).or_default().push(n);
        }
        m
    };
    let child_ids: std::collections::HashSet<&str> = all
        .iter()
        .filter(|n| n.status != "cancelled")
        .filter_map(|n| n.parent_id.as_deref())
        .collect();

    fn walk(
        parent: Option<&str>,
        by_parent: &HashMap<Option<&str>, Vec<&GraphTask>>,
        child_ids: &std::collections::HashSet<&str>,
        depth: usize,
        leaf_mark: bool,
        out: &mut String,
    ) {
        let Some(kids) = by_parent.get(&parent) else {
            return;
        };
        for n in kids {
            let indent = "  ".repeat(depth);
            let leaf = if leaf_mark && !child_ids.contains(n.id.as_str()) {
                " ← leaf"
            } else {
                ""
            };
            let v = if n.status == "done" {
                if n.verify_bit {
                    " (verified)"
                } else {
                    " (UNVERIFIED)"
                }
            } else {
                ""
            };
            out.push_str(&format!(
                "{indent}- [{}] {} ({}){v}{leaf}\n",
                n.status, n.title, n.id
            ));
            walk(
                Some(n.id.as_str()),
                by_parent,
                child_ids,
                depth + 1,
                leaf_mark,
                out,
            );
        }
    }

    // Roots present in `nodes` OR whose descendants are in nodes.
    let node_ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut out = String::new();
    // Emit roots (no parent or parent not in this set) that are in nodes.
    let roots: Vec<&GraphTask> = nodes
        .iter()
        .filter(|n| {
            n.parent_id
                .as_ref()
                .map(|p| !node_ids.contains(p.as_str()))
                .unwrap_or(true)
        })
        .collect();
    // If hierarchy spans sealed/open, fall back to flat for missing parents.
    if roots.is_empty() {
        for n in nodes {
            let leaf = if leaf_mark && !child_ids.contains(n.id.as_str()) {
                " ← leaf"
            } else {
                ""
            };
            let v = if n.status == "done" {
                if n.verify_bit {
                    " (verified)"
                } else {
                    " (UNVERIFIED)"
                }
            } else {
                ""
            };
            out.push_str(&format!(
                "- [{}] {} ({}){v}{leaf}\n",
                n.status, n.title, n.id
            ));
        }
        return out;
    }
    for r in roots {
        let leaf = if leaf_mark && !child_ids.contains(r.id.as_str()) {
            " ← leaf"
        } else {
            ""
        };
        let v = if r.status == "done" {
            if r.verify_bit {
                " (verified)"
            } else {
                " (UNVERIFIED)"
            }
        } else {
            ""
        };
        out.push_str(&format!(
            "- [{}] {} ({}){v}{leaf}\n",
            r.status, r.title, r.id
        ));
        walk(
            Some(r.id.as_str()),
            &by_parent,
            &child_ids,
            1,
            leaf_mark,
            &mut out,
        );
    }
    out
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

/// Record tool-round timestamp + current graph fingerprint for stall detection.
pub fn stamp_tool_round(conn: &Connection) -> Result<()> {
    let now = now_secs().to_string();
    set_mission_meta(conn, super::decompose::META_LAST_TOOL_ROUND_AT, &now)?;
    let fp = graph_fingerprint(conn)?;
    set_mission_meta(conn, super::decompose::META_LAST_GRAPH_FINGERPRINT, &fp)?;
    Ok(())
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

    fn flat(items: &[(&str, &str)]) -> Vec<ChecklistNode> {
        items
            .iter()
            .map(|(t, s)| ChecklistNode {
                title: (*t).into(),
                status: (*s).into(),
                parent_title: None,
                id: None,
            })
            .collect()
    }

    #[test]
    fn verify_bit_and_false_done_reopen() {
        let conn = mem();
        replace_nodes_from_checklist(&conn, &flat(&[("a", "done"), ("b", "pending")])).unwrap();
        let false_done = list_false_done(&conn).unwrap();
        assert_eq!(false_done.len(), 1);
        assert_eq!(false_done[0].title, "a");

        set_verify_bit_with_evidence(&conn, &false_done[0].id, true, None).unwrap();
        assert!(list_false_done(&conn).unwrap().is_empty());

        set_verify_bit_with_evidence(&conn, &false_done[0].id, false, None).unwrap();
        let open = list_open(&conn).unwrap();
        assert!(open
            .iter()
            .any(|n| n.id == false_done[0].id && n.status == "active"));
    }

    #[test]
    fn stable_id_and_verify_preserve() {
        let conn = mem();
        replace_nodes_from_checklist(&conn, &flat(&[("x", "done"), ("y", "pending")])).unwrap();
        let sealed = list_sealed(&conn).unwrap();
        let x_node = sealed.iter().find(|n| n.title == "x").unwrap();
        set_verify_bit_with_evidence(&conn, &x_node.id, true, None).unwrap();
        let x_id_before = x_node.id.clone();

        replace_nodes_from_checklist(&conn, &flat(&[("x", "done"), ("y", "pending")])).unwrap();
        let sealed = list_sealed(&conn).unwrap();
        let x_node2 = sealed.iter().find(|n| n.title == "x").unwrap();
        assert_eq!(x_node2.id, x_id_before);
        assert!(x_node2.verify_bit);

        replace_nodes_from_checklist(&conn, &flat(&[("x", "pending"), ("y", "done")])).unwrap();
        let open = list_open(&conn).unwrap();
        let x_open = open.iter().find(|n| n.title == "x").unwrap();
        assert_eq!(x_open.id, x_id_before);
        assert!(!x_open.verify_bit);
    }

    #[test]
    fn hierarchy_parent_rollup_and_leaf_only_verify() {
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "epic".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "leaf-a".into(),
                    status: "pending".into(),
                    parent_title: Some("epic".into()),
                    id: None,
                },
                ChecklistNode {
                    title: "leaf-b".into(),
                    status: "pending".into(),
                    parent_title: Some("epic".into()),
                    id: None,
                },
            ],
        )
        .unwrap();
        let all = list_all(&conn).unwrap();
        let epic = all.iter().find(|n| n.title == "epic").unwrap();
        let a = all.iter().find(|n| n.title == "leaf-a").unwrap();
        let b = all.iter().find(|n| n.title == "leaf-b").unwrap();

        assert!(set_verify_bit_with_evidence(&conn, &epic.id, true, None).is_err());

        set_verify_bit_with_evidence(&conn, &a.id, true, None).unwrap();
        set_verify_bit_with_evidence(&conn, &b.id, true, None).unwrap();
        let epic2 = get_node(&conn, &epic.id).unwrap().unwrap();
        assert_eq!(epic2.status, "done");
        assert!(epic2.verify_bit);
        // Parent rollup event must exist alongside verified children.
        let rollup_n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'rollup' AND detail = 'all_children_verified'",
                [&epic.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(rollup_n >= 1, "expected atomic parent rollup event");

        // Invalidate child → ancestors reopen.
        set_verify_bit_with_evidence(&conn, &a.id, false, None).unwrap();
        let epic3 = get_node(&conn, &epic.id).unwrap().unwrap();
        assert_eq!(epic3.status, "active");
        assert!(!epic3.verify_bit);
    }

    #[test]
    fn verify_and_parent_rollup_are_one_transaction() {
        // Behavioral atomicity: after successful verify of the last open child,
        // parent is done+verified AND a rollup event exists — there is no window
        // where the child is done without parent rollup (same commit).
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "p".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "c".into(),
                    status: "pending".into(),
                    parent_title: Some("p".into()),
                    id: None,
                },
            ],
        )
        .unwrap();
        let all = list_all(&conn).unwrap();
        let p = all.iter().find(|n| n.title == "p").unwrap().id.clone();
        let c = all.iter().find(|n| n.title == "c").unwrap().id.clone();
        set_verify_bit_with_evidence(&conn, &c, true, Some("tests green")).unwrap();
        let parent = get_node(&conn, &p).unwrap().unwrap();
        let child = get_node(&conn, &c).unwrap().unwrap();
        assert_eq!(child.status, "done");
        assert!(child.verify_bit);
        assert_eq!(parent.status, "done");
        assert!(parent.verify_bit);
        let ev: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'verify_evidence'",
                [&c],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ev, 1);
        let roll: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sdlc_events WHERE node_id = ?1 AND kind = 'rollup'",
                [&p],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(roll, 1);
    }

    #[test]
    fn update_status_rollup_reopens_done_parent() {
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "p".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "c".into(),
                    status: "pending".into(),
                    parent_title: Some("p".into()),
                    id: None,
                },
            ],
        )
        .unwrap();
        let all = list_all(&conn).unwrap();
        let p = all.iter().find(|n| n.title == "p").unwrap().id.clone();
        let c = all.iter().find(|n| n.title == "c").unwrap().id.clone();
        set_verify_bit_with_evidence(&conn, &c, true, None).unwrap();
        assert_eq!(get_node(&conn, &p).unwrap().unwrap().status, "done");
        // Reopen child via status update — parent must roll open in same path.
        update_node_status(&conn, &c, "active").unwrap();
        let parent = get_node(&conn, &p).unwrap().unwrap();
        assert_eq!(parent.status, "active");
        assert!(!parent.verify_bit);
    }

    #[test]
    fn rejects_cycle_and_deep_tree() {
        let err = validate_checklist_nodes(&[
            ChecklistNode {
                title: "a".into(),
                status: "pending".into(),
                parent_title: Some("b".into()),
                id: None,
            },
            ChecklistNode {
                title: "b".into(),
                status: "pending".into(),
                parent_title: Some("a".into()),
                id: None,
            },
        ])
        .unwrap_err();
        assert!(err.to_string().contains("cycle"));

        let err = validate_checklist_nodes(&[
            ChecklistNode {
                title: "e".into(),
                status: "pending".into(),
                parent_title: None,
                id: None,
            },
            ChecklistNode {
                title: "s".into(),
                status: "pending".into(),
                parent_title: Some("e".into()),
                id: None,
            },
            ChecklistNode {
                title: "t".into(),
                status: "pending".into(),
                parent_title: Some("s".into()),
                id: None,
            },
            ChecklistNode {
                title: "x".into(),
                status: "pending".into(),
                parent_title: Some("t".into()),
                id: None,
            },
        ])
        .unwrap_err();
        assert!(err.to_string().contains("deeper"));
    }

    #[test]
    fn cancellation_is_not_verification() {
        let conn = mem();
        replace_nodes_from_checklist(&conn, &flat(&[("a", "cancelled"), ("b", "done")])).unwrap();
        // only b is false-done leaf
        let fd = list_false_done(&conn).unwrap();
        assert_eq!(fd.len(), 1);
        assert_eq!(fd[0].title, "b");
        assert!(!all_required_leaves_verified(&conn).unwrap());
    }

    #[test]
    fn mission_meta_roundtrip() {
        let conn = mem();
        set_mission_meta(&conn, "k", "v").unwrap();
        assert_eq!(get_mission_meta(&conn, "k").unwrap().as_deref(), Some("v"));
        assert_eq!(get_mission_meta(&conn, "missing").unwrap(), None);
    }

    #[test]
    fn update_node_status_unknown_fails() {
        let conn = mem();
        assert!(update_node_status(&conn, "nope", "done").is_err());
    }

    #[test]
    fn parent_identity_preserved_on_title_match() {
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "p".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "c".into(),
                    status: "pending".into(),
                    parent_title: Some("p".into()),
                    id: None,
                },
            ],
        )
        .unwrap();
        let c1 = list_all(&conn)
            .unwrap()
            .into_iter()
            .find(|n| n.title == "c")
            .unwrap();
        let pid = c1.parent_id.clone().unwrap();

        // Re-upsert without parent_title — parent_id preserved.
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "p".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "c".into(),
                    status: "active".into(),
                    parent_title: None,
                    id: None,
                },
            ],
        )
        .unwrap();
        let c2 = list_all(&conn)
            .unwrap()
            .into_iter()
            .find(|n| n.title == "c")
            .unwrap();
        assert_eq!(c2.parent_id.as_deref(), Some(pid.as_str()));
        assert_eq!(c2.id, c1.id);
    }

    #[test]
    fn verify_with_evidence_is_atomic_on_unknown() {
        let conn = mem();
        // Unknown node: no evidence row may be left behind.
        assert!(set_verify_bit_with_evidence(&conn, "nope", true, Some("ev")).is_err());
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sdlc_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn update_node_status_atomic_and_clears_verify_when_reopening() {
        let conn = mem();
        replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
        let id = list_all(&conn).unwrap()[0].id.clone();
        set_verify_bit_with_evidence(&conn, &id, true, None).unwrap();
        let done = get_node(&conn, &id).unwrap().unwrap();
        assert_eq!(done.status, "done");
        assert!(done.verify_bit);

        update_node_status(&conn, &id, "active").unwrap();
        let reopened = get_node(&conn, &id).unwrap().unwrap();
        assert_eq!(reopened.status, "active");
        assert!(!reopened.verify_bit);

        let kinds: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT kind FROM sdlc_events WHERE node_id = ?1 ORDER BY id")
                .unwrap();
            stmt.query_map([&id], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(kinds.iter().any(|k| k == "status_change"));
    }

    #[test]
    fn snapshot_checklist_roundtrips_for_restore() {
        let conn = mem();
        replace_nodes_from_checklist(
            &conn,
            &[
                ChecklistNode {
                    title: "epic".into(),
                    status: "pending".into(),
                    parent_title: None,
                    id: None,
                },
                ChecklistNode {
                    title: "leaf".into(),
                    status: "active".into(),
                    parent_title: Some("epic".into()),
                    id: None,
                },
            ],
        )
        .unwrap();
        let snap = snapshot_checklist(&conn).unwrap();
        assert_eq!(snap.len(), 2);
        // Mutate graph away from snapshot.
        replace_nodes_from_checklist(&conn, &flat(&[("other", "pending")])).unwrap();
        assert_eq!(list_open(&conn).unwrap().len(), 1);
        assert_eq!(list_open(&conn).unwrap()[0].title, "other");
        // Restore.
        replace_nodes_from_checklist(&conn, &snap).unwrap();
        let titles: std::collections::HashSet<_> = list_open(&conn)
            .unwrap()
            .into_iter()
            .map(|n| n.title)
            .collect();
        assert!(titles.contains("epic"));
        assert!(titles.contains("leaf"));
        assert!(!titles.contains("other"));
    }

    #[test]
    fn claim_leaf_is_exclusive_second_claim_fails() {
        let conn = mem();
        replace_nodes_from_checklist(&conn, &flat(&[("a", "pending")])).unwrap();
        let id = list_all(&conn).unwrap()[0].id.clone();
        claim_leaf(&conn, &id).unwrap();
        let n = get_node(&conn, &id).unwrap().unwrap();
        assert_eq!(n.status, "active");
        // Second claim on already-active leaf must fail closed.
        let err = claim_leaf(&conn, &id).unwrap_err().to_string();
        assert!(
            err.contains("already claimed") || err.contains("not claimable"),
            "unexpected err: {err}"
        );
        // Still a single active node.
        assert_eq!(get_node(&conn, &id).unwrap().unwrap().status, "active");
        // Done leaf also not claimable.
        update_node_status(&conn, &id, "done").unwrap();
        assert!(claim_leaf(&conn, &id).is_err());
    }
}
