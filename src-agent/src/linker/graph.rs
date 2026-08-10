//! In-memory directed import graph.
//!
//! Nodes are canonical file paths; edges represent import/module relationships
//! extracted by tree-sitter. A reverse index enables fast dependents/impact
//! queries without traversal.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ipc::linker_proto::{
    GraphDirection, GraphNodeRole, GraphViewEdge, GraphViewNode, GraphViewResult,
    VisualizationRequest,
};

/// Supported source languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    Go,
    Java,
    TypeScript,
    JavaScript,
    Php,
    C,
    Cpp,
    Dart,
    Swift,
    Unknown,
}

/// A directed edge from a source file to a target.
#[derive(Debug, Clone)]
pub struct Edge {
    pub target: EdgeTarget,
    /// Reserved for future mod-edge discrimination; always `Import` today.
    #[allow(dead_code)]
    pub kind: EdgeKind,
}

/// Where an import points — either a resolved file or an external (unresolved) specifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeTarget {
    /// Canonical file path within the scanned workspace.
    File(String),
    /// External / unresolved import string (e.g. crate name, npm package).
    External(String),
}

/// The kind of import relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// `use`, `import`, `require`, etc.
    Import,
    /// `mod` declaration (Rust-specific). Reserved for future use.
    #[allow(dead_code)]
    Mod,
}

/// A node in the graph — a source file.
#[derive(Debug, Clone)]
pub struct Node {
    pub lang: Lang,
    /// Canonical path (identity); stored for future tooling.
    #[allow(dead_code)]
    pub path: String,
}

/// The complete import graph for one or more workspace roots.
#[derive(Debug, Default)]
pub struct ImportGraph {
    /// Canonical path → Node
    pub nodes: HashMap<String, Node>,
    /// Source path → outgoing edges
    pub edges: HashMap<String, Vec<Edge>>,
    /// Target (file path or external string) → list of source paths (reverse index)
    pub reverse: HashMap<String, Vec<String>>,
    /// Total files tracked.
    pub file_count: usize,
    /// Total edges tracked.
    pub edge_count: usize,
    /// Monotonically increasing scan generation counter.
    pub generation: u64,
}

impl ImportGraph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all data and increment generation.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.reverse.clear();
        self.file_count = 0;
        self.edge_count = 0;
        self.generation += 1;
    }

    /// Ensure a node exists for the given path and language.
    pub fn ensure_node(&mut self, path: &str, lang: Lang) {
        self.nodes.entry(path.to_string()).or_insert_with(|| Node {
            lang,
            path: path.to_string(),
        });
    }

    /// Replace all edges for a source file, updating the reverse index.
    pub fn set_edges(&mut self, source: &str, lang: Lang, new_edges: Vec<Edge>) {
        // Ensure the source node exists.
        self.ensure_node(source, lang);

        // Remove old reverse entries for this source.
        if let Some(old_edges) = self.edges.remove(source) {
            for old in &old_edges {
                let key = edge_target_key(&old.target);
                if let Some(list) = self.reverse.get_mut(key) {
                    list.retain(|s| s != source);
                    if list.is_empty() {
                        self.reverse.remove(key);
                    }
                }
                self.edge_count -= 1;
            }
        }

        // Add new edges.
        let count = new_edges.len();
        for edge in &new_edges {
            let key = edge_target_key(&edge.target);
            self.reverse
                .entry(key.to_string())
                .or_default()
                .push(source.to_string());
        }
        self.edge_count += count;
        self.edges.insert(source.to_string(), new_edges);
        self.file_count = self.nodes.len();
    }

    /// Remove a node and all its edges (both incoming and outgoing).
    pub fn remove_node(&mut self, path: &str) {
        // Remove outgoing edges and their reverse entries.
        if let Some(outgoing) = self.edges.remove(path) {
            for edge in &outgoing {
                let key = edge_target_key(&edge.target);
                if let Some(list) = self.reverse.get_mut(key) {
                    list.retain(|s| s != path);
                    if list.is_empty() {
                        self.reverse.remove(key);
                    }
                }
                self.edge_count -= 1;
            }
        }

        // Remove incoming edges (files that import this one).
        if let Some(incoming) = self.reverse.remove(path) {
            for source in &incoming {
                if let Some(edges) = self.edges.get_mut(source) {
                    edges.retain(|e| edge_target_key(&e.target) != path);
                }
            }
        }

        self.nodes.remove(path);
        self.file_count = self.nodes.len();
    }

    /// Resolve a query path to a graph node key.
    ///
    /// 1. Normalize: backslash → `/`, strip trailing `/`.
    /// 2. Exact match in `nodes` or `reverse` → return that key.
    /// 3. Unique suffix match (`key` ends with `/{q}` or equals `q`) → return that key.
    /// 4. Otherwise `None`.
    ///
    /// Suffix matching requires a `/` boundary so `foo.rs` doesn't match `barfoo.rs`.
    /// If multiple nodes match the suffix, returns `None` (ambiguous).
    pub fn resolve_key<'a>(&'a self, path: &str) -> Option<&'a str> {
        let q = path.replace('\\', "/");
        let q = q.trim_end_matches('/');
        if q.is_empty() {
            return None;
        }

        // Exact match in nodes — return the actual key from the map.
        if let Some(key) = self.nodes.keys().find(|k| k.as_str() == q) {
            return Some(key.as_str());
        }
        // Exact match in reverse index.
        if let Some(key) = self.reverse.keys().find(|k| k.as_str() == q) {
            return Some(key.as_str());
        }

        // Unique suffix match: key == q || key.ends_with("/" + q).
        let mut candidates: Vec<&str> = Vec::new();
        for key in self.nodes.keys() {
            if key == q || key.ends_with(&format!("/{q}")) {
                candidates.push(key.as_str());
            }
        }
        match candidates.len() {
            1 => Some(candidates[0]),
            _ => None,
        }
    }

    /// Direct dependencies of a file (outgoing edges resolved to file paths).
    pub fn dependencies(&self, path: &str) -> Vec<&str> {
        self.edges
            .get(path)
            .map(|es| {
                es.iter()
                    .filter_map(|e| match &e.target {
                        EdgeTarget::File(f) => Some(f.as_str()),
                        EdgeTarget::External(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Direct dependents of a file (files that import it).
    pub fn dependents(&self, path: &str) -> Vec<&str> {
        self.reverse
            .get(path)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Transitive impact set: all files that transitively depend on `path`,
    /// up to `max_depth` hops (BFS). Includes `path` itself.
    pub fn impact<'a>(&'a self, path: &'a str, max_depth: u32) -> Vec<&'a str> {
        let mut visited = std::collections::HashSet::new();
        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back((path, 0u32));
        visited.insert(path);

        while let Some((current, depth)) = queue.pop_front() {
            result.push(current);
            if depth < max_depth {
                if let Some(sources) = self.reverse.get(current) {
                    for source in sources {
                        if visited.insert(source.as_str()) {
                            queue.push_back((source.as_str(), depth + 1));
                        }
                    }
                }
            }
        }
        result
    }

    /// 1-hop neighborhood: (dependencies, dependents).
    pub fn neighborhood(&self, path: &str) -> (Vec<&str>, Vec<&str>) {
        (self.dependencies(path), self.dependents(path))
    }

    /// Set of languages present in the graph.
    pub fn languages(&self) -> Vec<String> {
        let mut langs = std::collections::HashSet::new();
        for node in self.nodes.values() {
            langs.insert(format!("{:?}", node.lang));
        }
        let mut v: Vec<String> = langs.into_iter().collect();
        v.sort();
        v
    }

    /// Top N files by fan-in (most depended upon).
    pub fn top_fan_in(&self, n: usize) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = self
            .reverse
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect();
        counts.sort_by_key(|b| std::cmp::Reverse(b.1));
        counts.truncate(n);
        counts
    }

    /// Entry points: files with zero dependents (nobody imports them), up to `n`.
    pub fn entry_points(&self, n: usize) -> Vec<String> {
        let mut eps: Vec<String> = self
            .nodes
            .keys()
            .filter(|k| self.reverse.get(*k).is_none_or(|v| v.is_empty()))
            .cloned()
            .collect();
        eps.sort();
        eps.truncate(n);
        eps
    }

    /// Build a bounded subgraph view for GUI visualization.
    ///
    /// If `req.path` is `None`, returns overview metadata with top fan-in and entry-point
    /// nodes. If `req.path` is `Some`, BFS from the focal node in the requested direction(s).
    pub fn visualization_view(&self, req: &VisualizationRequest) -> GraphViewResult {
        let max_nodes = if req.max_nodes == 0 {
            200
        } else {
            req.max_nodes
        };
        let max_edges = if req.max_edges == 0 {
            400
        } else {
            req.max_edges
        };
        let depth = req.depth.clamp(1, 3);

        let base = GraphViewResult {
            nodes: Vec::new(),
            edges: Vec::new(),
            focus: None,
            generation: self.generation,
            file_count: self.file_count,
            edge_count: self.edge_count,
            languages: self.languages(),
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 0,
            total_edges_available: 0,
        };

        match &req.path {
            None => self.overview_view(base, max_nodes),
            Some(path) => match self.resolve_key(path) {
                None => GraphViewResult {
                    nodes_truncated: true,
                    edges_truncated: true,
                    total_nodes_available: 0,
                    total_edges_available: 0,
                    ..base
                },
                Some(key) => {
                    let key = key.to_string();
                    self.focus_view(base, &key, &req.direction, depth, max_nodes, max_edges)
                }
            },
        }
    }

    /// Build an overview (no focal file): top fan-in + entry points, all Overview role.
    fn overview_view(&self, base: GraphViewResult, max_nodes: usize) -> GraphViewResult {
        let mut paths: Vec<String> = Vec::new();
        // Top fan-in entries.
        for (path, _count) in self.top_fan_in(max_nodes) {
            paths.push(path);
        }
        // Entry points.
        for ep in self.entry_points(max_nodes) {
            if !paths.contains(&ep) {
                paths.push(ep);
            }
        }
        paths.sort();
        paths.dedup();
        paths.truncate(max_nodes);

        let total = paths.len();
        let nodes: Vec<GraphViewNode> = paths
            .iter()
            .map(|p| {
                let lang = self
                    .nodes
                    .get(p)
                    .map(|n| format!("{:?}", n.lang))
                    .unwrap_or_default();
                GraphViewNode {
                    path: p.clone(),
                    language: lang,
                    out_degree: self.dependencies(p).len(),
                    in_degree: self.dependents(p).len(),
                    role: GraphNodeRole::Overview,
                    depth_from_focus: None,
                }
            })
            .collect();

        GraphViewResult {
            nodes,
            edges: Vec::new(),
            focus: None,
            total_nodes_available: total,
            total_edges_available: 0,
            ..base
        }
    }

    /// Build a focus view: BFS from `key` in the requested direction(s).
    fn focus_view(
        &self,
        base: GraphViewResult,
        key: &str,
        direction: &GraphDirection,
        depth: u32,
        max_nodes: usize,
        max_edges: usize,
    ) -> GraphViewResult {
        // BFS to discover nodes + edges within the view.
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        // (path, depth_from_focus)
        let mut discovered: Vec<(String, u32)> = Vec::new();
        let mut raw_edges: Vec<(String, String)> = Vec::new(); // (from, to)
        // Track whether each non-focus node was discovered via outgoing (dependency)
        // or incoming (dependent) edge traversal.
        let mut is_dependency_set: HashSet<String> = HashSet::new();
        let mut is_dependent_set: HashSet<String> = HashSet::new();

        queue.push_back((key.to_string(), 0));
        visited.insert(key.to_string());

        while let Some((current, d)) = queue.pop_front() {
            discovered.push((current.clone(), d));
            if d >= depth {
                continue;
            }

            // Traverse outgoing edges (dependencies) if direction allows.
            if matches!(
                direction,
                GraphDirection::Dependencies | GraphDirection::Both
            ) {
                if let Some(outgoing) = self.edges.get(&current) {
                    for edge in outgoing {
                        if let crate::linker::graph::EdgeTarget::File(target) = &edge.target {
                            if visited.insert(target.clone()) {
                                raw_edges.push((current.clone(), target.clone()));
                                is_dependency_set.insert(target.clone());
                                queue.push_back((target.clone(), d + 1));
                            }
                        }
                    }
                }
            }

            // Traverse incoming edges (dependents) if direction allows.
            if matches!(
                direction,
                GraphDirection::Dependents | GraphDirection::Both
            ) {
                if let Some(incoming) = self.reverse.get(&current) {
                    for source in incoming {
                        if visited.insert(source.clone()) {
                            raw_edges.push((source.clone(), current.clone()));
                            is_dependent_set.insert(source.clone());
                            queue.push_back((source.clone(), d + 1));
                        }
                    }
                }
            }
        }

        let total_nodes_available = discovered.len();
        let total_edges_available = raw_edges.len();

        // Build sortable node list: focal first, then BFS depth, then fan-in (reverse), then lex path.
        let mut sortable: Vec<(usize, u32, usize, String, u32)> = discovered
            .iter()
            .map(|(p, d)| {
                let is_focus = p == key;
                let fan_in = self.reverse.get(p).map_or(0, |v| v.len());
                let (role_rank, fan_in_prio) = if is_focus {
                    (0, 0)
                } else if is_dependency_set.contains(p.as_str()) {
                    (1, usize::MAX - fan_in)
                } else if is_dependent_set.contains(p.as_str()) {
                    (2, usize::MAX - fan_in)
                } else {
                    (3, usize::MAX - fan_in)
                };
                (role_rank, *d, fan_in_prio, p.clone(), *d)
            })
            .collect();
        sortable.sort();

        // Apply node cap.
        let nodes_truncated = sortable.len() > max_nodes;
        sortable.truncate(max_nodes);

        let node_set: HashSet<&str> = sortable.iter().map(|(_, _, _, p, _)| p.as_str()).collect();

        // Filter edges to only those within the capped node set, then apply edge cap.
        let mut filtered_edges: Vec<(String, String)> = raw_edges
            .into_iter()
            .filter(|(from, to)| node_set.contains(from.as_str()) && node_set.contains(to.as_str()))
            .collect();
        let edges_truncated = total_edges_available > max_edges;
        let filtered_len = filtered_edges.len();
        if filtered_len > max_edges {
            filtered_edges.truncate(max_edges);
        }

        // Sort edges for stable ordering.
        filtered_edges.sort();

        // Count in/out degree within the view for each node.
        let mut in_deg: HashMap<&str, usize> = HashMap::new();
        let mut out_deg: HashMap<&str, usize> = HashMap::new();
        for (from, to) in &filtered_edges {
            *out_deg.entry(from.as_str()).or_insert(0) += 1;
            *in_deg.entry(to.as_str()).or_insert(0) += 1;
        }

        let nodes: Vec<GraphViewNode> = sortable
            .into_iter()
            .map(|(_, d, _, p, _)| {
                let lang = self
                    .nodes
                    .get(&p)
                    .map(|n| format!("{:?}", n.lang))
                    .unwrap_or_default();
                let role = if p == key {
                    GraphNodeRole::Focus
                } else if is_dependency_set.contains(p.as_str()) {
                    GraphNodeRole::Dependency
                } else if is_dependent_set.contains(p.as_str()) {
                    GraphNodeRole::Dependent
                } else {
                    GraphNodeRole::Overview
                };
                GraphViewNode {
                    path: p.clone(),
                    language: lang,
                    out_degree: *out_deg.get(p.as_str()).unwrap_or(&0),
                    in_degree: *in_deg.get(p.as_str()).unwrap_or(&0),
                    role,
                    depth_from_focus: Some(d),
                }
            })
            .collect();

        let edges: Vec<GraphViewEdge> = filtered_edges
            .into_iter()
            .map(|(from, to)| GraphViewEdge { from, to })
            .collect();

        GraphViewResult {
            nodes,
            edges,
            focus: Some(key.to_string()),
            nodes_truncated,
            edges_truncated,
            total_nodes_available,
            total_edges_available,
            ..base
        }
    }
}

/// Extract the string key used in the reverse index for a given target.
fn edge_target_key(target: &EdgeTarget) -> &str {
    match target {
        EdgeTarget::File(f) => f,
        EdgeTarget::External(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_edges_and_reverse_index() {
        let mut g = ImportGraph::new();
        g.set_edges(
            "a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        assert_eq!(g.file_count, 1);
        assert_eq!(g.edge_count, 1);
        assert_eq!(g.dependencies("a.rs"), vec!["b.rs"]);
        assert_eq!(g.dependents("b.rs"), vec!["a.rs"]);
    }

    #[test]
    fn remove_node_cleans_both_indexes() {
        let mut g = ImportGraph::new();
        g.set_edges(
            "a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "b.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("c.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.file_count = g.nodes.len();

        g.remove_node("b.rs");
        assert!(!g.edges.contains_key("a.rs") || g.edges["a.rs"].is_empty());
        assert!(!g.reverse.contains_key("b.rs"));
    }

    #[test]
    fn impact_bfs() {
        let mut g = ImportGraph::new();
        g.set_edges(
            "a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "b.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("c.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.file_count = g.nodes.len();

        let impact = g.impact("c.rs", 10);
        assert!(impact.contains(&"c.rs"));
        assert!(impact.contains(&"b.rs"));
        assert!(impact.contains(&"a.rs"));
    }

    #[test]
    fn graph_clear_and_generation() {
        let mut g = ImportGraph::new();
        assert_eq!(g.generation, 0);
        g.set_edges(
            "a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.file_count = g.nodes.len();
        assert_eq!(g.file_count, 1);
        assert_eq!(g.generation, 0); // set_edges doesn't bump generation

        g.clear();
        assert_eq!(g.nodes.len(), 0);
        assert_eq!(g.generation, 1); // clear bumps generation
    }

    // --- resolve_key tests ---

    #[test]
    fn resolve_key_exact_hit() {
        let mut g = ImportGraph::new();
        g.ensure_node("/abs/src/main.rs", Lang::Rust);
        assert_eq!(g.resolve_key("/abs/src/main.rs"), Some("/abs/src/main.rs"));
    }

    #[test]
    fn resolve_key_unique_suffix() {
        let mut g = ImportGraph::new();
        g.ensure_node("/abs/project/src/tool/mod.rs", Lang::Rust);
        // Query with relative path that uniquely suffix-matches.
        assert_eq!(
            g.resolve_key("src/tool/mod.rs"),
            Some("/abs/project/src/tool/mod.rs")
        );
    }

    #[test]
    fn resolve_key_ambiguous_two_roots() {
        let mut g = ImportGraph::new();
        g.ensure_node("/root_a/src/main.rs", Lang::Rust);
        g.ensure_node("/root_b/src/main.rs", Lang::Rust);
        // Ambiguous — same suffix in two roots.
        assert_eq!(g.resolve_key("src/main.rs"), None);
    }

    #[test]
    fn resolve_key_no_false_prefix() {
        let mut g = ImportGraph::new();
        g.ensure_node("/abs/barfoo.rs", Lang::Rust);
        // "foo.rs" should NOT match "barfoo.rs".
        assert_eq!(g.resolve_key("foo.rs"), None);
    }

    #[test]
    fn resolve_key_empty_returns_none() {
        let g = ImportGraph::new();
        assert_eq!(g.resolve_key(""), None);
    }

    #[test]
    fn resolve_key_trailing_slash_stripped() {
        let mut g = ImportGraph::new();
        g.ensure_node("/abs/src/main.rs", Lang::Rust);
        assert_eq!(g.resolve_key("/abs/src/main.rs/"), Some("/abs/src/main.rs"));
    }

    // --- visualization_view tests ---

    use crate::ipc::linker_proto::{
        GraphDirection, GraphNodeRole, VisualizationRequest,
    };

    /// Helper: build a linear chain A→B→C.
    fn chain_graph() -> ImportGraph {
        let mut g = ImportGraph::new();
        g.set_edges(
            "/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "/b.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/c.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/c.rs", Lang::Rust, vec![]);
        g
    }

    #[test]
    fn visualization_view_overview_returns_metadata() {
        let g = chain_graph();
        let req = VisualizationRequest {
            path: None,
            depth: 1,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
        };
        let result = g.visualization_view(&req);
        assert!(result.focus.is_none());
        assert!(!result.nodes.is_empty());
        for node in &result.nodes {
            assert_eq!(node.role, GraphNodeRole::Overview);
            assert!(node.depth_from_focus.is_none());
        }
        assert!(result.edges.is_empty());
        assert_eq!(result.file_count, 3);
    }

    #[test]
    fn visualization_view_focus_dependencies() {
        let g = chain_graph();
        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.focus.as_deref(), Some("/a.rs"));
        // A→B→C: all three should be discovered
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        // Focus node
        let focus = result.nodes.iter().find(|n| n.role == GraphNodeRole::Focus).unwrap();
        assert_eq!(focus.path, "/a.rs");
        assert_eq!(focus.depth_from_focus, Some(0));
        // B is dependency at depth 1
        let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
        assert_eq!(b.role, GraphNodeRole::Dependency);
        assert_eq!(b.depth_from_focus, Some(1));
        // C is dependency at depth 2
        let c = result.nodes.iter().find(|n| n.path == "/c.rs").unwrap();
        assert_eq!(c.role, GraphNodeRole::Dependency);
        assert_eq!(c.depth_from_focus, Some(2));
    }

    #[test]
    fn visualization_view_focus_dependents() {
        let g = chain_graph();
        let req = VisualizationRequest {
            path: Some("/c.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependents,
            max_nodes: 100,
            max_edges: 100,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.focus.as_deref(), Some("/c.rs"));
        // C ← B ← A: all three discovered
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        let focus = result.nodes.iter().find(|n| n.role == GraphNodeRole::Focus).unwrap();
        assert_eq!(focus.depth_from_focus, Some(0));
        // B is dependent at depth 1
        let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
        assert_eq!(b.role, GraphNodeRole::Dependent);
        assert_eq!(b.depth_from_focus, Some(1));
        // A is dependent at depth 2
        let a = result.nodes.iter().find(|n| n.path == "/a.rs").unwrap();
        assert_eq!(a.role, GraphNodeRole::Dependent);
        assert_eq!(a.depth_from_focus, Some(2));
    }

    #[test]
    fn visualization_view_focus_both() {
        // Diamond: A→B, A→C, B→D, C→D
        let mut g = ImportGraph::new();
        g.set_edges(
            "/a.rs",
            Lang::Rust,
            vec![
                Edge { target: EdgeTarget::File("/b.rs".into()), kind: EdgeKind::Import },
                Edge { target: EdgeTarget::File("/c.rs".into()), kind: EdgeKind::Import },
            ],
        );
        g.set_edges(
            "/b.rs",
            Lang::Rust,
            vec![Edge { target: EdgeTarget::File("/d.rs".into()), kind: EdgeKind::Import }],
        );
        g.set_edges(
            "/c.rs",
            Lang::Rust,
            vec![Edge { target: EdgeTarget::File("/d.rs".into()), kind: EdgeKind::Import }],
        );
        g.set_edges("/d.rs", Lang::Rust, vec![]);

        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
        };
        let result = g.visualization_view(&req);
        // Focus A, deps B,C,D (all outgoing)
        assert_eq!(result.nodes.len(), 4);
        assert_eq!(result.edges.len(), 3); // A→B, A→C, B→D (C→D already visited via B→D? no — depends on BFS order)
        // All non-focus nodes should be Dependency (no dependents of A in this graph)
        for node in &result.nodes {
            if node.path != "/a.rs" {
                assert_ne!(node.role, GraphNodeRole::Dependent);
            }
        }
    }

    #[test]
    fn visualization_view_bounded_depth() {
        let g = chain_graph();
        // depth=1: only A and its direct dep B
        let req1 = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 1,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
        };
        let r1 = g.visualization_view(&req1);
        assert_eq!(r1.nodes.len(), 2);
        assert_eq!(r1.edges.len(), 1);

        // depth=2: A→B→C
        let req2 = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 2,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
        };
        let r2 = g.visualization_view(&req2);
        assert_eq!(r2.nodes.len(), 3);
        assert_eq!(r2.edges.len(), 2);
    }

    #[test]
    fn visualization_view_cycle_safe() {
        // A→B→C→A cycle
        let mut g = ImportGraph::new();
        g.set_edges(
            "/a.rs",
            Lang::Rust,
            vec![Edge { target: EdgeTarget::File("/b.rs".into()), kind: EdgeKind::Import }],
        );
        g.set_edges(
            "/b.rs",
            Lang::Rust,
            vec![Edge { target: EdgeTarget::File("/c.rs".into()), kind: EdgeKind::Import }],
        );
        g.set_edges(
            "/c.rs",
            Lang::Rust,
            vec![Edge { target: EdgeTarget::File("/a.rs".into()), kind: EdgeKind::Import }],
        );

        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 10,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
        };
        // Must terminate — no infinite loop
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 3);
        // Edges: A→B, B→C (C→A not included since A already visited)
        assert_eq!(result.edges.len(), 2);
    }

    #[test]
    fn visualization_view_caps_truncation() {
        let mut g = ImportGraph::new();
        g.set_edges("/a.rs", Lang::Rust, vec![Edge { target: EdgeTarget::File("/b.rs".into()), kind: EdgeKind::Import }]);
        g.set_edges("/b.rs", Lang::Rust, vec![Edge { target: EdgeTarget::File("/c.rs".into()), kind: EdgeKind::Import }]);
        g.set_edges("/c.rs", Lang::Rust, vec![Edge { target: EdgeTarget::File("/d.rs".into()), kind: EdgeKind::Import }]);
        g.set_edges("/d.rs", Lang::Rust, vec![Edge { target: EdgeTarget::File("/e.rs".into()), kind: EdgeKind::Import }]);
        g.set_edges("/e.rs", Lang::Rust, vec![]);

        assert_eq!(g.file_count, 5);
        assert_eq!(g.nodes.len(), 5);

        // depth=3 clamps to 3, so BFS finds 4 nodes (A,B,C,D — D is at depth 3, stops traversal)
        // Cap to 2 nodes / 1 edge to trigger truncation
        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependencies,
            max_nodes: 2,
            max_edges: 1,
        };
        let result = g.visualization_view(&req);
        assert!(result.total_nodes_available >= 4, "total={}", result.total_nodes_available);
        assert!(result.nodes.len() <= 2);
        assert!(result.edges.len() <= 1);
        assert!(result.nodes_truncated);
        assert!(result.edges_truncated);
    }

    #[test]
    fn visualization_view_missing_path() {
        let g = chain_graph();
        let req = VisualizationRequest {
            path: Some("/nonexistent.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
        };
        let result = g.visualization_view(&req);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert_eq!(result.total_nodes_available, 0);
    }

    #[test]
    fn visualization_view_stable_ordering() {
        let g = chain_graph();
        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
        };
        let r1 = g.visualization_view(&req);
        let r2 = g.visualization_view(&req);
        // Same input → same output
        assert_eq!(r1.nodes.len(), r2.nodes.len());
        for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.role, b.role);
            assert_eq!(a.depth_from_focus, b.depth_from_focus);
        }
        assert_eq!(r1.edges.len(), r2.edges.len());
        for (a, b) in r1.edges.iter().zip(r2.edges.iter()) {
            assert_eq!(a.from, b.from);
            assert_eq!(a.to, b.to);
        }
    }
}
