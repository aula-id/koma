//! The shelf, goals and reports as the browse screens draw them.
//!
//! The terminal shows a folder tree of the shelf, a collapsible list of goals with
//! their nodes, and a list of reports. All three come from the same shelf rows;
//! this turns them into plain data the client can render without parsing records.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::shelf::{Shelf, ShelfEntry};
use super::trajectory::{self, TrajectoryView};

/// One row of the shelf tree: enough to list it, not the whole record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShelfRow {
    pub key: String,
    pub revision: i64,
    pub updated: i64,
    /// The record kind (`claim`, `finding`, ...), or empty for free text.
    pub kind: String,
    /// A line that says what the row is.
    pub title: String,
    /// `open`, `confirmed`, a verdict, or empty.
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub status: String,
    pub severity: String,
    pub note: String,
    /// The shelf key this node stands for, when it has one.
    pub resource_key: String,
    pub rank: usize,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalEdge {
    pub from: String,
    pub to: String,
    pub rel: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub goal_key: String,
    pub open: usize,
    pub verified: usize,
    pub returned: usize,
    pub nodes: Vec<GoalNode>,
    pub edges: Vec<GoalEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub key: String,
    pub revision: i64,
    pub updated: i64,
    pub title: String,
    pub audience: String,
    pub severity_summary: String,
    pub go_no_go: String,
    pub finding_keys: Vec<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Browse {
    pub shelf: Vec<ShelfRow>,
    pub goals: Vec<Goal>,
    pub reports: Vec<Report>,
}

/// One whole shelf item, for a detail screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub key: String,
    pub revision: i64,
    pub updated: i64,
    pub content: String,
}

const CLIP: usize = 100;

fn clip(text: &str, max: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

fn field(map: &serde_json::Map<String, Value>, names: &[&str]) -> String {
    names.iter().find_map(|n| map.get(*n).and_then(Value::as_str).filter(|s| !s.trim().is_empty())).map(|s| clip(s, CLIP)).unwrap_or_default()
}

fn row_of(entry: &ShelfEntry) -> ShelfRow {
    let content = entry.content.as_deref().unwrap_or("");
    let record = serde_json::from_str::<Value>(content).ok().and_then(|v| v.as_object().cloned());
    let (kind, title, status) = match &record {
        Some(map) => (
            field(map, &["kind"]),
            field(map, &["name", "title", "summary", "host", "goal_key", "claim_key", "markdown", "body"]),
            field(map, &["verdict", "status"]),
        ),
        None => (String::new(), clip(content, CLIP), String::new()),
    };
    ShelfRow { key: entry.key.clone(), revision: entry.revision, updated: entry.updated, kind, title, status }
}

fn goal_of(view: &TrajectoryView) -> Goal {
    let mut nodes: Vec<GoalNode> = view
        .nodes
        .iter()
        .map(|n| GoalNode {
            id: n.id.clone(),
            label: n.label.clone(),
            kind: n.kind.clone(),
            status: n.status.clone().unwrap_or_default(),
            severity: n.severity.clone().unwrap_or_default(),
            note: n.note.clone().unwrap_or_default(),
            resource_key: n.resource_key.clone().unwrap_or_default(),
            rank: n.rank,
            row: n.row,
        })
        .collect();
    nodes.sort_by_key(|n| (n.rank, n.row));
    Goal {
        goal_key: view.goal_key.clone(),
        open: view.counts.open,
        verified: view.counts.verified,
        returned: view.counts.returned,
        nodes,
        edges: view
            .edges
            .iter()
            .map(|e| GoalEdge { from: e.from.clone(), to: e.to.clone(), rel: e.rel.clone().unwrap_or_default() })
            .collect(),
    }
}

fn report_of(entry: &ShelfEntry) -> Option<Report> {
    let map = serde_json::from_str::<Value>(entry.content.as_deref()?).ok()?.as_object()?.clone();
    let text = |name: &str| map.get(name).and_then(Value::as_str).unwrap_or("").to_string();
    let keys = map.get("finding_keys").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let markdown = text("markdown");
    let title = Some(text("title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| clip(&markdown, 80));
    Some(Report {
        key: entry.key.clone(),
        revision: entry.revision,
        updated: entry.updated,
        title,
        audience: text("audience"),
        severity_summary: text("severity_summary"),
        go_no_go: text("go_no_go"),
        finding_keys: keys,
        markdown,
    })
}

/// Everything the three browse screens draw, from the shelf as it is now.
pub fn build(shelf: &Shelf) -> Browse {
    let entries = shelf.scan("").unwrap_or_default();
    let goals = trajectory::project(shelf, None).unwrap_or_default().iter().map(goal_of).collect();
    let reports = entries.iter().filter(|e| e.key.starts_with("reports/")).filter_map(report_of).collect();
    Browse { shelf: entries.iter().map(row_of).collect(), goals, reports }
}

/// One item with its full content, or `None` when the key is not on the shelf.
pub fn item(shelf: &Shelf, key: &str) -> Option<Item> {
    shelf.scan(key).ok()?.into_iter().find(|e| e.key == key).map(|e| Item { key: e.key, revision: e.revision, updated: e.updated, content: e.content.unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shelf() -> Shelf {
        let shelf = Shelf::open_in_memory().expect("shelf");
        let allow: Vec<String> = ["put_claim", "put_trajectory", "put_report", "put", "put_asset"].iter().map(|s| s.to_string()).collect();
        let call = |tool: &str, args: Value| shelf.call_as(&allow, tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}"));
        call("put_claim", json!({ "key": "claims/web/ssrf", "vulnclass": "ssrf", "name": "SSRF in fetch", "severity": "high", "summary": "s", "host": "app", "poc": "curl", "expected_revision": 0 }));
        call("put_asset", json!({ "key": "assets/app", "host": "app", "asset_type": "host", "title": "App server" }));
        call("put_trajectory", json!({ "key": "trajectory/g1", "goal_key": "g1", "nodes": [{ "id": "a", "label": "Map the app" }, { "id": "b", "label": "Try SSRF", "resource_key": "claims/web/ssrf" }], "edges": [{ "from": "a", "to": "b", "rel": "leads_to" }] }));
        call("put_report", json!({ "key": "reports/final", "markdown": "# Final\n\nAll good.", "finding_key": "findings/x", "title": "Final report", "audience": "manager", "expected_revision": 0 }));
        call("put", json!({ "key": "notes/plan", "content": "first line\nsecond", "expected_revision": 0 }));
        shelf
    }

    #[test]
    fn the_shelf_is_listed_with_a_title_per_row() {
        let browse = build(&shelf());
        let by = |key: &str| browse.shelf.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no row {key}")).clone();
        assert_eq!((by("claims/web/ssrf").kind.as_str(), by("claims/web/ssrf").title.as_str()), ("claim", "SSRF in fetch"));
        assert_eq!(by("notes/plan").title, "first line");
        assert_eq!(by("assets/app").title, "App server");
    }

    #[test]
    fn goals_carry_their_nodes_in_order_and_their_edges() {
        let browse = build(&shelf());
        let goal = browse.goals.iter().find(|g| g.goal_key == "g1").expect("goal g1");
        let ids: Vec<&str> = goal.nodes.iter().map(|n| n.id.as_str()).collect();
        let at = |id: &str| ids.iter().position(|i| *i == id).unwrap_or_else(|| panic!("no node {id} in {ids:?}"));
        assert!(at("a") < at("b"), "nodes come in the order of the links: {ids:?}");
        assert!(goal.nodes.iter().any(|n| n.resource_key == "claims/web/ssrf"));
        assert!(goal.edges.iter().any(|e| e.from == "a" && e.to == "b" && e.rel == "leads_to"));
    }

    #[test]
    fn reports_come_with_their_body_and_an_item_opens_whole() {
        let shelf = shelf();
        let browse = build(&shelf);
        assert_eq!(browse.reports.len(), 1);
        let report = &browse.reports[0];
        assert_eq!((report.title.as_str(), report.audience.as_str()), ("Final report", "manager"));
        assert!(report.markdown.contains("All good."));
        let whole = item(&shelf, "notes/plan").expect("item");
        assert_eq!(whole.content, "first line\nsecond");
        assert!(item(&shelf, "notes/nope").is_none());
    }
}
