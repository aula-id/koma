//! SDLC graph: nodes + events in `messages.sqlite`.
//!
//! The graph is the session-scoped authority for mission tasks. TODO.md is a
//! projection only and must never override graph membership or status.

use anyhow::{bail, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::handoff::{HandoffStatus, ValidatedHandoff};

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
    /// Glob patterns for file paths this node owns (e.g. `["src/foo.rs", "src/bar/**"]`).
    /// During execute, write/edit/delete to paths matching a DIFFERENT active node's
    /// patterns is rejected at the tool intercept level.
    pub owned_paths: Vec<String>,
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
    /// Glob patterns for file paths this node owns. Empty means no path ownership.
    pub owned_paths: Vec<String>,
}

/// Result of applying a validated subagent handoff to the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffOutcome {
    pub children_added: bool,
    pub claim_must_be_cleared: bool,
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
            updated_at INTEGER NOT NULL,
            owned_paths TEXT NOT NULL DEFAULT '[]'
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
    // Migration: add owned_paths column if missing (existing databases).
    let has_col: bool = conn
        .prepare("SELECT owned_paths FROM sdlc_nodes LIMIT 0")
        .is_ok();
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE sdlc_nodes ADD COLUMN owned_paths TEXT NOT NULL DEFAULT '[]';",
        )?;
    }
    Ok(())
}

/// Serialize owned_paths to JSON text for SQLite storage.
fn owned_paths_to_json(paths: &[String]) -> String {
    serde_json::to_string(paths).unwrap_or_else(|_| "[]".to_string())
}

/// Deserialize owned_paths from JSON text stored in SQLite.
fn owned_paths_from_json(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
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
        hasher.update(b"|");
        hasher.update(owned_paths_to_json(&n.owned_paths).as_bytes());
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
        // Seal only via mission_verify: checklist cannot write bare done.
        if it.status == "done" {
            let existing_verified = ex.map(|e| e.verify_bit).unwrap_or(false);
            if !existing_verified {
                bail!(
                    "cannot set node '{}' to done without mission_verify (verify_bit required)",
                    it.title
                );
            }
        }
        let verify_bit = if it.status == "done" {
            ex.map(|e| e.verify_bit).unwrap_or(false)
        } else {
            false
        };
        let phase = ex.and_then(|e| e.phase.clone());
        let notes = ex.map(|e| e.notes.clone()).unwrap_or_default();
        // Use provided owned_paths from checklist input; preserve existing when not provided.
        let owned_paths = if it.owned_paths.is_empty() {
            ex.map(|e| e.owned_paths.clone()).unwrap_or_default()
        } else {
            it.owned_paths.clone()
        };

        // Prefer preserving an existing unambiguous parent_id when parent_title omitted.
        let parent_id = match (&parent_id, ex) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(e)) if it.parent_title.is_none() => e.parent_id.clone(),
            _ => parent_id,
        };

        tx.execute(
            "INSERT INTO sdlc_nodes (id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               parent_id = excluded.parent_id,
               title = excluded.title,
               status = excluded.status,
               phase = excluded.phase,
               notes = excluded.notes,
               verify_bit = excluded.verify_bit,
               updated_at = excluded.updated_at,
               owned_paths = excluded.owned_paths",
            rusqlite::params![
                id,
                parent_id,
                it.title,
                it.status,
                phase,
                notes,
                verify_bit as i64,
                now,
                owned_paths_to_json(&owned_paths)
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
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths
         FROM sdlc_nodes WHERE status NOT IN ('done', 'cancelled') ORDER BY updated_at",
    )
}

/// List all nodes that are done (sealed tasks).
pub fn list_sealed(conn: &Connection) -> Result<Vec<GraphTask>> {
    query_nodes(
        conn,
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths
         FROM sdlc_nodes WHERE status = 'done' ORDER BY updated_at",
    )
}

/// List all graph nodes.
pub fn list_all(conn: &Connection) -> Result<Vec<GraphTask>> {
    query_nodes(
        conn,
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths
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
            owned_paths: {
                let text: String = row.get(8)?;
                owned_paths_from_json(&text)
            },
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
        "SELECT id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths
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
            owned_paths: {
                let text: String = row.get(8)?;
                owned_paths_from_json(&text)
            },
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

    // One OPEN leaf claim at a time: any OTHER non-cancelled leaf already active.
    let other_active: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sdlc_nodes n
         WHERE n.status = 'active'
           AND n.id != ?1
           AND n.status != 'cancelled'
           AND NOT EXISTS (
             SELECT 1 FROM sdlc_nodes c
             WHERE c.parent_id = n.id AND c.status != 'cancelled'
           )",
        rusqlite::params![node_id],
        |r| r.get(0),
    )?;
    if other_active > 0 {
        bail!("another leaf is already active — verify or finish it first");
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

fn depth_of_node(nodes: &[GraphTask], node_id: &str) -> Result<usize> {
    let by_id: std::collections::HashMap<&str, &GraphTask> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut current = node_id;
    let mut seen = std::collections::HashSet::new();
    let mut depth = 0;

    loop {
        if !seen.insert(current) {
            bail!("cycle detected while finding depth of node '{node_id}'");
        }
        let node = by_id
            .get(current)
            .ok_or_else(|| anyhow::anyhow!("node '{current}' has an unknown parent"))?;
        depth += 1;
        match node.parent_id.as_deref() {
            Some(parent_id) => current = parent_id,
            None => return Ok(depth),
        }
    }
}

/// Apply one validated handoff as the graph's transaction-scoped authority.
///
/// Report `done` is deliberately not graph sealing; only verification may set
/// `done` or `verify_bit`. A rejected application performs no writes, so its
/// caller may use [`record_rejected_handoff`] to retain an audit record.
pub fn apply_handoff(
    conn: &Connection,
    expected_active_leaf_id: &str,
    handoff: &ValidatedHandoff,
) -> Result<HandoffOutcome> {
    let envelope = &handoff.envelope;
    if expected_active_leaf_id != envelope.node_id {
        bail!(
            "handoff node_id '{}' does not match expected active leaf '{}'",
            envelope.node_id,
            expected_active_leaf_id
        );
    }

    let accepted_audit = serde_json::to_string(&serde_json::json!({
        "outcome": "accepted",
        "expected_active_leaf_id": expected_active_leaf_id,
        "node_id": &envelope.node_id,
        "summary": &envelope.summary,
        "status": &envelope.status,
        "artifacts": &envelope.artifacts,
        "evidence_refs": &envelope.evidence_refs,
        "commit_shas": &envelope.commit_shas,
        "decisions": &envelope.decisions,
    }))?;

    let tx = conn.unchecked_transaction()?;
    let all = list_all(&tx)?;
    let claimed = all
        .iter()
        .find(|node| node.id == expected_active_leaf_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown node_id '{expected_active_leaf_id}'"))?;
    if claimed.status != "active" {
        bail!(
            "handoff node '{}' is stale: expected active leaf, found status {}",
            expected_active_leaf_id,
            claimed.status
        );
    }
    if all.iter().any(|node| {
        node.parent_id.as_deref() == Some(expected_active_leaf_id) && node.status != "cancelled"
    }) {
        bail!(
            "handoff node '{}' is stale: it is no longer a leaf",
            expected_active_leaf_id
        );
    }

    // Validate every child against the transaction snapshot before the first
    // write. This makes duplicate/depth failures all-or-nothing.
    let mut children = Vec::with_capacity(envelope.updates.child_proposals.len());
    let mut proposal_titles = std::collections::HashSet::new();
    let mut proposal_ids = std::collections::HashSet::new();
    if !envelope.updates.child_proposals.is_empty() {
        let depth = depth_of_node(&all, expected_active_leaf_id)?;
        if depth >= MAX_DEPTH {
            bail!(
                "cannot add child below '{}' at maximum graph depth {MAX_DEPTH}",
                expected_active_leaf_id
            );
        }
    }
    if envelope.updates.child_proposals.len() > 16 {
        bail!("too many child proposals (max 16)");
    }
    for proposal in &envelope.updates.child_proposals {
        let title = proposal.title.trim();
        if title.is_empty() || title.len() > 128 {
            bail!("invalid child proposal title");
        }
        let normalized_title = title.to_ascii_lowercase();
        if !proposal_titles.insert(normalized_title) {
            bail!("duplicate child proposal title '{title}'");
        }
        if proposal.note.as_ref().is_some_and(|note| note.len() > 200) {
            bail!("child proposal note exceeds 200 characters");
        }
        if all.iter().any(|node| {
            node.parent_id.as_deref() == Some(expected_active_leaf_id)
                && node.title.trim().eq_ignore_ascii_case(title)
        }) {
            bail!("duplicate sibling child title '{title}'");
        }

        let child_id = stable_id_for_title(&format!("{expected_active_leaf_id}:{title}"));
        if all.iter().any(|node| node.id == child_id) || !proposal_ids.insert(child_id.clone()) {
            bail!("child proposal id collision for title '{title}'");
        }
        children.push((
            child_id,
            title.to_string(),
            proposal.note.clone().unwrap_or_default(),
        ));
    }

    let now = now_secs();
    if let Some(note) = &envelope.updates.node_note {
        let updated = tx.execute(
            "UPDATE sdlc_nodes
             SET notes = CASE WHEN notes = '' THEN ?1 ELSE notes || char(10) || ?1 END,
                 updated_at = ?2
             WHERE id = ?3 AND status = 'active'",
            rusqlite::params![note, now, expected_active_leaf_id],
        )?;
        if updated != 1 {
            bail!("handoff node '{expected_active_leaf_id}' is no longer active");
        }
    }

    if envelope.status == HandoffStatus::Blocked {
        let updated = tx.execute(
            "UPDATE sdlc_nodes SET status = 'blocked', updated_at = ?1
             WHERE id = ?2 AND status = 'active'",
            rusqlite::params![now, expected_active_leaf_id],
        )?;
        if updated != 1 {
            bail!("handoff node '{expected_active_leaf_id}' is no longer active");
        }
    }

    for (child_id, title, note) in &children {
        tx.execute(
            "INSERT INTO sdlc_nodes
             (id, parent_id, title, status, phase, notes, verify_bit, updated_at, owned_paths)
             VALUES (?1, ?2, ?3, 'pending', NULL, ?4, 0, ?5, ?6)",
            rusqlite::params![
                child_id,
                expected_active_leaf_id,
                title,
                note,
                now,
                owned_paths_to_json(&claimed.owned_paths),
            ],
        )?;
    }

    tx.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
         VALUES (?1, 'handoff_accepted', ?2, ?3)",
        rusqlite::params![expected_active_leaf_id, accepted_audit, now],
    )?;
    tx.commit()?;

    let children_added = !children.is_empty();
    Ok(HandoffOutcome {
        children_added,
        claim_must_be_cleared: children_added || envelope.status == HandoffStatus::Blocked,
    })
}

/// Write only a rejected-handoff audit event; it never changes graph nodes.
///
/// Accepts an optional raw node id so a transport can record parse or validation
/// failures before a [`ValidatedHandoff`] exists.
pub fn record_rejected_handoff(
    conn: &Connection,
    expected_active_leaf_id: Option<&str>,
    reported_node_id: Option<&str>,
    reason: &str,
) -> Result<()> {
    let detail = serde_json::to_string(&serde_json::json!({
        "outcome": "rejected",
        "expected_active_leaf_id": expected_active_leaf_id,
        "node_id": reported_node_id,
        "reason": reason,
    }))?;
    conn.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at)
         VALUES (?1, 'handoff_rejected', ?2, ?3)",
        rusqlite::params![reported_node_id, detail, now_secs()],
    )?;
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
/// Sealing to `done` requires verify_bit already true (mission_verify only path).
pub fn update_node_status(conn: &Connection, node_id: &str, status: &str) -> Result<()> {
    let node =
        get_node(conn, node_id)?.ok_or_else(|| anyhow::anyhow!("unknown node '{node_id}'"))?;
    if node.status == status {
        return Ok(());
    }
    if status == "done" && !node.verify_bit {
        bail!("cannot seal node '{node_id}' to done without mission_verify (verify_bit is false)");
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
            owned_paths: n.owned_paths.clone(),
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
            } else if any_open && (parent.status == "done" || parent.verify_bit) {
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

/// Extract commit SHAs from the latest passing verify_evidence events for each node.
/// Returns a map of node_id → list of commit SHAs (extracted from detail strings containing `commit:`).
pub fn latest_verified_commit_shas(
    conn: &Connection,
    node_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut result: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for id in node_ids {
        result.insert(id.clone(), Vec::new());
    }
    for id in node_ids {
        let mut stmt = conn.prepare(
            "SELECT detail FROM sdlc_events
             WHERE node_id = ?1 AND kind = 'verify_evidence'
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;
        for detail in rows.flatten() {
            for part in detail.split('|') {
                let part = part.trim();
                if let Some(sha) = part.strip_prefix("commit:") {
                    let sha = sha.trim();
                    if !sha.is_empty()
                        && !result
                            .get(id)
                            .map(|v| v.contains(&sha.to_string()))
                            .unwrap_or(false)
                    {
                        if let Some(v) = result.get_mut(id) {
                            v.push(sha.to_string());
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}

/// Format OPEN/SEALED tree lines for the capsule (marks leaves).
/// When `commit_shas` is `Some`, verified nodes also show the first commit SHA.
pub fn format_tree(
    nodes: &[GraphTask],
    all: &[GraphTask],
    leaf_mark: bool,
    commit_shas: Option<&std::collections::HashMap<String, Vec<String>>>,
) -> String {
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

    fn verified_display(
        verify_bit: bool,
        commit_shas: Option<&std::collections::HashMap<String, Vec<String>>>,
        node_id: &str,
    ) -> String {
        if !verify_bit {
            return " (UNVERIFIED)".to_string();
        }
        let commit_suffix = commit_shas
            .and_then(|cs| cs.get(node_id))
            .and_then(|v| v.first())
            .map(|sha| {
                let short = if sha.len() > 7 { &sha[..7] } else { sha };
                format!(" (commit: {short})")
            })
            .unwrap_or_default();
        format!(" (verified){commit_suffix}")
    }

    fn walk(
        parent: Option<&str>,
        by_parent: &HashMap<Option<&str>, Vec<&GraphTask>>,
        child_ids: &std::collections::HashSet<&str>,
        depth: usize,
        leaf_mark: bool,
        commit_shas: Option<&std::collections::HashMap<String, Vec<String>>>,
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
                verified_display(n.verify_bit, commit_shas, &n.id)
            } else {
                "".to_string()
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
                commit_shas,
                out,
            );
        }
    }

    // Roots present in `nodes` OR whose descendants are in nodes.
    let node_ids_set: std::collections::HashSet<&str> =
        nodes.iter().map(|n| n.id.as_str()).collect();
    let mut out = String::new();
    // Emit roots (no parent or parent not in this set) that are in nodes.
    let roots: Vec<&GraphTask> = nodes
        .iter()
        .filter(|n| {
            n.parent_id
                .as_ref()
                .map(|p| !node_ids_set.contains(p.as_str()))
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
                verified_display(n.verify_bit, commit_shas, &n.id)
            } else {
                "".to_string()
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
            verified_display(r.verify_bit, commit_shas, &r.id)
        } else {
            "".to_string()
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
            commit_shas,
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

/// Check if writing to `target_path` would violate SDLC path ownership.
///
/// Returns `Ok(())` when the write is allowed, or `Err(message)` when denied.
///
/// Rules:
/// 1. Path matching a *different* active node's non-empty `owned_paths` → deny.
/// 2. When `active_node_id` is Some and that node has **non-empty** owned_paths,
///    the path must match **own** globs; otherwise deny.
/// 3. When the claim's owned_paths is empty (or node missing): allow any path
///    not foreign-owned (worktree sandbox still applies elsewhere).
///
/// - `active_node_id`: the node this actor is working on.
/// - `conn`: an open graph connection.
pub fn check_path_ownership(
    conn: &Connection,
    active_node_id: Option<&str>,
    target_path: &str,
) -> Result<(), String> {
    let all = list_all(conn).map_err(|e| format!("error: graph read failed: {e}"))?;

    // Foreign active owned_paths match → deny.
    for node in &all {
        if node.status != "active" || node.owned_paths.is_empty() {
            continue;
        }
        // Skip the actor's own node.
        if Some(node.id.as_str()) == active_node_id {
            continue;
        }
        let mut builder = globset::GlobSetBuilder::new();
        let mut patterns_valid = true;
        for pat in &node.owned_paths {
            match globset::Glob::new(pat) {
                Ok(g) => {
                    builder.add(g);
                }
                Err(_) => {
                    patterns_valid = false;
                    break;
                }
            }
        }
        if !patterns_valid {
            continue;
        }
        match builder.build() {
            Ok(gs) if gs.is_match(target_path) => {
                return Err(format!(
                    "error: '{target_path}' is owned by active node '{}' ({}) — \
                     write/edit/delete to another node's owned paths is forbidden",
                    node.id, node.title
                ));
            }
            _ => {}
        }
    }

    // Own non-empty owned_paths: path must match own globs.
    if let Some(aid) = active_node_id {
        if let Some(own) = all.iter().find(|n| n.id == aid) {
            if !own.owned_paths.is_empty() {
                let mut builder = globset::GlobSetBuilder::new();
                let mut any = false;
                for pat in &own.owned_paths {
                    if let Ok(g) = globset::Glob::new(pat) {
                        builder.add(g);
                        any = true;
                    }
                }
                if any {
                    match builder.build() {
                        Ok(gs) if gs.is_match(target_path) => {}
                        Ok(_) => {
                            return Err(format!(
                                "error: '{target_path}' is outside claimed leaf '{}' owned_paths \
                                 ({}) — stay inside the leaf's owned paths",
                                own.id,
                                own.owned_paths.join(", ")
                            ));
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    Ok(())
}

// --- SDLC edit history rail ---

/// Compact audit record for a single successful workspace mutation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditAuditRecord {
    pub tool: String,
    pub path: String,
    pub node_id: Option<String>,
    pub batch_id: String,
}

/// Compact summary record for a completed edit batch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditSummaryRecord {
    pub batch_id: String,
    pub purpose: String,
    pub paths: Vec<String>,
    pub node_id: Option<String>,
}

/// A single recent edit history entry for capsule projection.
#[derive(Debug, Clone)]
pub struct RecentEditEntry {
    pub kind: String,
    pub detail: String,
}

/// Append a deterministic edit_audit event. Best-effort: serialization or
/// persistence failure is silently ignored (caller must not treat this as fatal).
pub fn append_edit_audit(conn: &Connection, node_id: Option<&str>, record: &EditAuditRecord) {
    let Ok(detail) = serde_json::to_string(record) else {
        return;
    };
    let _ = conn.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'edit_audit', ?2, ?3)",
        rusqlite::params![node_id, detail, now_secs()],
    );
}

/// Append a best-effort edit_summary event. Best-effort like audit.
pub fn append_edit_summary(conn: &Connection, node_id: Option<&str>, record: &EditSummaryRecord) {
    let Ok(detail) = serde_json::to_string(record) else {
        return;
    };
    let _ = conn.execute(
        "INSERT INTO sdlc_events (node_id, kind, detail, created_at) VALUES (?1, 'edit_summary', ?2, ?3)",
        rusqlite::params![node_id, detail, now_secs()],
    );
}

/// Query the newest edit history entries for capsule projection.
/// Returns up to `limit` entries (default 20), preferring summaries.
/// When a batch has both audits and a summary, the summary is the
/// primary entry and audits without a matching summary are retained as fallback.
pub fn recent_edit_history(conn: &Connection, limit: usize) -> Result<Vec<RecentEditEntry>> {
    let limit = limit.clamp(1, 50);
    let mut stmt = conn.prepare(
        "SELECT kind, detail
         FROM sdlc_events
         WHERE kind IN ('edit_audit', 'edit_summary')
         ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(RecentEditEntry {
            kind: row.get(0)?,
            detail: row.get(1)?,
        })
    })?;

    let mut all: Vec<RecentEditEntry> = rows.filter_map(|r| r.ok()).collect();
    // Already DESC ordered; keep up to `limit` * 2 to allow summary-preferred
    // filtering, then cap at `limit`.
    all.truncate(limit * 2);
    all.reverse(); // chronological (oldest first)

    // Summary-preferred: collect batch_ids that have summaries, drop raw audits
    // for those batches.
    let summary_batches: std::collections::HashSet<String> = all
        .iter()
        .filter(|e| e.kind == "edit_summary")
        .filter_map(|e| {
            serde_json::from_str::<EditSummaryRecord>(&e.detail)
                .ok()
                .map(|r| r.batch_id)
        })
        .collect();

    let filtered: Vec<RecentEditEntry> = all
        .into_iter()
        .filter(|e| {
            if e.kind == "edit_audit" {
                if let Ok(rec) = serde_json::from_str::<EditAuditRecord>(&e.detail) {
                    return !summary_batches.contains(&rec.batch_id);
                }
            }
            true
        })
        .collect();

    // Take the last `limit` entries (most recent).
    let len = filtered.len();
    let skip = len.saturating_sub(limit);
    Ok(filtered.into_iter().skip(skip).collect())
}

#[cfg(test)]
#[path = "graph_test.rs"]
mod tests;
