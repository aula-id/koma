//! Layered layout: a node's column is its link depth, so a chain is as long as
//! its links; a branch stacks its children in the next column; a merge sits in
//! the column after the lanes that feed it, between them.
//!
//! Ported from Orpheus `trajectory-layout.ts`, with rows in terminal lines
//! (pitch 1) in place of pixels. The rank and lane rules are unchanged.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::graph::{Edge, Node};

/// Where one node sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub id: String,
    pub rank: usize,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// One per node, in node order.
    pub cells: Vec<Cell>,
    /// Edges between known, distinct nodes.
    pub edges: Vec<Edge>,
    pub columns: usize,
    pub rows: usize,
}

fn root_bias(node: &Node) -> u8 {
    u8::from(!(node.kind == "start" || node.kind == "org" || node.id == "start" || node.id == "org:scope"))
}

/// Rank = link depth. Roots (no incoming edge, start first) are rank 0; a node
/// is placed once all its predecessors are, one rank after the deepest.
fn assign_ranks<'a>(
    nodes: &'a [Node],
    edges: &'a [Edge],
) -> (HashMap<&'a str, usize>, HashMap<&'a str, Vec<&'a str>>) {
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut incoming: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.id.as_str(), Vec::new())).collect();
    let mut outgoing: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.id.as_str(), Vec::new())).collect();
    let mut indegree: HashMap<&str, i64> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for e in edges {
        let (Some(preds), Some(_)) = (incoming.get(e.to.as_str()), outgoing.get(e.from.as_str())) else {
            continue;
        };
        if preds.contains(&e.from.as_str()) {
            continue;
        }
        incoming.entry(e.to.as_str()).or_default().push(e.from.as_str());
        outgoing.entry(e.from.as_str()).or_default().push(e.to.as_str());
        *indegree.entry(e.to.as_str()).or_default() += 1;
    }

    let mut rank_by_id: HashMap<&str, usize> = HashMap::new();
    let mut placed: HashSet<&str> = HashSet::new();
    let mut queue: Vec<&str> = Vec::new();
    let enqueue = |queue: &mut Vec<&'a str>, placed: &HashSet<&str>, id: &'a str| {
        if queue.contains(&id) || placed.contains(id) {
            return;
        }
        let at = index.get(id).copied().unwrap_or(0);
        let slot = queue.iter().position(|q| index.get(q).copied().unwrap_or(0) > at).unwrap_or(queue.len());
        queue.insert(slot, id);
    };

    let mut roots: Vec<&Node> = nodes.iter().filter(|n| indegree.get(n.id.as_str()).copied().unwrap_or(0) == 0).collect();
    roots.sort_by_key(|n| (root_bias(n), index.get(n.id.as_str()).copied().unwrap_or(0)));
    for root in roots {
        rank_by_id.insert(root.id.as_str(), 0);
        enqueue(&mut queue, &placed, root.id.as_str());
    }

    while placed.len() < nodes.len() {
        if queue.is_empty() {
            // a cycle or an island: take the next unplaced node
            let mut rest: Vec<&Node> = nodes.iter().filter(|n| !placed.contains(n.id.as_str())).collect();
            rest.sort_by_key(|n| (root_bias(n), index.get(n.id.as_str()).copied().unwrap_or(0)));
            let Some(next) = rest.first() else { break };
            let known: Vec<&str> = incoming[next.id.as_str()].iter().copied().filter(|p| placed.contains(p)).collect();
            let base = if let Some(deepest) = known.iter().map(|p| rank_by_id.get(p).copied().unwrap_or(0)).max() {
                deepest + 1
            } else if !placed.is_empty() {
                rank_by_id.values().copied().max().unwrap_or(0) + 1
            } else {
                0
            };
            rank_by_id.insert(next.id.as_str(), base);
            indegree.insert(next.id.as_str(), 0);
            enqueue(&mut queue, &placed, next.id.as_str());
        }
        if queue.is_empty() {
            continue;
        }
        let id = queue.remove(0);
        if !placed.insert(id) {
            continue;
        }
        let rank = rank_by_id.get(id).copied().unwrap_or(0);
        for &to in &outgoing[id] {
            if placed.contains(to) {
                continue;
            }
            let entry = rank_by_id.entry(to).or_insert(0);
            *entry = (*entry).max(rank + 1);
            let left = indegree.get(to).copied().unwrap_or(1) - 1;
            indegree.insert(to, left);
            if left <= 0 {
                enqueue(&mut queue, &placed, to);
            }
        }
    }
    for n in nodes {
        rank_by_id.entry(n.id.as_str()).or_insert(0);
    }
    (rank_by_id, incoming)
}

/// Rows for each node: a child sits level with the mean of its parents, pushed
/// down just enough to keep a row apart from the node above, then the whole
/// column is re-centred on where its parents wanted it.
fn place_lanes<'a>(
    nodes: &'a [Node],
    incoming: &HashMap<&'a str, Vec<&'a str>>,
    rank_by_id: &HashMap<&'a str, usize>,
) -> HashMap<&'a str, f64> {
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut columns: BTreeMap<usize, Vec<&Node>> = BTreeMap::new();
    for n in nodes {
        columns.entry(rank_by_id.get(n.id.as_str()).copied().unwrap_or(0)).or_default().push(n);
    }
    let max_rank = columns.keys().next_back().copied().unwrap_or(0);

    let mut y_by_id: HashMap<&str, f64> = HashMap::new();
    for rank in 0..=max_rank {
        let mut column = columns.remove(&rank).unwrap_or_default();
        let centers: HashMap<&str, Option<f64>> = column
            .iter()
            .map(|n| {
                let ys: Vec<f64> = incoming.get(n.id.as_str()).into_iter().flatten().filter_map(|p| y_by_id.get(p).copied()).collect();
                (n.id.as_str(), (!ys.is_empty()).then(|| ys.iter().sum::<f64>() / ys.len() as f64))
            })
            .collect();
        column.sort_by(|a, b| {
            if rank == 0 {
                let bias = root_bias(a).cmp(&root_bias(b));
                if bias != std::cmp::Ordering::Equal {
                    return bias;
                }
            }
            let by_index = || index[a.id.as_str()].cmp(&index[b.id.as_str()]);
            match (centers[a.id.as_str()], centers[b.id.as_str()]) {
                (None, None) => by_index(),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(ay), Some(by)) => ay.partial_cmp(&by).filter(|o| *o != std::cmp::Ordering::Equal).unwrap_or_else(by_index),
            }
        });

        let mut placed: Vec<f64> = Vec::new();
        let mut wants: Vec<Option<f64>> = Vec::new();
        let mut cursor = f64::NEG_INFINITY;
        for n in &column {
            let want = centers[n.id.as_str()];
            wants.push(want);
            let y = if placed.is_empty() { want.unwrap_or(0.0) } else { want.unwrap_or(cursor + 1.0).max(cursor + 1.0) };
            placed.push(y);
            cursor = y;
        }
        let known: Vec<f64> = wants.iter().copied().flatten().collect();
        let want_mid = if known.is_empty() { placed.first().copied().unwrap_or(0.0) } else { known.iter().sum::<f64>() / known.len() as f64 };
        let placed_mid = match (placed.first(), placed.last()) {
            (Some(first), Some(last)) => (first + last) / 2.0,
            _ => want_mid,
        };
        let shift = want_mid - placed_mid;
        for (n, y) in column.iter().zip(&placed) {
            y_by_id.insert(n.id.as_str(), y + shift);
        }
    }
    y_by_id
}

pub fn layout(nodes: &[Node], edges: &[Edge]) -> Layout {
    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let valid: Vec<Edge> = edges
        .iter()
        .filter(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()) && e.from != e.to)
        .cloned()
        .collect();
    if nodes.is_empty() {
        return Layout::default();
    }
    let (ranks, incoming) = assign_ranks(nodes, &valid);
    let ys = place_lanes(nodes, &incoming, &ranks);
    let min_y = ys.values().copied().fold(f64::INFINITY, f64::min);
    let min_y = if min_y.is_finite() { min_y } else { 0.0 };
    let cells: Vec<Cell> = nodes
        .iter()
        .map(|n| Cell {
            id: n.id.clone(),
            rank: ranks.get(n.id.as_str()).copied().unwrap_or(0),
            row: (ys.get(n.id.as_str()).copied().unwrap_or(0.0) - min_y).round().max(0.0) as usize,
        })
        .collect();
    let columns = cells.iter().map(|c| c.rank).max().map_or(0, |m| m + 1);
    let rows = cells.iter().map(|c| c.row).max().map_or(0, |m| m + 1);
    Layout { cells, edges: valid, columns, rows }
}
