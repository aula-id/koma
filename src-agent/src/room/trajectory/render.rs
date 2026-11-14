//! Drawing a view as text. The outline tree is the default: it reads at any
//! terminal width. The layered view mirrors the Orpheus graph for wide
//! terminals; `dot`, `mermaid` and `json` are for reports and tools.
//!
//! Glyphs carry meaning by shape as well as colour, so the output reads without
//! colour: `○` open, `◐` in progress, `●` verified, `↩` returned, `✕` dismissed,
//! `◌` an exhausted goal, `◆` the start.

use std::collections::{HashMap, HashSet};

use super::view::{TrajectoryView, ViewNode};
use crate::room::work::is_exhausted;

pub fn glyph(node: &ViewNode) -> char {
    if node.kind == "start" {
        return '◆';
    }
    let status = node.status.as_deref().unwrap_or("").trim().to_lowercase();
    match status.as_str() {
        "verified" => '●',
        "returned" => '↩',
        "dismissed" | "discarded" => '✕',
        "review-ok" | "in progress" | "in-progress" | "running" | "active" => '◐',
        _ if node.kind == "goal" && !status.is_empty() && is_exhausted(&status) => '◌',
        _ => '○',
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn summary(view: &TrajectoryView) -> String {
    let c = view.counts;
    format!(
        "goal: {}   ● {} verified · ○ {} open · ↩ {} returned · {} reports",
        view.goal_key, c.verified, c.open, c.returned, c.reports
    )
}

fn node_line(node: &ViewNode) -> String {
    let severity = node.severity.as_deref().filter(|s| !s.is_empty()).map(|s| format!(" [{s}]")).unwrap_or_default();
    let revision = node.revision.map(|r| format!("  rev {r}")).unwrap_or_default();
    format!("{} {}: {}{severity}{revision}", glyph(node), node.kind, node.label)
}

/// The tree from the start node. A node reached twice is shown once in full and
/// referenced after that; nodes the start cannot reach are listed at the end.
pub fn outline(view: &TrajectoryView) -> String {
    let mut out = format!("{}\n", summary(view));
    if view.nodes.is_empty() {
        out.push_str("(nothing on the shelf yet)\n");
        return out;
    }
    let by_id: HashMap<&str, &ViewNode> = view.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &view.edges {
        children.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    let root = view.nodes.iter().find(|n| n.kind == "start").unwrap_or(&view.nodes[0]);
    let mut seen: HashSet<&str> = HashSet::new();

    /// The read-only shape of the graph the walk follows.
    struct Tree<'a> {
        by_id: HashMap<&'a str, &'a ViewNode>,
        children: HashMap<&'a str, Vec<&'a str>>,
    }

    /// Where a line sits in the drawing: the bar prefix, and whether it is the
    /// last sibling or the top of the tree.
    #[derive(Clone, Copy)]
    enum Place<'p> {
        Top,
        Child { prefix: &'p str, last: bool },
    }

    fn walk<'a>(tree: &Tree<'a>, id: &'a str, place: Place<'_>, seen: &mut HashSet<&'a str>, out: &mut String) {
        let Some(node) = tree.by_id.get(id) else { return };
        let (branch, next_prefix) = match place {
            Place::Top => (String::new(), String::new()),
            Place::Child { prefix, last } => (
                format!("{prefix}{}", if last { "└─ " } else { "├─ " }),
                format!("{prefix}{}", if last { "   " } else { "│  " }),
            ),
        };
        if !seen.insert(id) {
            out.push_str(&format!("{branch}↪ {} (shown above)\n", node.label));
            return;
        }
        out.push_str(&format!("{branch}{}\n", node_line(node)));
        let kids = tree.children.get(id).cloned().unwrap_or_default();
        for (i, kid) in kids.iter().enumerate() {
            walk(tree, kid, Place::Child { prefix: &next_prefix, last: i + 1 == kids.len() }, seen, out);
        }
    }
    let tree = Tree { by_id, children };
    walk(&tree, root.id.as_str(), Place::Top, &mut seen, &mut out);

    let unlinked: Vec<&ViewNode> = view.nodes.iter().filter(|n| !seen.contains(n.id.as_str())).collect();
    if !unlinked.is_empty() {
        out.push_str("unlinked:\n");
        for n in unlinked {
            out.push_str(&format!("  {}\n", node_line(n)));
        }
    }
    out
}

/// Columns by link depth, rows by lane. A straight link into the next column
/// is drawn as an arrow; every other link is listed underneath. `None` when
/// the graph is wider than `max_width`, so the caller can use the outline.
pub fn layered(view: &TrajectoryView, max_width: usize) -> Option<String> {
    if view.nodes.is_empty() {
        return None;
    }
    let text = |n: &ViewNode| clip(&format!("{} {}", glyph(n), n.label), 26);
    let mut width = vec![0usize; view.columns];
    for n in &view.nodes {
        width[n.rank] = width[n.rank].max(text(n).chars().count());
    }
    let arrow = " ─▶ ";
    let total: usize = width.iter().sum::<usize>() + arrow.chars().count() * view.columns.saturating_sub(1);
    if total > max_width {
        return None;
    }
    let at: HashMap<(usize, usize), &ViewNode> = view.nodes.iter().map(|n| ((n.rank, n.row), n)).collect();
    let straight: HashSet<(&str, &str)> = view
        .edges
        .iter()
        .filter_map(|e| {
            let from = view.nodes.iter().find(|n| n.id == e.from)?;
            let to = view.nodes.iter().find(|n| n.id == e.to)?;
            (to.rank == from.rank + 1 && to.row == from.row).then_some((e.from.as_str(), e.to.as_str()))
        })
        .collect();

    let mut out = format!("{}\n", summary(view));
    for row in 0..view.rows {
        let mut line = String::new();
        for (rank, column_width) in width.iter().enumerate() {
            let cell = at.get(&(rank, row));
            let body = cell.map(|n| text(n)).unwrap_or_default();
            line.push_str(&format!("{body:<w$}", w = *column_width));
            if rank + 1 < view.columns {
                let drawn = cell
                    .zip(at.get(&(rank + 1, row)))
                    .is_some_and(|(a, b)| straight.contains(&(a.id.as_str(), b.id.as_str())));
                line.push_str(if drawn { arrow } else { "    " });
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let label = |id: &str| view.nodes.iter().find(|n| n.id == id).map_or(id.to_string(), |n| n.label.clone());
    let other: Vec<String> = view
        .edges
        .iter()
        .filter(|e| !straight.contains(&(e.from.as_str(), e.to.as_str())))
        .map(|e| format!("  {} ─{}▶ {}", label(&e.from), e.rel.as_deref().unwrap_or(""), label(&e.to)))
        .collect();
    if !other.is_empty() {
        out.push_str("links:\n");
        out.push_str(&other.join("\n"));
        out.push('\n');
    }
    Some(out)
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
}

pub fn dot(view: &TrajectoryView) -> String {
    let mut out = format!("digraph \"{}\" {{\n  rankdir=LR;\n  node [shape=box];\n", escape(&view.goal_key));
    for n in &view.nodes {
        out.push_str(&format!("  \"{}\" [label=\"{} {}\"];\n", escape(&n.id), glyph(n), escape(&n.label)));
    }
    for e in &view.edges {
        out.push_str(&format!("  \"{}\" -> \"{}\" [label=\"{}\"];\n", escape(&e.from), escape(&e.to), escape(e.rel.as_deref().unwrap_or(""))));
    }
    out.push_str("}\n");
    out
}

pub fn mermaid(view: &TrajectoryView) -> String {
    let ids: HashMap<&str, usize> = view.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut out = String::from("flowchart LR\n");
    for (i, n) in view.nodes.iter().enumerate() {
        out.push_str(&format!("  n{i}[\"{} {}\"]\n", glyph(n), escape(&n.label).replace('"', "'")));
    }
    for e in &view.edges {
        if let (Some(a), Some(b)) = (ids.get(e.from.as_str()), ids.get(e.to.as_str())) {
            match e.rel.as_deref().filter(|r| !r.is_empty()) {
                Some(rel) => out.push_str(&format!("  n{a} -->|{}| n{b}\n", escape(rel))),
                None => out.push_str(&format!("  n{a} --> n{b}\n")),
            }
        }
    }
    out
}

pub fn json(views: &[TrajectoryView]) -> Result<String, String> {
    serde_json::to_string_pretty(views).map_err(|e| e.to_string())
}
