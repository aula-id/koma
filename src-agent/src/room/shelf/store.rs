//! The SQLite-backed shelf: one table of `key -> (content, revision)`.
//!
//! One connection behind a mutex, and every tool call runs in one transaction,
//! so a merge is a plain read-modify-write with no retry loop. Plain writes use
//! `expected_revision` as a compare-and-swap; trajectory and asset writes merge
//! into whatever revision is current.

use std::path::Path;
use std::sync::{mpsc, Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

use super::args::{
    assert_size, expect_revision, fail, now, parse_object, require_key, valid_key, Args,
};
use crate::room::types::Reference;

use super::links::assert_claim_links;
use super::record::{build_record, typed_prefix};
use super::reporter;
use super::trajectory;
use super::ShelfError;

/// Prefixes the free `put` tool may write under; gate records use typed tools.
const FREE_PUT_PREFIXES: &[&str] = &["results/", "skills/", "notes/"];
const LIST_PAGE: usize = 100;

pub(super) struct Row {
    pub content: String,
    pub revision: i64,
    pub updated: i64,
}

/// What a write returned.
pub(super) struct Written {
    pub key: String,
    pub revision: i64,
    pub updated: i64,
}

impl Written {
    fn into_map(self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("key".into(), self.key.into());
        map.insert("revision".into(), self.revision.into());
        map.insert("updated".into(), self.updated.into());
        map
    }
}

pub(super) fn get_row(conn: &Connection, key: &str) -> Result<Option<Row>, ShelfError> {
    Ok(conn
        .query_row(
            "SELECT content, revision, updated FROM shelf_items WHERE key = ?1",
            params![key],
            |r| Ok(Row { content: r.get(0)?, revision: r.get(1)?, updated: r.get(2)? }),
        )
        .optional()?)
}

pub(super) fn write_item(
    conn: &Connection,
    key: &str,
    content: &str,
    expected: i64,
) -> Result<Written, ShelfError> {
    assert_size(content)?;
    let revision = get_row(conn, key)?.map_or(0, |r| r.revision);
    if revision != expected {
        return fail("Revision conflict; get the current item before updating");
    }
    let updated = now();
    conn.execute(
        "INSERT INTO shelf_items (key, content, revision, updated) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET content = excluded.content,
             revision = excluded.revision, updated = excluded.updated",
        params![key, content, revision + 1, updated],
    )?;
    Ok(Written { key: key.to_string(), revision: revision + 1, updated })
}

type MergeFn = fn(Option<Map<String, Value>>, Map<String, Value>) -> Result<Map<String, Value>, ShelfError>;

/// Merge `incoming` into the current record and bump the revision. No
/// `expected_revision`: parallel writers add to each other instead of colliding.
fn merge_write(
    conn: &Connection,
    key: &str,
    incoming: Map<String, Value>,
    merge: MergeFn,
) -> Result<Written, ShelfError> {
    let existing = get_row(conn, key)?;
    let prev = existing.as_ref().and_then(|r| parse_object(&r.content));
    let content = serde_json::to_string(&Value::Object(merge(prev, incoming)?))?;
    assert_size(&content)?;
    let revision = existing.map_or(0, |r| r.revision) + 1;
    let updated = now();
    conn.execute(
        "INSERT INTO shelf_items (key, content, revision, updated) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET content = excluded.content,
             revision = excluded.revision, updated = excluded.updated",
        params![key, content, revision, updated],
    )?;
    Ok(Written { key: key.to_string(), revision, updated })
}

/// After an evidence write, make the parent claim list the evidence key.
/// Returns the claim's new write, if it changed.
fn link_evidence_onto_claim(
    conn: &Connection,
    evidence_key: &str,
    claim_key: &str,
) -> Result<Option<Written>, ShelfError> {
    let Some(claim) = get_row(conn, claim_key)? else {
        return Ok(None);
    };
    let Some(mut body) = parse_object(&claim.content) else {
        return Ok(None);
    };
    if body.get("kind").and_then(Value::as_str) != Some("claim") {
        return Ok(None);
    }
    let mut keys: Vec<Value> = body.get("evidence_keys").and_then(Value::as_array).cloned().unwrap_or_default();
    if keys.iter().any(|k| k.as_str() == Some(evidence_key)) {
        return Ok(None);
    }
    keys.push(evidence_key.into());
    body.insert("evidence_keys".into(), Value::Array(keys));
    body.insert("updated".into(), now().into());
    let written = write_item(conn, claim_key, &serde_json::to_string(&Value::Object(body))?, claim.revision)?;
    Ok(Some(written))
}

/// One row of a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfEntry {
    pub key: String,
    pub revision: i64,
    pub updated: i64,
    /// Present only when content was requested and fits under the size limit.
    pub content: Option<String>,
}

/// A key that was written, announced after its transaction committed.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfEvent {
    pub key: String,
    pub revision: i64,
}

impl From<&Written> for ShelfEvent {
    fn from(w: &Written) -> Self {
        Self { key: w.key.clone(), revision: w.revision }
    }
}

pub struct Shelf {
    conn: Mutex<Connection>,
    subscribers: Mutex<Vec<mpsc::Sender<ShelfEvent>>>,
}

impl Shelf {
    pub fn open(path: &Path) -> Result<Self, ShelfError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, ShelfError> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Open an existing shelf for reading only. No tables are created and no
    /// data, schema or journal mode changes; writes fail. SQLite may still
    /// create its `-wal` and `-shm` index files beside a WAL database, which it
    /// needs to read one. For viewers such as `koma room trajectory`, which must
    /// be safe against a live session.
    pub fn open_read_only(path: &Path) -> Result<Self, ShelfError> {
        use rusqlite::OpenFlags;
        if !path.is_file() {
            return fail(format!("no room database at {}", path.display()));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
        let has_shelf: Option<i64> = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'shelf_items'", [], |r| r.get(0))
            .optional()?;
        if has_shelf.is_none() {
            return fail(format!("{} has no shelf (not a room database)", path.display()));
        }
        Ok(Self { conn: Mutex::new(conn), subscribers: Mutex::new(Vec::new()) })
    }

    fn init(conn: Connection) -> Result<Self, ShelfError> {
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS shelf_items (
                 key      TEXT PRIMARY KEY,
                 content  TEXT NOT NULL,
                 revision INTEGER NOT NULL,
                 updated  INTEGER NOT NULL
             );",
        )?;
        Ok(Self { conn: Mutex::new(conn), subscribers: Mutex::new(Vec::new()) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Be told about every committed write (the trajectory view and the clerk
    /// react to these). Dropping the receiver unsubscribes.
    pub fn subscribe(&self) -> mpsc::Receiver<ShelfEvent> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
        rx
    }

    fn announce(&self, events: Vec<ShelfEvent>) {
        if events.is_empty() {
            return;
        }
        let mut subscribers = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        subscribers.retain(|tx| events.iter().all(|e| tx.send(e.clone()).is_ok()));
    }

    /// Check that every cited `{key, revision}` still exists on the shelf.
    /// Assets and trajectories are living records, so a newer revision of those
    /// is accepted; everything else must match exactly.
    pub fn verify_references(&self, refs: &[Reference]) -> Result<(), ShelfError> {
        let conn = self.lock();
        for r in refs {
            let row = get_row(&conn, &r.key)?;
            let living = r.key.starts_with("assets/") || r.key.starts_with("asset/") || r.key.starts_with("trajectory/");
            let ok = row.is_some_and(|row| {
                let have = row.revision.max(0) as u64;
                have == r.revision || (living && have >= r.revision)
            });
            if !ok {
                return fail(format!(
                    "Shared item changed or missing: {}; publish and verify the current revision",
                    r.key
                ));
            }
        }
        Ok(())
    }

    /// Run a shelf tool for a role. `allow` is the role's `shelf_allow` list.
    pub fn call_as(&self, allow: &[String], name: &str, args: &Value) -> Result<Value, ShelfError> {
        if !allow.iter().any(|a| a == name) {
            return fail(format!("{name} is not available to this role"));
        }
        // Evidence is listed on its claim, and that rewrites the claim. A role that cannot publish claims
        // (the reviewer) must not rewrite one: it would move the revision the review is judging, supersede
        // the review, and start it again. Its evidence is cited from its verdict instead.
        self.call_linking(name, args, allow.iter().any(|a| a == "put_claim"))
    }

    /// Run a shelf tool without a role check (daemon-internal callers).
    pub fn call(&self, name: &str, args: &Value) -> Result<Value, ShelfError> {
        self.call_linking(name, args, true)
    }

    fn call_linking(&self, name: &str, args: &Value, link: bool) -> Result<Value, ShelfError> {
        let Value::Object(args) = args else {
            return fail("Arguments must be an object");
        };
        let (value, events) = {
            let mut conn = self.lock();
            match name {
                "list" => (list_tool(&conn, args)?, Vec::new()),
                "get" => (get_tool(&conn, args)?, Vec::new()),
                "put" => put_free(&mut conn, args)?,
                _ => match name.strip_prefix("put_") {
                    Some(kind) => put_typed(&mut conn, kind, args, link)?,
                    None => return fail("Unknown tool"),
                },
            }
        };
        self.announce(events);
        Ok(value)
    }

    /// Entries ordered by key, filtered by key prefixes (any of), for UIs.
    pub fn list_items(
        &self,
        prefixes: &[&str],
        include_content: bool,
        take: usize,
    ) -> Result<Vec<ShelfEntry>, ShelfError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT key, revision, updated, content FROM shelf_items ORDER BY key")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
        })?;
        let take = take.clamp(1, 400);
        let mut out = Vec::new();
        for row in rows {
            let (key, revision, updated, content) = row?;
            if !prefixes.is_empty() && !prefixes.iter().any(|p| key.starts_with(p)) {
                continue;
            }
            let content = (include_content && content.len() <= super::args::MAX_CONTENT).then_some(content);
            out.push(ShelfEntry { key, revision, updated, content });
            if out.len() >= take {
                break;
            }
        }
        Ok(out)
    }

    /// Every entry under `prefix` with its content, ordered by key, without the
    /// listing cap. For internal scans such as the review queue and trajectory goals.
    pub fn scan(&self, prefix: &str) -> Result<Vec<ShelfEntry>, ShelfError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT key, revision, updated, content FROM shelf_items
             WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key",
        )?;
        let rows = stmt.query_map(params![prefix], |r| {
            Ok(ShelfEntry {
                key: r.get(0)?,
                revision: r.get(1)?,
                updated: r.get(2)?,
                content: Some(r.get(3)?),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// A short digest of every key and revision on the shelf. It changes
    /// whenever anything is written, so a watcher can tell "nothing new" from
    /// "something changed" without reading any content.
    pub fn fingerprint(&self) -> Result<String, ShelfError> {
        use sha2::{Digest, Sha256};
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT key, revision FROM shelf_items ORDER BY key")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut hasher = Sha256::new();
        for row in rows {
            let (key, revision) = row?;
            hasher.update(key.as_bytes());
            hasher.update(b"\0");
            hasher.update(revision.to_le_bytes());
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// One item if it exists. Unlike `read_item`, a missing key is `None`.
    pub fn peek(&self, key: &str) -> Result<Option<ShelfEntry>, ShelfError> {
        let conn = self.lock();
        Ok(get_row(&conn, key)?.map(|row| ShelfEntry {
            key: key.to_string(),
            revision: row.revision,
            updated: row.updated,
            content: Some(row.content),
        }))
    }

    /// One item's content and revision.
    pub fn read_item(&self, key: &str) -> Result<ShelfEntry, ShelfError> {
        if !valid_key(key) {
            return fail("Invalid shelf key");
        }
        let conn = self.lock();
        let Some(row) = get_row(&conn, key)? else {
            return fail("Item not found");
        };
        if row.content.len() > super::args::MAX_CONTENT {
            return fail("Shelf item exceeds 128 KiB");
        }
        Ok(ShelfEntry { key: key.to_string(), revision: row.revision, updated: row.updated, content: Some(row.content) })
    }

    pub fn wipe(&self) -> Result<(), ShelfError> {
        self.lock().execute("DELETE FROM shelf_items", [])?;
        Ok(())
    }

    /// Fails when findings exist but the technical-per-finding plus manager
    /// report pack is incomplete.
    pub fn assert_reporter_pack_complete(&self) -> Result<(), ShelfError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT key, content FROM shelf_items ORDER BY key")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut relevant = Vec::new();
        for row in rows {
            let (key, content) = row?;
            if reporter::is_relevant(&key) {
                relevant.push((key, content));
            }
        }
        if !relevant.iter().any(|(k, _)| k.starts_with("findings/") || k.starts_with("finding/")) {
            return Ok(());
        }
        match reporter::format_gaps(&reporter::pack_gaps(&relevant)) {
            Some(message) => fail(message),
            None => Ok(()),
        }
    }
}

fn list_tool(conn: &Connection, args: &Args) -> Result<Value, ShelfError> {
    let prefix = match args.get("prefix") {
        None | Some(Value::Null) => "",
        Some(Value::String(p)) if p.chars().count() <= 128 => p.as_str(),
        _ => return fail("Invalid prefix"),
    };
    let after = match args.get("after") {
        None | Some(Value::Null) => "",
        Some(Value::String(a)) if a.chars().count() <= 128 => a.as_str(),
        _ => return fail("after must be a key up to 128 chars"),
    };
    let mut stmt = conn.prepare(
        "SELECT key, revision, updated FROM shelf_items
         WHERE substr(key, 1, length(?1)) = ?1 AND key > ?2
         ORDER BY key LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![prefix, after, LIST_PAGE as i64], |r| {
        Ok(json!({ "key": r.get::<_, String>(0)?, "revision": r.get::<_, i64>(1)?, "updated": r.get::<_, i64>(2)? }))
    })?;
    let items = rows.collect::<Result<Vec<Value>, _>>()?;
    let truncated = items.len() == LIST_PAGE;
    let next_after = if truncated { items.last().and_then(|i| i.get("key")).cloned() } else { None };
    Ok(json!({ "items": items, "truncated": truncated, "next_after": next_after }))
}

fn get_tool(conn: &Connection, args: &Args) -> Result<Value, ShelfError> {
    let key = require_key(args.get("key"))?;
    let Some(row) = get_row(conn, &key)? else {
        return fail("Item not found");
    };
    Ok(json!({ "key": key, "content": row.content, "revision": row.revision, "updated": row.updated }))
}

fn put_free(conn: &mut Connection, args: &Args) -> Result<(Value, Vec<ShelfEvent>), ShelfError> {
    let key = require_key(args.get("key"))?;
    if !FREE_PUT_PREFIXES.iter().any(|p| key.starts_with(p)) {
        return fail(
            "Free put only allows results/, skills/, notes/. Use put_claim, put_evidence, put_verdict, put_finding, put_report, put_asset, put_scope, put_trajectory, or put_result for gate records.",
        );
    }
    let Some(Value::String(content)) = args.get("content") else {
        return fail("Content must be text");
    };
    let expected = expect_revision(args.get("expected_revision"))?;
    let tx = conn.transaction()?;
    let written = write_item(&tx, &key, content, expected)?;
    tx.commit()?;
    let events = vec![ShelfEvent::from(&written)];
    Ok((Value::Object(written.into_map()), events))
}

fn put_typed(conn: &mut Connection, kind: &str, args: &Args, link: bool) -> Result<(Value, Vec<ShelfEvent>), ShelfError> {
    let Some(prefix) = typed_prefix(kind) else {
        return fail("Unknown tool");
    };
    let key = require_key(args.get("key"))?;
    if !key.starts_with(prefix) {
        return fail(format!("{kind} keys must start with {prefix}"));
    }
    let record = build_record(kind, args)?;
    let tx = conn.transaction()?;
    let mut events = Vec::new();
    let out = match kind {
        "trajectory" => {
            let (nodes, edges) = trajectory::lists(args)?;
            let drops = trajectory::summarize_drops(&nodes, &edges);
            let written = merge_write(&tx, &key, record, trajectory::merge_trajectory)?;
            let dangling: Vec<String> = get_row(&tx, &key)?
                .and_then(|r| parse_object(&r.content))
                .map(|graph| trajectory::dangling_edges(&graph).into_iter().take(16).collect())
                .unwrap_or_default();
            events.push(ShelfEvent::from(&written));
            let mut out = written.into_map();
            if !drops.is_empty() {
                out.insert("dropped".into(), json!(drops));
            }
            if !dangling.is_empty() {
                out.insert("dangling_edges".into(), json!(dangling));
            }
            out
        }
        "asset" => {
            let written = merge_write(&tx, &key, record, trajectory::merge_asset)?;
            events.push(ShelfEvent::from(&written));
            written.into_map()
        }
        _ => {
            assert_claim_links(&tx, kind, &key, &record)?;
            let expected = expect_revision(args.get("expected_revision"))?;
            let content = serde_json::to_string(&Value::Object(record.clone()))?;
            let written = write_item(&tx, &key, &content, expected)?;
            events.push(ShelfEvent::from(&written));
            let mut similar: Vec<String> = Vec::new();
            if kind == "claim" {
                let mine = Value::Object(record.clone());
                let mut stmt = tx.prepare("SELECT key, content FROM shelf_items WHERE substr(key, 1, 7) = 'claims/' AND key != ?1")?;
                let rows = stmt.query_map(params![key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                for row in rows {
                    let (other, content) = row?;
                    if serde_json::from_str::<Value>(&content).is_ok_and(|c| super::similar::near(&mine, &c)) {
                        similar.push(other);
                    }
                }
            }
            if kind == "evidence" && link {
                if let Some(claim_key) = record.get("claim_key").and_then(Value::as_str) {
                    if let Some(linked) = link_evidence_onto_claim(&tx, &key, claim_key)? {
                        events.push(ShelfEvent::from(&linked));
                    }
                }
            }
            let mut out = written.into_map();
            if !similar.is_empty() {
                out.insert("similar_to".into(), json!(similar));
                out.insert("note".into(), json!("This looks like a claim already on the shelf. Add what is new to that claim instead of publishing a second one, unless it really is a different problem."));
            }
            out
        }
    };
    tx.commit()?;
    Ok((Value::Object(out), events))
}
