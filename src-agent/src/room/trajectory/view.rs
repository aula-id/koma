//! The projection: everything a UI or the CLI needs to draw one goal's graph.
//!
//! Computed in the daemon so the terminal UI, headless `koma room trajectory`
//! and any later GUI all draw the same thing from one source of truth.

use serde::Serialize;

use super::gate::{self, Counts};
use super::graph::{merge_with_shelf, parse_trajectory, Edge, Graph};
use super::layout::layout;
use crate::room::shelf::{Shelf, ShelfEntry};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub status: Option<String>,
    pub severity: Option<String>,
    pub note: Option<String>,
    pub resource_key: Option<String>,
    /// Revision of the shelf row this node stands for, when there is one.
    pub revision: Option<i64>,
    pub rank: usize,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrajectoryView {
    pub goal_key: String,
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<Edge>,
    pub counts: Counts,
    pub columns: usize,
    pub rows: usize,
}

/// Build one view from a merged graph and the shelf rows behind it.
pub fn build(graph: Graph, items: &[ShelfEntry]) -> TrajectoryView {
    let placed = layout(&graph.nodes, &graph.edges);
    let nodes = graph
        .nodes
        .into_iter()
        .zip(&placed.cells)
        .map(|(n, cell)| {
            let revision = n
                .resource_key
                .as_deref()
                .or(Some(n.id.as_str()))
                .and_then(|k| items.iter().find(|i| i.key == k))
                .map(|i| i.revision);
            ViewNode {
                id: n.id,
                label: n.label,
                kind: n.kind,
                status: n.status,
                severity: n.severity,
                note: n.note,
                resource_key: n.resource_key,
                revision,
                rank: cell.rank,
                row: cell.row,
            }
        })
        .collect();
    TrajectoryView {
        goal_key: graph.goal_key,
        nodes,
        edges: placed.edges,
        counts: gate::count(items),
        columns: placed.columns,
        rows: placed.rows,
    }
}

/// One view per goal: every `trajectory/` record on the shelf, each merged with
/// the shelf's gate rows. With no published trajectory the shelf alone is drawn.
/// `goal` limits the result to one goal key (with or without its `trajectory/`
/// prefix). An empty shelf gives no views.
pub fn project(shelf: &Shelf, goal: Option<&str>) -> Result<Vec<TrajectoryView>, String> {
    let items = shelf.scan("").map_err(|e| e.0)?;
    let wanted = goal.map(|g| g.strip_prefix("trajectory/").unwrap_or(g).to_string());
    let loaded: Vec<(String, Graph)> = items
        .iter()
        .filter_map(|i| {
            let key = i.key.strip_prefix("trajectory/")?.to_string();
            let graph = parse_trajectory(i.content.as_deref()?, &key)?;
            Some((key, graph))
        })
        .collect();

    let mut views: Vec<TrajectoryView> = loaded
        .iter()
        .filter(|(key, g)| wanted.as_deref().is_none_or(|w| key == w || g.goal_key == w))
        .map(|(key, g)| build(merge_with_shelf(Some(g), &items, Some(key)), &items))
        .collect();

    if loaded.is_empty() && wanted.is_none() && !items.is_empty() {
        let graph = merge_with_shelf(None, &items, None);
        if graph.nodes.len() > 1 {
            views.push(build(graph, &items));
        }
    }
    Ok(views)
}
