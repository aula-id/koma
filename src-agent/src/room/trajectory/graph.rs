//! The trajectory graph: what the lanes published as a map, merged with what
//! the shelf proves.
//!
//! Ported from Orpheus `trajectory-graph.ts`. The shelf owns the gate spine
//! (scope, asset, claim, evidence, verdict, finding); the lanes' own
//! exploration nodes (intent, goal, fact, hint, digest, probe) are kept on top,
//! so a fact is never dropped just because a claim exists.
//!
//! The CWE `speculation/` node kind is not ported; that feature is deferred.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

use super::gate::{self, apply_gate_statuses, display_status_for_verdict};
use crate::room::shelf::ShelfEntry;

pub const EXPLORATION_KINDS: &[&str] = &["start", "intent", "goal", "fact", "hint", "digest", "probe"];
pub const GATE_KINDS: &[&str] = &["scope", "asset", "claim", "evidence", "verdict", "finding"];
const KEPT_EXPLORATION: &[&str] = &["intent", "goal", "fact", "hint", "digest", "probe"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub status: Option<String>,
    pub severity: Option<String>,
    pub resource_key: Option<String>,
    pub note: Option<String>,
}

impl Node {
    pub fn new(id: &str, label: &str, kind: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            status: None,
            severity: None,
            resource_key: None,
            note: None,
        }
    }
    fn with_resource(mut self, key: &str) -> Self {
        self.resource_key = Some(key.to_string());
        self
    }
    fn with_status(mut self, status: Option<String>) -> Self {
        self.status = status;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub rel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Graph {
    pub goal_key: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

fn is_gate(kind: &str) -> bool {
    GATE_KINDS.contains(&kind.to_lowercase().as_str())
}

fn is_exploration(kind: &str) -> bool {
    let k = kind.to_lowercase();
    EXPLORATION_KINDS.contains(&k.as_str()) || k == "node" || (!GATE_KINDS.contains(&k.as_str()) && k != "start")
}

/// The last path segment, lowercased: how bots refer to a record loosely.
pub fn node_slug(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.split('/').rfind(|p| !p.is_empty()).unwrap_or(trimmed).to_lowercase()
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

fn label_slug(node: &Node) -> String {
    let basis = if !node.label.is_empty() {
        node.label.as_str()
    } else {
        nonempty(&node.resource_key).unwrap_or(&node.id)
    };
    node_slug(basis)
}

fn resource_or_id_slug(node: &Node) -> String {
    node_slug(nonempty(&node.resource_key).unwrap_or(&node.id))
}

fn text(map: &Option<serde_json::Map<String, Value>>, name: &str) -> Option<String> {
    map.as_ref()?.get(name)?.as_str().map(str::to_string)
}

fn first_text(map: &serde_json::Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| map.get(*n).and_then(Value::as_str)).map(str::to_string)
}

/// Accept from/to, source/target and the other spellings lanes publish.
pub fn normalize_edge(raw: &Value) -> Option<Edge> {
    let edge = raw.as_object()?;
    let from = first_text(edge, &["from", "source", "source_id", "src", "a"])?;
    let to = first_text(edge, &["to", "target", "target_id", "dst", "b"])?;
    if from.trim().is_empty() || to.trim().is_empty() {
        return None;
    }
    let rel = first_text(edge, &["rel", "relation", "type", "label"]).map(|r| r.to_lowercase());
    Some(Edge { from: from.trim().to_string(), to: to.trim().to_string(), rel })
}

/// Read a stored `trajectory/` record. `None` when it holds no nodes or edges.
pub fn parse_trajectory(content: &str, fallback_key: &str) -> Option<Graph> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    if let Some(inner) = parsed.get("content").and_then(Value::as_str) {
        if inner.trim().starts_with('{') {
            return parse_trajectory(inner, fallback_key);
        }
    }
    let nodes: Vec<Node> = parsed
        .get("nodes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|n| {
                    let map = n.as_object()?;
                    let id = map.get("id")?.as_str()?.trim().to_string();
                    let label = map.get("label").and_then(Value::as_str).map_or_else(|| id.clone(), str::to_string);
                    let kind = map.get("kind").and_then(Value::as_str).map_or_else(|| "fact".to_string(), str::to_lowercase);
                    Some(Node {
                        id,
                        label,
                        kind,
                        status: map.get("status").and_then(Value::as_str).map(str::to_string),
                        severity: map.get("severity").and_then(Value::as_str).map(str::to_string),
                        resource_key: ["resourceKey", "resource_key", "shelf_key"]
                            .iter()
                            .find_map(|k| map.get(*k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
                            .map(str::to_string),
                        note: map.get("note").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let edges: Vec<Edge> = parsed
        .get("edges")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(normalize_edge).collect())
        .unwrap_or_default();
    if nodes.is_empty() && edges.is_empty() {
        return None;
    }
    let goal_key = parsed.get("goal_key").and_then(Value::as_str).map_or_else(|| fallback_key.to_string(), str::to_string);
    Some(Graph { goal_key, nodes, edges })
}

fn last_segment(key: &str) -> String {
    key.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(key).to_string()
}

fn resolve_claim_id(claim_key: Option<&str>, claim_ids: &[String]) -> Option<String> {
    let claim_key = claim_key?;
    if claim_ids.iter().any(|c| c == claim_key) {
        return Some(claim_key.to_string());
    }
    let slug = node_slug(claim_key);
    claim_ids
        .iter()
        .find(|id| node_slug(id) == slug || id.ends_with(&format!("/{slug}")))
        .cloned()
}

fn edge(from: &str, to: &str, rel: &str) -> Edge {
    Edge { from: from.to_string(), to: to.to_string(), rel: Some(rel.to_string()) }
}

/// Build a usable path graph from the typed shelf rows alone.
pub fn synthesize(items: &[ShelfEntry], preferred_goal: Option<&str>) -> Graph {
    let idx = gate::index(items);
    let goal_key = preferred_goal
        .map(|g| g.strip_prefix("trajectory/").unwrap_or(g))
        .filter(|g| !g.is_empty())
        .map(str::to_string)
        .or_else(|| items.iter().find(|i| i.key.starts_with("scope/")).map(|i| i.key.replacen("scope/", "", 1)))
        .unwrap_or_else(|| "gate".to_string());

    let start = "start";
    let mut nodes = vec![Node::new(start, &goal_key, "start").with_status(Some("in-scope".into()))];
    let mut edges = Vec::new();
    let mut claim_ids: Vec<String> = Vec::new();
    let rec = |i: &ShelfEntry| gate::parse_object(i.content.as_deref());

    for item in items.iter().filter(|i| i.key.starts_with("scope/")) {
        nodes.push(Node::new(&item.key, &last_segment(&item.key), "scope").with_resource(&item.key));
        edges.push(edge(start, &item.key, "scope"));
    }
    for item in items.iter().filter(|i| gate::is_claim_key(&i.key)) {
        let record = rec(item);
        let label = text(&record, "name").filter(|s| !s.is_empty()).unwrap_or_else(|| last_segment(&item.key));
        let status = gate::gate_status_for_claim(&item.key, &idx, text(&record, "status").as_deref());
        let mut node = Node::new(&item.key, &label, "claim").with_resource(&item.key).with_status(Some(status.as_str().into()));
        node.severity = text(&record, "severity");
        nodes.push(node);
        claim_ids.push(item.key.clone());
        edges.push(edge(start, &item.key, "derives"));
        let assets = record
            .as_ref()
            .and_then(|r| r.get("asset_keys"))
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        for asset in assets {
            if !nodes.iter().any(|n| n.id == asset) {
                nodes.push(Node::new(&asset, &last_segment(&asset), "asset").with_resource(&asset));
            }
            edges.push(edge(&asset, &item.key, "hosts"));
        }
    }
    for item in items.iter().filter(|i| i.key.starts_with("evidence/")) {
        let record = rec(item);
        let label = text(&record, "evidence_kind").filter(|s| !s.is_empty()).unwrap_or_else(|| last_segment(&item.key));
        nodes.push(Node::new(&item.key, &label, "evidence").with_resource(&item.key));
        let claim = text(&record, "claim_key").map(|s| s.trim().to_string());
        let from = resolve_claim_id(claim.as_deref(), &claim_ids).unwrap_or_else(|| start.to_string());
        edges.push(edge(&from, &item.key, "supports"));
    }
    for item in items.iter().filter(|i| i.key.starts_with("assets/") || i.key.starts_with("asset/")) {
        if nodes.iter().any(|n| n.id == item.key) {
            continue;
        }
        let record = rec(item);
        let label = text(&record, "title").filter(|s| !s.is_empty()).unwrap_or_else(|| last_segment(&item.key));
        nodes.push(Node::new(&item.key, &label, "asset").with_resource(&item.key).with_status(text(&record, "status")));
        edges.push(edge(start, &item.key, "maps"));
    }
    for item in items.iter().filter(|i| gate::is_finding_key(&i.key)) {
        let record = rec(item);
        let label = text(&record, "name").filter(|s| !s.is_empty()).unwrap_or_else(|| last_segment(&item.key));
        nodes.push(Node::new(&item.key, &label, "finding").with_resource(&item.key).with_status(Some("verified".into())));
        let claim = text(&record, "claim_key").map(|s| s.trim().to_string());
        let from = match claim.as_deref() {
            Some(c) if claim_ids.iter().any(|id| id == c) => c.to_string(),
            other => resolve_claim_id(other, &claim_ids).unwrap_or_else(|| start.to_string()),
        };
        edges.push(edge(&from, &item.key, "proves"));
    }
    for item in items.iter().filter(|i| gate::is_verdict_key(&i.key)) {
        let record = rec(item);
        let claim = text(&record, "claim_key").map(|s| s.trim().to_string());
        let verdict = text(&record, "verdict").unwrap_or_else(|| "verdict".to_string());
        let status = display_status_for_verdict(&verdict, claim.as_deref(), &idx);
        nodes.push(Node::new(&item.key, &status, "verdict").with_resource(&item.key).with_status(Some(status.clone())));
        let from = claim.as_deref().and_then(|c| resolve_claim_id(Some(c), &claim_ids)).unwrap_or_else(|| start.to_string());
        edges.push(edge(&from, &item.key, "checks"));
    }
    Graph { goal_key, nodes, edges }
}

// ---- resolving loosely-named endpoints ----

struct NodeIndex {
    ids: Vec<String>,
    by_kind_slug: HashMap<String, String>,
    by_resource: HashMap<String, String>,
}

fn has_gate_prefix(id: &str) -> bool {
    ["claims/", "findings/", "verdicts/", "scope/", "assets/", "evidence/"].iter().any(|p| id.starts_with(p))
}

fn index_nodes(nodes: &[Node]) -> NodeIndex {
    let mut idx = NodeIndex { ids: Vec::new(), by_kind_slug: HashMap::new(), by_resource: HashMap::new() };
    for node in nodes {
        idx.ids.push(node.id.clone());
        let kind = if node.kind.is_empty() { "node".to_string() } else { node.kind.to_lowercase() };
        let mut slugs: Vec<String> = Vec::new();
        for s in [node_slug(nonempty(&node.resource_key).unwrap_or("")), node_slug(&node.id), node_slug(&node.label)] {
            if !s.is_empty() && !slugs.contains(&s) {
                slugs.push(s);
            }
        }
        for slug in slugs {
            let key = format!("{kind}:{slug}");
            if !idx.by_kind_slug.contains_key(&key) || has_gate_prefix(&node.id) {
                idx.by_kind_slug.insert(key, node.id.clone());
            }
        }
        if let Some(rk) = nonempty(&node.resource_key) {
            idx.by_resource.insert(rk.to_string(), node.id.clone());
        }
        if has_gate_prefix(&node.id) {
            idx.by_resource.insert(node.id.clone(), node.id.clone());
        }
    }
    idx
}

const RESOLVE_ORDER: &[&str] = &["claim", "finding", "verdict", "scope", "asset", "evidence", "intent", "goal", "fact", "start", "node"];

fn resolve_endpoint(reference: &str, idx: &NodeIndex, hint_kind: Option<&str>) -> Option<String> {
    if idx.ids.iter().any(|id| id == reference) {
        return Some(reference.to_string());
    }
    if let Some(hit) = idx.by_resource.get(reference) {
        return Some(hit.clone());
    }
    let slug = node_slug(reference);
    if let Some(kind) = hint_kind {
        if let Some(hit) = idx.by_kind_slug.get(&format!("{kind}:{slug}")) {
            return Some(hit.clone());
        }
    }
    for kind in RESOLVE_ORDER {
        if let Some(hit) = idx.by_kind_slug.get(&format!("{kind}:{slug}")) {
            return Some(hit.clone());
        }
    }
    idx.ids.iter().find(|id| node_slug(id) == slug).cloned()
}

fn match_by_bot_ref(reference: &str, bot_nodes: &[Node], idx: &NodeIndex) -> Option<String> {
    let bot = bot_nodes.iter().find(|n| n.id == reference)?;
    let kind = if bot.kind.is_empty() { "node".to_string() } else { bot.kind.to_lowercase() };
    if let Some(hit) = idx.by_kind_slug.get(&format!("{kind}:{}", label_slug(bot))) {
        return Some(hit.clone());
    }
    resolve_endpoint(&label_slug(bot), idx, Some(&kind))
}

/// Edges added once, never between a node and itself, never to an unknown node.
struct EdgeSet {
    edges: Vec<Edge>,
    seen: HashSet<(String, String, String)>,
}

impl EdgeSet {
    fn new(initial: Vec<Edge>) -> Self {
        let seen = initial.iter().map(Self::key).collect();
        Self { edges: initial, seen }
    }
    fn key(e: &Edge) -> (String, String, String) {
        (e.from.clone(), e.to.clone(), e.rel.clone().unwrap_or_default())
    }
    fn push(&mut self, e: Edge, known: Option<&HashSet<String>>) {
        if e.from == e.to || known.is_some_and(|ids| !ids.contains(&e.from) || !ids.contains(&e.to)) {
            return;
        }
        if self.seen.insert(Self::key(&e)) {
            self.edges.push(e);
        }
    }
}

fn orphan_rel(kind: &str) -> &'static str {
    match kind {
        "scope" => "scope",
        "claim" => "derives",
        "finding" => "proves",
        "verdict" => "checks",
        "intent" | "goal" => "spawns",
        "fact" => "yields",
        _ => "relates",
    }
}

fn fold_exploration_onto_shelf(shelf: Graph, loaded: &Graph, items: &[ShelfEntry]) -> Graph {
    let mut nodes: Vec<Node> = shelf
        .nodes
        .iter()
        .map(|node| {
            let kind = node.kind.to_lowercase();
            let slug = label_slug(node);
            let bot = loaded.nodes.iter().find(|c| {
                c.kind.to_lowercase() == kind
                    && (resource_or_id_slug(c) == resource_or_id_slug(node)
                        || label_slug(c) == slug
                        || label_slug(c) == node_slug(&node.id))
            });
            match bot {
                None => node.clone(),
                Some(bot) => {
                    let mut merged = node.clone();
                    if bot.label.chars().count() > node.label.chars().count() {
                        merged.label = bot.label.clone();
                    }
                    merged.severity = node.severity.clone().filter(|s| !s.is_empty()).or_else(|| bot.severity.clone());
                    merged.note = node.note.clone().filter(|s| !s.is_empty()).or_else(|| bot.note.clone());
                    merged
                }
            }
        })
        .collect();

    let mut shelf_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let start = nodes.iter().find(|n| n.kind == "start").map_or("start", |n| n.id.as_str()).to_string();
    let mut set = EdgeSet::new(shelf.edges.clone());

    for node in &loaded.nodes {
        let kind = if node.kind.is_empty() { "fact".to_string() } else { node.kind.to_lowercase() };
        if kind == "start" {
            continue;
        }
        // Gate kinds belong to the shelf; a lane's short-id stand-in is dropped.
        if is_gate(&kind) && (!node.id.contains('/') || shelf_ids.contains(&node.id)) {
            continue;
        }
        if is_gate(&kind)
            && nodes.iter().any(|e| is_gate(&e.kind) && (label_slug(e) == label_slug(node) || resource_or_id_slug(e) == resource_or_id_slug(node)))
        {
            continue;
        }
        if shelf_ids.contains(&node.id) {
            continue;
        }
        if is_exploration(&kind)
            && !KEPT_EXPLORATION.contains(&kind.as_str())
            && nodes.iter().any(|e| label_slug(e) == label_slug(node) && ["claim", "finding", "verdict"].contains(&e.kind.to_lowercase().as_str()))
        {
            continue;
        }
        if KEPT_EXPLORATION.contains(&kind.as_str()) || is_exploration(&kind) {
            nodes.push(Node { kind, ..node.clone() });
            shelf_ids.insert(node.id.clone());
        }
    }

    let idx = index_nodes(&nodes);
    for raw in &loaded.edges {
        let from = resolve_endpoint(&raw.from, &idx, None).or_else(|| match_by_bot_ref(&raw.from, &loaded.nodes, &idx));
        let to = resolve_endpoint(&raw.to, &idx, None).or_else(|| match_by_bot_ref(&raw.to, &loaded.nodes, &idx));
        if let (Some(from), Some(to)) = (from, to) {
            set.push(Edge { from, to, rel: raw.rel.clone() }, Some(&shelf_ids));
        }
    }

    // Hang nodes with no incoming edge under the start node.
    let mut incoming: HashSet<String> = set.edges.iter().map(|e| e.to.clone()).collect();
    for node in &nodes {
        if node.id == start || incoming.contains(&node.id) {
            continue;
        }
        let kind = node.kind.to_lowercase();
        if KEPT_EXPLORATION.contains(&kind.as_str()) || !is_gate(&kind) {
            let rel = if KEPT_EXPLORATION.contains(&kind.as_str()) { orphan_rel(&kind) } else { "relates" };
            set.push(edge(&start, &node.id, rel), Some(&shelf_ids));
            incoming.insert(node.id.clone());
        }
    }

    Graph {
        goal_key: if loaded.goal_key.is_empty() { shelf.goal_key } else { loaded.goal_key.clone() },
        nodes: apply_gate_statuses(nodes, items),
        edges: set.edges,
    }
}

fn repair_loaded_graph(loaded: &Graph, items: &[ShelfEntry]) -> Graph {
    let mut nodes: Vec<Node> = loaded.nodes.iter().map(|n| Node { kind: n.kind.to_lowercase(), ..n.clone() }).collect();
    if !nodes.iter().any(|n| n.kind == "start" || n.id == "start") {
        let label = if loaded.goal_key.is_empty() { "goal" } else { loaded.goal_key.as_str() };
        nodes.insert(0, Node::new("start", label, "start").with_status(Some("in-scope".into())));
    }
    let idx = index_nodes(&nodes);
    let mut set = EdgeSet::new(Vec::new());
    for e in &loaded.edges {
        if let (Some(from), Some(to)) = (resolve_endpoint(&e.from, &idx, None), resolve_endpoint(&e.to, &idx, None)) {
            set.push(Edge { from, to, rel: e.rel.clone() }, None);
        }
    }
    let start = nodes.iter().find(|n| n.kind == "start").map_or("start", |n| n.id.as_str()).to_string();
    let mut incoming: HashSet<String> = set.edges.iter().map(|e| e.to.clone()).collect();
    for node in &nodes {
        if node.id == start || incoming.contains(&node.id) {
            continue;
        }
        set.push(edge(&start, &node.id, orphan_rel(&node.kind.to_lowercase())), None);
        incoming.insert(node.id.clone());
    }
    Graph { goal_key: loaded.goal_key.clone(), nodes: apply_gate_statuses(nodes, items), edges: set.edges }
}

/// Merge a lane-published trajectory with the shelf. With no published graph
/// the shelf alone is drawn; with gate rows on the shelf the shelf's spine wins.
pub fn merge_with_shelf(loaded: Option<&Graph>, items: &[ShelfEntry], preferred_goal: Option<&str>) -> Graph {
    let shelf = synthesize(items, preferred_goal.or(loaded.map(|l| l.goal_key.as_str())));
    let shelf_has_gate = items.iter().any(|i| {
        gate::is_claim_key(&i.key)
            || gate::is_finding_key(&i.key)
            || gate::is_verdict_key(&i.key)
            || i.key.starts_with("scope/")
            || i.key.starts_with("assets/")
            || i.key.starts_with("evidence/")
    });
    match loaded {
        None => Graph { nodes: apply_gate_statuses(shelf.nodes.clone(), items), ..shelf },
        Some(l) if l.nodes.is_empty() => Graph { nodes: apply_gate_statuses(shelf.nodes.clone(), items), ..shelf },
        Some(l) if shelf_has_gate => fold_exploration_onto_shelf(shelf, l, items),
        Some(l) => repair_loaded_graph(l, items),
    }
}
