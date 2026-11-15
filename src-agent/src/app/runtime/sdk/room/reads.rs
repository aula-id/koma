//! Room reads served from disk: `room.shelf.list`, `room.item`,
//! `room.trajectory` and `room.reports`.
//!
//! Every read goes through the binding's read-only shelf handle on the
//! blocking pool, never over the room socket, so the reads work the same in
//! ROOM(live) and ROOM(closed). Every string of a result is scrubbed before it
//! leaves the adapter.
use super::ring::PAGE_BYTES;
use super::{bad_cursor, bound_room, decode_cursor, encode_cursor, object, out_of_range};
use crate::app::runtime::sdk::methods::HandlerFuture;
use crate::app::runtime::sdk::protocol::{Reason, Request, RpcError};
use crate::app::runtime::sdk::Adapter;
use crate::room::browse;
use crate::room::shelf::{Shelf, ShelfEntry, ShelfError};
use crate::room::trajectory::{self, TrajectoryView};
use base64::Engine as _;
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::Arc;

/// `room.shelf.list` pages hold at most this many rows.
pub const SHELF_ROWS: usize = 500;
const SHELF_DEFAULT: usize = 200;
const PREFIX_CHARS: usize = 200;
/// `room.reports` pages hold at most this many reports.
pub const REPORTS: usize = 100;
const REPORTS_DEFAULT: usize = 50;
const KEY_CHARS: usize = 512;
/// `room.item` content is cut at this many bytes.
pub const ITEM_BYTES: usize = 16 * 1024 * 1024;
/// `room.trajectory` caps.
pub const GOALS: usize = 200;
pub const GOAL_NODES: usize = 2000;
pub const GOAL_EDGES: usize = 8000;
const FINDING_KEYS: usize = 1000;
/// A row title is at most this many characters: 100 plus the ellipsis.
const TITLE_CHARS: usize = 101;

// ── DTOs ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShelfRow {
    pub key: String,
    pub revision: i64,
    pub updated_at: i64,
    pub kind: String,
    pub title: String,
    pub status: String,
}

impl ShelfRow {
    fn of(entry: &ShelfEntry) -> Self {
        let row = browse::row_of(entry);
        Self {
            key: row.key,
            revision: row.revision,
            updated_at: row.updated,
            kind: row.kind,
            title: row.title,
            status: row.status,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ShelfPage {
    rows: Vec<ShelfRow>,
    total: usize,
    next_cursor: Option<String>,
    fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ShelfItem {
    #[serde(flatten)]
    row: ShelfRow,
    content: String,
    json: Option<Value>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Counts {
    pub open: usize,
    pub verified: usize,
    pub returned: usize,
    pub reports: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct GoalNode {
    id: String,
    label: String,
    kind: String,
    status: Option<String>,
    severity: Option<String>,
    note: Option<String>,
    resource_key: Option<String>,
    revision: Option<i64>,
    rank: usize,
    row: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct GoalEdge {
    from: String,
    to: String,
    rel: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Goal {
    goal_key: String,
    counts: Counts,
    columns: usize,
    rows: usize,
    nodes: Vec<GoalNode>,
    nodes_truncated: bool,
    edges: Vec<GoalEdge>,
    edges_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct TrajectoryResult {
    goals: Vec<Goal>,
    goals_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Report {
    key: String,
    revision: i64,
    updated_at: i64,
    title: String,
    audience: String,
    severity_summary: String,
    go_no_go: String,
    finding_keys: Vec<String>,
    finding_keys_truncated: bool,
    markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ReportPage {
    reports: Vec<Report>,
    next_cursor: Option<String>,
}

// ── helpers ─────────────────────────────────────────────────────────────

pub(super) fn unreadable(e: impl std::fmt::Display) -> RpcError {
    RpcError::reason(Reason::RoomUnreadable, format!("the room's shelf cannot be read: {e}"))
}

fn invalid_name(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::InvalidName, message)
}

fn to_value(value: &impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new("internal", e.to_string()))
}

fn limit_param(map: &Map<String, Value>, max: usize, default: usize) -> Result<usize, RpcError> {
    match map.get("limit") {
        None => Ok(default),
        Some(value) => match value.as_u64() {
            Some(n) if (1..=max as u64).contains(&n) => Ok(n as usize),
            _ => Err(out_of_range(format!("limit must be 1 to {max}"))),
        },
    }
}

fn cursor_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    match map.get("cursor") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) => Ok(Some(cursor.clone())),
        Some(_) => Err(bad_cursor()),
    }
}

/// `goal` for `room.trajectory` and `room.spend`, without its `trajectory/`
/// prefix.
pub(super) fn goal_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    match map.get("goal") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(goal)) if !goal.is_empty() => Ok(Some(
            goal.strip_prefix("trajectory/").unwrap_or(goal).to_string(),
        )),
        Some(_) => Err(out_of_range("goal must be a nonempty string")),
    }
}

/// The shelf's key grammar: 1..128 letters, digits and `. _ : / -`, starting
/// with a letter or digit.
fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

/// Cut `text` at `max` bytes on a char boundary.
fn cut(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

/// Clip a title that scrubbing made longer back to [`TITLE_CHARS`].
fn clip_title(value: &mut Value) {
    if let Some(Value::String(title)) = value.get_mut("title") {
        if title.chars().count() > TITLE_CHARS {
            let mut clipped: String = title.chars().take(TITLE_CHARS - 1).collect();
            clipped.push('…');
            *title = clipped;
        }
    }
}

/// Scrub a read result with the bound room's scrubber.
async fn scrubbed(adapter: &Adapter, value: Value) -> Result<Value, RpcError> {
    Ok(bound_room(adapter)?.scrub(value).await)
}

fn shelf_of(adapter: &Adapter) -> Result<Arc<Shelf>, RpcError> {
    bound_room(adapter)?.shelf()
}

// ── room.shelf.list ─────────────────────────────────────────────────────

pub fn handle_shelf_list(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(shelf_list(r.params, adapter))
}

fn encode_key(key: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key)
}

fn decode_key(cursor: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    String::from_utf8(bytes).ok()
}

fn shelf_page(
    shelf: &Shelf,
    prefix: &str,
    after: Option<&str>,
    limit: usize,
) -> Result<ShelfPage, ShelfError> {
    let entries = shelf.scan(prefix)?;
    let total = entries.len();
    let mut rows: Vec<ShelfRow> = entries
        .iter()
        .filter(|entry| after.is_none_or(|key| entry.key.as_str() > key))
        .take(limit + 1)
        .map(ShelfRow::of)
        .collect();
    let next_cursor = (rows.len() > limit).then(|| encode_key(&rows[limit - 1].key));
    rows.truncate(limit);
    Ok(ShelfPage {
        rows,
        total,
        next_cursor,
        fingerprint: shelf.fingerprint()?,
    })
}

async fn shelf_list(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let prefix = match map.get("prefix") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(prefix)) if prefix.chars().count() <= PREFIX_CHARS => prefix.clone(),
        Some(_) => {
            return Err(out_of_range(format!(
                "prefix must be a string of at most {PREFIX_CHARS} characters"
            )))
        }
    };
    let limit = limit_param(&map, SHELF_ROWS, SHELF_DEFAULT)?;
    let after = match cursor_param(&map)? {
        Some(cursor) => Some(decode_key(&cursor).ok_or_else(bad_cursor)?),
        None => None,
    };
    let shelf = shelf_of(adapter)?;
    let page = adapter
        .blocking(move || {
            shelf_page(&shelf, &prefix, after.as_deref(), limit).map_err(unreadable)
        })
        .await?;
    let mut value = scrubbed(adapter, to_value(&page)?).await?;
    if let Some(Value::Array(rows)) = value.get_mut("rows") {
        rows.iter_mut().for_each(clip_title);
    }
    Ok(value)
}

// ── room.item ───────────────────────────────────────────────────────────

pub fn handle_item(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(item(r.params, adapter))
}

fn item_of(entry: ShelfEntry) -> ShelfItem {
    let row = ShelfRow::of(&entry);
    let mut content = entry.content.unwrap_or_default();
    let truncated = cut(&mut content, ITEM_BYTES);
    let json = if truncated {
        None
    } else {
        serde_json::from_str::<Value>(&content)
            .ok()
            .filter(Value::is_object)
    };
    ShelfItem {
        row,
        content,
        json,
        truncated,
    }
}

async fn item(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let key = match map.get("key") {
        Some(Value::String(key)) if !key.is_empty() && key.chars().count() <= KEY_CHARS => {
            key.clone()
        }
        _ => {
            return Err(out_of_range(format!(
                "key must be a string of 1 to {KEY_CHARS} characters"
            )))
        }
    };
    if !valid_key(&key) {
        return Err(invalid_name(
            "a shelf key is 1 to 128 letters, digits or . _ : / -, starting with a letter or digit",
        ));
    }
    let shelf = shelf_of(adapter)?;
    let found = adapter
        .blocking(move || shelf.peek(&key).map_err(unreadable))
        .await?;
    let Some(entry) = found else {
        return Err(RpcError::reason(Reason::NotFound, "no such shelf item"));
    };
    let mut value = scrubbed(adapter, to_value(&item_of(entry))?).await?;
    clip_title(&mut value);
    Ok(value)
}

// ── room.trajectory ─────────────────────────────────────────────────────

pub fn handle_trajectory(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(trajectory(r.params, adapter))
}

fn goal_of(view: TrajectoryView) -> Goal {
    let mut nodes: Vec<GoalNode> = view
        .nodes
        .into_iter()
        .map(|n| GoalNode {
            id: n.id,
            label: n.label,
            kind: n.kind,
            status: n.status,
            severity: n.severity,
            note: n.note,
            resource_key: n.resource_key,
            revision: n.revision,
            rank: n.rank,
            row: n.row,
        })
        .collect();
    nodes.sort_by_key(|n| (n.rank, n.row));
    let nodes_truncated = nodes.len() > GOAL_NODES;
    nodes.truncate(GOAL_NODES);
    let mut edges: Vec<GoalEdge> = view
        .edges
        .into_iter()
        .map(|e| GoalEdge {
            from: e.from,
            to: e.to,
            rel: e.rel,
        })
        .collect();
    let edges_truncated = edges.len() > GOAL_EDGES;
    edges.truncate(GOAL_EDGES);
    Goal {
        goal_key: view.goal_key,
        counts: Counts {
            open: view.counts.open,
            verified: view.counts.verified,
            returned: view.counts.returned,
            reports: view.counts.reports,
        },
        columns: view.columns,
        rows: view.rows,
        nodes,
        nodes_truncated,
        edges,
        edges_truncated,
    }
}

async fn trajectory(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let goal = goal_param(&map)?;
    let shelf = shelf_of(adapter)?;
    let result = adapter
        .blocking(move || {
            let views = trajectory::project(&shelf, goal.as_deref()).map_err(unreadable)?;
            let goals_truncated = views.len() > GOALS;
            let goals = views.into_iter().take(GOALS).map(goal_of).collect();
            Ok(TrajectoryResult {
                goals,
                goals_truncated,
            })
        })
        .await?;
    scrubbed(adapter, to_value(&result)?).await
}

// ── room.reports ────────────────────────────────────────────────────────

pub fn handle_reports(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(reports(r.params, adapter))
}

fn report_of(report: browse::Report, include_markdown: bool) -> Report {
    let mut finding_keys = report.finding_keys;
    let finding_keys_truncated = finding_keys.len() > FINDING_KEYS;
    finding_keys.truncate(FINDING_KEYS);
    Report {
        key: report.key,
        revision: report.revision,
        updated_at: report.updated,
        title: report.title,
        audience: report.audience,
        severity_summary: report.severity_summary,
        go_no_go: report.go_no_go,
        finding_keys,
        finding_keys_truncated,
        markdown: include_markdown.then_some(report.markdown),
    }
}

/// Reports newest first (ties by key), after the keyset `after`: at most
/// `limit` of them and [`PAGE_BYTES`] of serialized reports, at least one.
fn report_page(
    shelf: &Shelf,
    include_markdown: bool,
    after: Option<(i64, String)>,
    limit: usize,
) -> Result<ReportPage, RpcError> {
    let mut all: Vec<browse::Report> = shelf
        .scan("reports/")
        .map_err(unreadable)?
        .iter()
        .filter_map(browse::report_of)
        .collect();
    all.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.key.cmp(&b.key)));
    let mut reports = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    for report in all.into_iter().filter(|r| {
        after
            .as_ref()
            .is_none_or(|(at, key)| r.updated < *at || (r.updated == *at && r.key.as_str() > key.as_str()))
    }) {
        let report = report_of(report, include_markdown);
        let size = serde_json::to_vec(&report).map_or(0, |b| b.len());
        if reports.len() >= limit || (!reports.is_empty() && bytes + size > PAGE_BYTES) {
            more = true;
            break;
        }
        bytes += size;
        reports.push(report);
    }
    let next_cursor = more
        .then(|| reports.last().map(|r| encode_cursor(r.updated_at, &r.key)))
        .flatten();
    Ok(ReportPage {
        reports,
        next_cursor,
    })
}

async fn reports(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let include_markdown = match map.get("include_markdown") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(out_of_range("include_markdown must be a boolean")),
    };
    let limit = limit_param(&map, REPORTS, REPORTS_DEFAULT)?;
    let after = match cursor_param(&map)? {
        Some(cursor) => Some(decode_cursor(&cursor).ok_or_else(bad_cursor)?),
        None => None,
    };
    let shelf = shelf_of(adapter)?;
    let page = adapter
        .blocking(move || report_page(&shelf, include_markdown, after, limit))
        .await?;
    scrubbed(adapter, to_value(&page)?).await
}
