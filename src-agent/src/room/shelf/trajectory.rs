//! Trajectory and asset records are built by several lanes at once, so writes
//! are merged into the current revision instead of requiring `expected_revision`.
//!
//! Models send nodes and edges as objects, as JSON strings of objects, or as a
//! JSON string of an array. Entries that cannot be used are dropped and
//! reported back so the next write can fix them.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use super::args::{fail, Args};
use super::ShelfError;

pub const NODE_CAP: usize = 400;
pub const EDGE_CAP: usize = 800;

const FROM_ALIASES: &[&str] = &["from", "source", "source_id", "src", "a"];
const TO_ALIASES: &[&str] = &["to", "target", "target_id", "dst", "b"];
const REL_ALIASES: &[&str] = &["rel", "relation", "type", "label"];

/// A string, or a string-of-a-string, that holds a JSON object or array.
/// Anything else (including invalid JSON) is `None`.
fn parse_json_text(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if !matches!(trimmed.chars().next(), Some('{' | '[' | '"')) {
        return None;
    }
    let mut value: Value = serde_json::from_str(trimmed).ok()?;
    if let Value::String(inner) = &value {
        let inner = inner.trim();
        if inner.starts_with('{') || inner.starts_with('[') {
            value = serde_json::from_str(inner).ok()?;
        }
    }
    Some(value)
}

fn coerce_item(item: &Value) -> Vec<Value> {
    let Value::String(text) = item else {
        return vec![item.clone()];
    };
    match parse_json_text(text) {
        Some(Value::Array(items)) => items.iter().flat_map(coerce_item).collect(),
        Some(obj @ Value::Object(_)) => vec![obj],
        _ => vec![item.clone()],
    }
}

fn coerce_list(value: Option<&Value>) -> Option<Vec<Value>> {
    match value? {
        Value::Array(items) => Some(items.iter().flat_map(coerce_item).collect()),
        s @ Value::String(_) => Some(coerce_item(s)),
        _ => None,
    }
}

/// The `nodes` and `edges` arguments, expanded from any accepted form.
pub fn lists(args: &Args) -> Result<(Vec<Value>, Vec<Value>), ShelfError> {
    let (Some(nodes), Some(edges)) = (coerce_list(args.get("nodes")), coerce_list(args.get("edges")))
    else {
        return fail("nodes and edges must be arrays");
    };
    if nodes.len() > NODE_CAP || edges.len() > EDGE_CAP {
        return fail("trajectory too large");
    }
    Ok((nodes, edges))
}

fn first_text(map: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| match map.get(*name) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    })
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn nonblank(map: &Map<String, Value>, name: &str, max: usize) -> Option<String> {
    match map.get(name) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(clip(s.trim(), max)),
        _ => None,
    }
}

/// A usable node, or `None` when it has no id.
pub fn normalize_node(raw: &Value) -> Option<Value> {
    let node = raw.as_object()?;
    let id = node.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if id.is_empty() || id.chars().count() > 128 {
        return None;
    }
    let label = nonblank(node, "label", 256).unwrap_or_else(|| id.to_string());
    let kind = nonblank(node, "kind", 32).map_or_else(|| "fact".to_string(), |k| k.to_lowercase());
    let mut out = Map::new();
    out.insert("id".into(), id.into());
    out.insert("label".into(), label.into());
    out.insert("kind".into(), kind.into());
    if let Some(status) = nonblank(node, "status", 64) {
        out.insert("status".into(), status.into());
    }
    if let Some(sev) = nonblank(node, "severity", 32) {
        out.insert("severity".into(), sev.into());
    }
    if let Some(key) = first_text(node, &["resource_key", "resourceKey", "shelf_key"]) {
        out.insert("resource_key".into(), clip(&key, 128).into());
    }
    if let Some(note) = nonblank(node, "note", 2000) {
        out.insert("note".into(), note.into());
    }
    Some(Value::Object(out))
}

/// A usable edge, or `None` when an endpoint is missing. Accepts the aliases
/// models tend to use (`source`/`target`, `relation`, ...).
pub fn normalize_edge(raw: &Value) -> Option<Value> {
    let edge = raw.as_object()?;
    let from = first_text(edge, FROM_ALIASES)?;
    let to = first_text(edge, TO_ALIASES)?;
    let mut out = Map::new();
    out.insert("from".into(), clip(&from, 128).into());
    out.insert("to".into(), clip(&to, 128).into());
    if let Some(rel) = first_text(edge, REL_ALIASES) {
        out.insert("rel".into(), clip(&rel.to_lowercase(), 32).into());
    }
    if let Some(id) = nonblank(edge, "id", 128) {
        out.insert("id".into(), id.into());
    }
    Some(Value::Object(out))
}

/// Human-readable reasons entries were dropped, by position.
pub fn summarize_drops(nodes: &[Value], edges: &[Value]) -> Vec<String> {
    let mut drops = Vec::new();
    for (i, raw) in nodes.iter().enumerate() {
        match raw {
            Value::String(_) => drops.push(format!("node[{i}]: JSON string did not parse as an object")),
            Value::Object(map) => {
                let id = map.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
                if id.is_empty() {
                    drops.push(format!("node[{i}]: missing id"));
                } else if id.chars().count() > 128 {
                    drops.push(format!("node[{i}] ({}…): id over 128 chars", clip(id, 32)));
                }
            }
            _ => drops.push(format!("node[{i}]: not an object")),
        }
    }
    for (i, raw) in edges.iter().enumerate() {
        match raw {
            Value::String(_) => drops.push(format!("edge[{i}]: JSON string did not parse as an object")),
            Value::Object(map) => {
                if first_text(map, FROM_ALIASES).is_none() || first_text(map, TO_ALIASES).is_none() {
                    drops.push(format!("edge[{i}]: missing from/to endpoint"));
                }
            }
            _ => drops.push(format!("edge[{i}]: not an object")),
        }
    }
    drops
}

/// Merge two lists by `id`; a later entry replaces an earlier one in place.
/// Entries without an id are kept in order.
fn merge_by_id(prev: Option<&Value>, next: Option<&Value>) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_slot: HashMap<String, Value> = HashMap::new();
    for list in [prev, next] {
        let Some(Value::Array(entries)) = list else {
            continue;
        };
        for entry in entries {
            let slot = entry
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("#{}", order.len()), str::to_string);
            if !by_slot.contains_key(&slot) {
                order.push(slot.clone());
            }
            by_slot.insert(slot, entry.clone());
        }
    }
    order.into_iter().filter_map(|slot| by_slot.remove(&slot)).collect()
}

pub fn merge_trajectory(
    prev: Option<Map<String, Value>>,
    next: Map<String, Value>,
) -> Result<Map<String, Value>, ShelfError> {
    let Some(prev) = prev else {
        return Ok(next);
    };
    let nodes = merge_by_id(prev.get("nodes"), next.get("nodes"));
    let edges = merge_by_id(prev.get("edges"), next.get("edges"));
    if nodes.len() > NODE_CAP || edges.len() > EDGE_CAP {
        return fail("trajectory too large after merge");
    }
    let mut merged = prev;
    merged.extend(next);
    merged.insert("nodes".into(), Value::Array(nodes));
    merged.insert("edges".into(), Value::Array(edges));
    Ok(merged)
}

/// Strings and numbers from both lists, first occurrence wins, no duplicates.
fn union_scalars(prev: Option<&Value>, next: Option<&Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for list in [prev, next] {
        let Some(Value::Array(items)) = list else {
            continue;
        };
        for item in items {
            let key = match item {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            if !key.is_empty() && seen.insert(key) {
                out.push(item.clone());
            }
        }
    }
    out
}

const ASSET_UNION_FIELDS: &[&str] = &[
    "tags",
    "tech",
    "paths",
    "ports",
    "evidence_keys",
    "claim_keys",
    "finding_keys",
];

pub fn merge_asset(
    prev: Option<Map<String, Value>>,
    next: Map<String, Value>,
) -> Result<Map<String, Value>, ShelfError> {
    let Some(prev) = prev else {
        return Ok(next);
    };
    let mut unions = Vec::new();
    for field in ASSET_UNION_FIELDS {
        unions.push((*field, union_scalars(prev.get(*field), next.get(*field))));
    }
    let mut merged = prev;
    merged.extend(next);
    for (field, list) in unions {
        merged.insert(field.into(), Value::Array(list));
    }
    Ok(merged)
}

/// `from→to` for every edge whose endpoint is not a node of the graph.
pub fn dangling_edges(graph: &Map<String, Value>) -> Vec<String> {
    let ids: HashSet<&str> = graph
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().filter_map(|n| n.get("id").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    graph
        .get("edges")
        .and_then(Value::as_array)
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| {
                    let from = e.get("from").and_then(Value::as_str).unwrap_or("");
                    let to = e.get("to").and_then(Value::as_str).unwrap_or("");
                    (!from.is_empty() && !to.is_empty() && (!ids.contains(from) || !ids.contains(to)))
                        .then(|| format!("{from}→{to}"))
                })
                .collect()
        })
        .unwrap_or_default()
}
