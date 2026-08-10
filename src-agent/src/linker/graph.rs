//! In-memory directed import graph.
//!
//! Nodes are canonical file paths; edges represent import/module relationships
//! extracted by tree-sitter. A reverse index enables fast dependents/impact
//! queries without traversal.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ipc::linker_proto::{
    GraphDirection, GraphNodeRole, GraphViewEdge, GraphViewNode, GraphViewResult, LanguageCount,
    VisualizationRequest, WorkspaceRootInfo,
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
    /// Registered workspace roots (sorted, canonical absolute paths).
    pub workspace_roots: Vec<String>,
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
        self.workspace_roots.clear();
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

    /// Resolve the workspace root for a given file path using longest-prefix
    /// matching with `Path::starts_with` (avoids `/foo/bar` matching
    /// `/foo/barista` via naive string prefix).
    pub fn resolve_root(&self, path: &str) -> Option<&str> {
        let p = std::path::Path::new(path);
        let mut best: Option<(&str, usize)> = None;
        for root in &self.workspace_roots {
            let root_path = std::path::Path::new(root);
            if p.starts_with(root_path) {
                let len = root.len();
                match best {
                    Some((_, best_len)) if best_len >= len => {}
                    _ => best = Some((root.as_str(), len)),
                }
            }
        }
        best.map(|(r, _)| r)
    }

    /// Build workspace info: per-root file count and language breakdown.
    /// Roots with zero files are included if they are registered.
    /// Results are sorted deterministically by root path.
    pub fn workspace_info(&self) -> Vec<WorkspaceRootInfo> {
        // Count files per root per language.
        let mut root_files: HashMap<&str, HashMap<String, usize>> = HashMap::new();
        for (path, node) in &self.nodes {
            if let Some(root) = self.resolve_root(path) {
                let lang = format!("{:?}", node.lang);
                *root_files.entry(root).or_default().entry(lang).or_insert(0) += 1;
            }
        }

        let mut result: Vec<WorkspaceRootInfo> = self
            .workspace_roots
            .iter()
            .map(|root| {
                let langs = root_files.get(root.as_str()).cloned().unwrap_or_default();
                let file_count: usize = langs.values().sum();
                let mut lang_counts: Vec<LanguageCount> = langs
                    .into_iter()
                    .map(|(name, count)| LanguageCount { name, count })
                    .collect();
                lang_counts.sort_by(|a, b| a.name.cmp(&b.name));
                WorkspaceRootInfo {
                    root: root.clone(),
                    file_count,
                    languages: lang_counts,
                }
            })
            .collect();
        result.sort_by(|a, b| a.root.cmp(&b.root));
        result
    }

    /// Check if a node path passes the active filters.
    fn passes_filters(
        &self,
        path: &str,
        filter_roots: &Option<Vec<String>>,
        filter_languages: &Option<Vec<String>>,
    ) -> bool {
        // Root filter: if filter_roots is non-empty, path must be under one of them.
        if let Some(roots) = filter_roots {
            if !roots.is_empty() {
                let p = std::path::Path::new(path);
                let dominated = roots.iter().any(|r| {
                    let rp = std::path::Path::new(r);
                    p.starts_with(rp)
                });
                if !dominated {
                    return false;
                }
            }
        }
        // Language filter: if filter_languages is non-empty, language must be in the set.
        if let Some(langs) = filter_languages {
            if !langs.is_empty() {
                if let Some(node) = self.nodes.get(path) {
                    let lang = format!("{:?}", node.lang);
                    if !langs.contains(&lang) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        true
    }

    /// Build a bounded subgraph view for GUI visualization.
    ///
    /// If `req.path` is `None`, returns overview metadata with top fan-in and entry-point
    /// nodes. If `req.path` is `Some`, BFS from the focal node in the requested direction(s).
    /// Filters are applied before traversal: filtered-out nodes are neither discovered
    /// nor traversed through.
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
        let filter_roots = req.filter_roots.clone();
        let filter_languages = req.filter_languages.clone();

        // Build available_roots from the full graph (unfiltered, for filter pickers).
        let available_roots = self.workspace_info();

        // Determine whether filters are active (non-None, non-empty).
        let filters_active = matches!(&filter_roots, Some(r) if !r.is_empty())
            || matches!(&filter_languages, Some(l) if !l.is_empty());

        // Compute filtered aggregate counts for metadata.
        let (filtered_file_count, filtered_edge_count, filtered_languages) = if filters_active {
            let file_count = self
                .nodes
                .keys()
                .filter(|p| self.passes_filters(p, &filter_roots, &filter_languages))
                .count();
            // Edge count: resolved file edges where BOTH source and target pass filters.
            let edge_count: usize = self
                .edges
                .iter()
                .filter(|(src, _)| self.passes_filters(src, &filter_roots, &filter_languages))
                .map(|(_, edges)| {
                    edges
                        .iter()
                        .filter(|e| {
                            if let EdgeTarget::File(target) = &e.target {
                                self.passes_filters(target, &filter_roots, &filter_languages)
                            } else {
                                false
                            }
                        })
                        .count()
                })
                .sum();
            // Languages: only from filtered nodes, sorted.
            let mut langs: Vec<String> = self
                .nodes
                .iter()
                .filter(|(p, _)| self.passes_filters(p, &filter_roots, &filter_languages))
                .map(|(_, n)| format!("{:?}", n.lang))
                .collect();
            langs.sort();
            langs.dedup();
            (file_count, edge_count, langs)
        } else {
            (
                self.file_count,
                self.edge_count,
                self.languages(),
            )
        };

        let base = GraphViewResult {
            nodes: Vec::new(),
            edges: Vec::new(),
            focus: None,
            generation: self.generation,
            file_count: filtered_file_count,
            edge_count: filtered_edge_count,
            languages: filtered_languages,
            nodes_truncated: false,
            edges_truncated: false,
            total_nodes_available: 0,
            total_edges_available: 0,
            available_roots,
        };

        match &req.path {
            None => self.overview_view(base, max_nodes, &filter_roots, &filter_languages),
            Some(path) => match self.resolve_key(path) {
                None => GraphViewResult {
                    nodes_truncated: true,
                    edges_truncated: true,
                    total_nodes_available: 0,
                    total_edges_available: 0,
                    ..base
                },
                Some(key) => {
                    // If the focus itself is excluded by filters, return empty view.
                    if !self.passes_filters(key, &filter_roots, &filter_languages) {
                        return GraphViewResult { ..base };
                    }
                    let key = key.to_string();
                    self.focus_view(
                        base,
                        &key,
                        &req.direction,
                        depth,
                        max_nodes,
                        max_edges,
                        &filter_roots,
                        &filter_languages,
                    )
                }
            },
        }
    }

    /// Build an overview (no focal file): top fan-in + entry points, all Overview role.
    /// Only includes nodes passing the active filters.
    /// Ranks candidates after filtering across the full graph so that nodes with
    /// high filtered fan-in are not lost behind unfiltered global top-N truncation.
    fn overview_view(
        &self,
        base: GraphViewResult,
        max_nodes: usize,
        filter_roots: &Option<Vec<String>>,
        filter_languages: &Option<Vec<String>>,
    ) -> GraphViewResult {
        // Step 1: collect ALL paths that pass filters.
        let all_filtered: Vec<&str> = self
            .nodes
            .keys()
            .filter(|p| self.passes_filters(p, filter_roots, filter_languages))
            .map(|p| p.as_str())
            .collect();

        // Step 2: rank filtered nodes by fan-in (count only allowed incoming resolved file edges).
        let mut fan_in_counts: Vec<(&str, usize)> = all_filtered
            .iter()
            .map(|p| {
                let count = self
                    .reverse
                    .get(*p)
                    .map(|sources| {
                        sources
                            .iter()
                            .filter(|src| {
                                self.passes_filters(src, filter_roots, filter_languages)
                                    && self.edges.get(src.as_str()).is_some_and(|edges| {
                                        edges.iter().any(|e| {
                                            matches!(&e.target, EdgeTarget::File(f) if f == *p)
                                        })
                                    })
                            })
                            .count()
                    })
                    .unwrap_or(0);
                (*p, count)
            })
            .collect();
        fan_in_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        // Step 3: collect entry points from filtered nodes (zero allowed incoming resolved file edges).
        let mut entry_points: Vec<&str> = all_filtered
            .iter()
            .filter(|p| {
                self.reverse
                    .get(**p)
                    .map(|sources| {
                        !sources.iter().any(|src| {
                            self.passes_filters(src, filter_roots, filter_languages)
                                && self.edges.get(src.as_str()).is_some_and(|edges| {
                                    edges.iter().any(|e| {
                                        matches!(&e.target, EdgeTarget::File(f) if f == **p)
                                    })
                                })
                        })
                    })
                    .unwrap_or(true)
            })
            .copied()
            .collect();
        entry_points.sort();

        // Step 4: merge — top fan-in first, then entry points, dedup, truncate.
        let mut paths: Vec<String> = Vec::new();
        for (path, _count) in &fan_in_counts {
            paths.push(path.to_string());
        }
        for ep in &entry_points {
            if !paths.iter().any(|p| p == *ep) {
                paths.push(ep.to_string());
            }
        }
        paths.dedup();

        let total_nodes_available = paths.len();
        let nodes_truncated = total_nodes_available > max_nodes;
        paths.truncate(max_nodes);

        let nodes: Vec<GraphViewNode> = paths
            .iter()
            .map(|p| {
                let lang = self
                    .nodes
                    .get(p)
                    .map(|n| format!("{:?}", n.lang))
                    .unwrap_or_default();
                // Count only allowed resolved edges for overview node degrees.
                let out_degree = self
                    .edges
                    .get(p)
                    .map(|es| {
                        es.iter()
                            .filter(|e| {
                                matches!(&e.target, EdgeTarget::File(f) if self.passes_filters(f, filter_roots, filter_languages))
                            })
                            .count()
                    })
                    .unwrap_or(0);
                let in_degree = self
                    .reverse
                    .get(p)
                    .map(|sources| {
                        sources
                            .iter()
                            .filter(|src| {
                                self.passes_filters(src, filter_roots, filter_languages)
                                    && self.edges.get(src.as_str()).is_some_and(|edges| {
                                        edges.iter().any(|e| {
                                            matches!(&e.target, EdgeTarget::File(f) if f == p)
                                        })
                                    })
                            })
                            .count()
                    })
                    .unwrap_or(0);
                GraphViewNode {
                    path: p.clone(),
                    language: lang,
                    out_degree,
                    in_degree,
                    role: GraphNodeRole::Overview,
                    depth_from_focus: None,
                    workspace_root: self.resolve_root(p).map(String::from),
                }
            })
            .collect();

        GraphViewResult {
            nodes,
            edges: Vec::new(),
            focus: None,
            nodes_truncated,
            total_nodes_available,
            total_edges_available: 0,
            ..base
        }
    }

    /// Build a focus view: BFS from `key` in the requested direction(s).
    /// Filters are applied: filtered-out nodes are not discovered or traversed through.
    fn focus_view(
        &self,
        base: GraphViewResult,
        key: &str,
        direction: &GraphDirection,
        depth: u32,
        max_nodes: usize,
        max_edges: usize,
        filter_roots: &Option<Vec<String>>,
        filter_languages: &Option<Vec<String>>,
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

        // Focus node must already pass filters (checked by caller).
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
                            // Skip filtered-out nodes entirely (no traversal through).
                            if !self.passes_filters(target, filter_roots, filter_languages) {
                                continue;
                            }
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
            if matches!(direction, GraphDirection::Dependents | GraphDirection::Both) {
                if let Some(incoming) = self.reverse.get(&current) {
                    for source in incoming {
                        // Skip filtered-out nodes entirely.
                        if !self.passes_filters(source, filter_roots, filter_languages) {
                            continue;
                        }
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
                    workspace_root: self.resolve_root(&p).map(String::from),
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

    use crate::ipc::linker_proto::{GraphDirection, GraphNodeRole, VisualizationRequest};

    /// Helper: build a linear chain A→B→C.
    fn chain_graph() -> ImportGraph {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
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

    fn make_req(path: Option<&str>, depth: u32, dir: GraphDirection) -> VisualizationRequest {
        VisualizationRequest {
            path: path.map(String::from),
            depth,
            direction: dir,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: None,
            filter_languages: None,
        }
    }

    #[test]
    fn visualization_view_overview_returns_metadata() {
        let g = chain_graph();
        let req = make_req(None, 1, GraphDirection::Both);
        let result = g.visualization_view(&req);
        assert!(result.focus.is_none());
        assert!(!result.nodes.is_empty());
        for node in &result.nodes {
            assert_eq!(node.role, GraphNodeRole::Overview);
            assert!(node.depth_from_focus.is_none());
        }
        assert!(result.edges.is_empty());
        assert_eq!(result.file_count, 3);
        assert!(!result.available_roots.is_empty());
    }

    #[test]
    fn visualization_view_focus_dependencies() {
        let g = chain_graph();
        let req = make_req(Some("/a.rs"), 3, GraphDirection::Dependencies);
        let result = g.visualization_view(&req);
        assert_eq!(result.focus.as_deref(), Some("/a.rs"));
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        let focus = result
            .nodes
            .iter()
            .find(|n| n.role == GraphNodeRole::Focus)
            .unwrap();
        assert_eq!(focus.path, "/a.rs");
        assert_eq!(focus.depth_from_focus, Some(0));
        assert_eq!(focus.workspace_root.as_deref(), Some("/"));
        let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
        assert_eq!(b.role, GraphNodeRole::Dependency);
        assert_eq!(b.depth_from_focus, Some(1));
        let c = result.nodes.iter().find(|n| n.path == "/c.rs").unwrap();
        assert_eq!(c.role, GraphNodeRole::Dependency);
        assert_eq!(c.depth_from_focus, Some(2));
    }

    #[test]
    fn visualization_view_focus_dependents() {
        let g = chain_graph();
        let req = make_req(Some("/c.rs"), 3, GraphDirection::Dependents);
        let result = g.visualization_view(&req);
        assert_eq!(result.focus.as_deref(), Some("/c.rs"));
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        let focus = result
            .nodes
            .iter()
            .find(|n| n.role == GraphNodeRole::Focus)
            .unwrap();
        assert_eq!(focus.depth_from_focus, Some(0));
        let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
        assert_eq!(b.role, GraphNodeRole::Dependent);
        assert_eq!(b.depth_from_focus, Some(1));
        let a = result.nodes.iter().find(|n| n.path == "/a.rs").unwrap();
        assert_eq!(a.role, GraphNodeRole::Dependent);
        assert_eq!(a.depth_from_focus, Some(2));
    }

    #[test]
    fn visualization_view_focus_both() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
        g.set_edges(
            "/a.rs",
            Lang::Rust,
            vec![
                Edge {
                    target: EdgeTarget::File("/b.rs".into()),
                    kind: EdgeKind::Import,
                },
                Edge {
                    target: EdgeTarget::File("/c.rs".into()),
                    kind: EdgeKind::Import,
                },
            ],
        );
        g.set_edges(
            "/b.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/d.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "/c.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/d.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/d.rs", Lang::Rust, vec![]);

        let req = make_req(Some("/a.rs"), 3, GraphDirection::Both);
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 4);
        assert_eq!(result.edges.len(), 3);
        for node in &result.nodes {
            if node.path != "/a.rs" {
                assert_ne!(node.role, GraphNodeRole::Dependent);
            }
        }
    }

    #[test]
    fn visualization_view_bounded_depth() {
        let g = chain_graph();
        let req1 = make_req(Some("/a.rs"), 1, GraphDirection::Dependencies);
        let r1 = g.visualization_view(&req1);
        assert_eq!(r1.nodes.len(), 2);
        assert_eq!(r1.edges.len(), 1);

        let req2 = make_req(Some("/a.rs"), 2, GraphDirection::Dependencies);
        let r2 = g.visualization_view(&req2);
        assert_eq!(r2.nodes.len(), 3);
        assert_eq!(r2.edges.len(), 2);
    }

    #[test]
    fn visualization_view_cycle_safe() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
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
        g.set_edges(
            "/c.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/a.rs".into()),
                kind: EdgeKind::Import,
            }],
        );

        let req = make_req(Some("/a.rs"), 10, GraphDirection::Dependencies);
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
    }

    #[test]
    fn visualization_view_caps_truncation() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
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
        g.set_edges(
            "/c.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/d.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "/d.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/e.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/e.rs", Lang::Rust, vec![]);

        assert_eq!(g.file_count, 5);
        assert_eq!(g.nodes.len(), 5);

        let req = VisualizationRequest {
            path: Some("/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependencies,
            max_nodes: 2,
            max_edges: 1,
            filter_roots: None,
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert!(
            result.total_nodes_available >= 4,
            "total={}",
            result.total_nodes_available
        );
        assert!(result.nodes.len() <= 2);
        assert!(result.edges.len() <= 1);
        assert!(result.nodes_truncated);
        assert!(result.edges_truncated);
    }

    #[test]
    fn visualization_view_missing_path() {
        let g = chain_graph();
        let req = make_req(Some("/nonexistent.rs"), 3, GraphDirection::Both);
        let result = g.visualization_view(&req);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert_eq!(result.total_nodes_available, 0);
    }

    #[test]
    fn visualization_view_stable_ordering() {
        let g = chain_graph();
        let req = make_req(Some("/a.rs"), 3, GraphDirection::Dependencies);
        let r1 = g.visualization_view(&req);
        let r2 = g.visualization_view(&req);
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

    // ─── NEW TESTS: root resolution, workspace info, filtering ────────────

    #[test]
    fn resolve_root_longest_match() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/project".to_string(), "/project/sub".to_string()];
        assert_eq!(
            g.resolve_root("/project/sub/src/main.rs"),
            Some("/project/sub")
        );
        assert_eq!(g.resolve_root("/project/lib/util.rs"), Some("/project"));
        assert_eq!(g.resolve_root("/other/file.rs"), None);
    }

    #[test]
    fn resolve_root_no_false_prefix() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/foo/bar".to_string()];
        assert_eq!(g.resolve_root("/foo/barista/file.rs"), None);
        assert_eq!(g.resolve_root("/foo/bar/file.rs"), Some("/foo/bar"));
    }

    #[test]
    fn workspace_info_deterministic() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/root_b".to_string(), "/root_a".to_string()];
        g.ensure_node("/root_a/src.rs", Lang::Rust);
        g.ensure_node("/root_a/lib.ts", Lang::TypeScript);
        g.ensure_node("/root_b/main.rs", Lang::Rust);
        g.file_count = g.nodes.len();

        let info = g.workspace_info();
        assert_eq!(info.len(), 2);
        assert_eq!(info[0].root, "/root_a");
        assert_eq!(info[0].file_count, 2);
        assert_eq!(info[1].root, "/root_b");
        assert_eq!(info[1].file_count, 1);
        let ra_langs: Vec<_> = info[0]
            .languages
            .iter()
            .map(|l| (l.name.as_str(), l.count))
            .collect();
        assert_eq!(ra_langs, vec![("Rust", 1usize), ("TypeScript", 1usize)]);
    }

    #[test]
    fn workspace_info_includes_zero_file_roots() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/empty".to_string(), "/has_files".to_string()];
        g.ensure_node("/has_files/main.rs", Lang::Rust);
        g.file_count = g.nodes.len();

        let info = g.workspace_info();
        assert_eq!(info.len(), 2);
        let empty_root = info.iter().find(|r| r.root == "/empty").unwrap();
        assert_eq!(empty_root.file_count, 0);
        assert!(empty_root.languages.is_empty());
    }

    #[test]
    fn filter_by_root_focus_view() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_b/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_b/b.rs", Lang::Rust, vec![]);
        g.set_edges(
            "/ws_b/c.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_a/d.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_a/d.rs", Lang::Rust, vec![]);
        g.file_count = g.nodes.len();

        let req = VisualizationRequest {
            path: Some("/ws_a/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].path, "/ws_a/a.rs");
        assert!(result.edges.is_empty());
    }

    #[test]
    fn filter_by_language_focus_view() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
        g.set_edges(
            "/main.rs",
            Lang::Rust,
            vec![
                Edge {
                    target: EdgeTarget::File("/util.rs".into()),
                    kind: EdgeKind::Import,
                },
                Edge {
                    target: EdgeTarget::File("/app.ts".into()),
                    kind: EdgeKind::Import,
                },
            ],
        );
        g.set_edges("/util.rs", Lang::Rust, vec![]);
        g.set_edges("/app.ts", Lang::TypeScript, vec![]);
        g.file_count = g.nodes.len();

        let req = VisualizationRequest {
            path: Some("/main.rs".to_string()),
            depth: 2,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: None,
            filter_languages: Some(vec!["Rust".to_string()]),
        };
        let result = g.visualization_view(&req);
        let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
        assert!(paths.contains(&"/main.rs"));
        assert!(paths.contains(&"/util.rs"));
        assert!(!paths.contains(&"/app.ts"));
        assert_eq!(result.edges.len(), 1);
    }

    #[test]
    fn filter_excluded_focus_returns_empty() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_b/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_b/b.rs", Lang::Rust, vec![]);
        g.file_count = g.nodes.len();

        let req = VisualizationRequest {
            path: Some("/ws_b/b.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn filter_no_traversal_through_filtered_node() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_b/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "/ws_b/b.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_a/c.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_a/c.rs", Lang::Rust, vec![]);
        g.file_count = g.nodes.len();

        let req = VisualizationRequest {
            path: Some("/ws_a/a.rs".to_string()),
            depth: 3,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].path, "/ws_a/a.rs");
        assert!(result.edges.is_empty());
    }

    #[test]
    fn filter_overview_view() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_b/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_b/b.rs", Lang::Rust, vec![]);
        g.file_count = g.nodes.len();

        let req = VisualizationRequest {
            path: None,
            depth: 1,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].path, "/ws_a/a.rs");
    }

    #[test]
    fn filter_combined_root_and_language() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![
                Edge {
                    target: EdgeTarget::File("/ws_a/b.ts".into()),
                    kind: EdgeKind::Import,
                },
                Edge {
                    target: EdgeTarget::File("/ws_a/c.rs".into()),
                    kind: EdgeKind::Import,
                },
            ],
        );
        g.set_edges("/ws_a/b.ts", Lang::TypeScript, vec![]);
        g.set_edges("/ws_a/c.rs", Lang::Rust, vec![]);
        g.file_count = g.nodes.len();

        // Filter to ws_a + Rust only: b.ts excluded, c.rs passes.
        let req = VisualizationRequest {
            path: Some("/ws_a/a.rs".to_string()),
            depth: 2,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: Some(vec!["Rust".to_string()]),
        };
        let result = g.visualization_view(&req);
        let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
        assert!(paths.contains(&"/ws_a/a.rs"));
        assert!(paths.contains(&"/ws_a/c.rs"));
        assert!(!paths.contains(&"/ws_a/b.ts"));
        assert_eq!(result.edges.len(), 1); // only a→c (a→b filtered)
    }

    #[test]
    fn available_roots_always_present() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/root_a".to_string(), "/root_b".to_string()];
        g.ensure_node("/root_a/f.rs", Lang::Rust);
        g.file_count = g.nodes.len();

        let req = make_req(Some("/root_a/f.rs"), 1, GraphDirection::Both);
        let result = g.visualization_view(&req);
        assert_eq!(result.available_roots.len(), 2);
        assert_eq!(result.available_roots[0].root, "/root_a");
        assert_eq!(result.available_roots[0].file_count, 1);
        assert_eq!(result.available_roots[1].root, "/root_b");
        assert_eq!(result.available_roots[1].file_count, 0);
    }

    #[test]
    fn serde_backward_compat() {
        // Old request without filter_roots/filter_languages should deserialize fine.
        let json = r#"{"path":null,"depth":2,"direction":"both","max_nodes":100,"max_edges":100}"#;
        let req: VisualizationRequest = serde_json::from_str(json).unwrap();
        assert!(req.filter_roots.is_none());
        assert!(req.filter_languages.is_none());

        // Old node without workspace_root should deserialize fine.
        let json = r#"{"path":"/a.rs","language":"Rust","out_degree":1,"in_degree":0,"role":"Focus","depth_from_focus":0}"#;
        let node: crate::ipc::linker_proto::GraphViewNode = serde_json::from_str(json).unwrap();
        assert!(node.workspace_root.is_none());
    }

    // ─── Filtered metadata and overview correctness ────────────────────────

    #[test]
    fn filtered_metadata_edge_count_consistent() {
        // a → b (same root), a → c (cross-root). Filter to ws_a only:
        // edge_count should be 1 (a→b), not 2 (the global count).
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![
                Edge {
                    target: EdgeTarget::File("/ws_a/b.rs".into()),
                    kind: EdgeKind::Import,
                },
                Edge {
                    target: EdgeTarget::File("/ws_b/c.rs".into()),
                    kind: EdgeKind::Import,
                },
            ],
        );
        g.set_edges("/ws_a/b.rs", Lang::Rust, vec![]);
        g.set_edges("/ws_b/c.rs", Lang::Rust, vec![]);

        let req = VisualizationRequest {
            path: None,
            depth: 1,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.file_count, 2, "file_count should be 2 (a.rs + b.rs under ws_a)");
        assert_eq!(result.edge_count, 1, "edge_count should be 1 (only a→b, both endpoints in ws_a)");
    }

    #[test]
    fn filtered_metadata_languages_exclude_filtered_out() {
        // Rust + TypeScript. Filter to Rust only: languages should be ["Rust"].
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
        g.set_edges(
            "/main.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/app.ts".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/app.ts", Lang::TypeScript, vec![]);

        let req = VisualizationRequest {
            path: Some("/main.rs".into()),
            depth: 2,
            direction: GraphDirection::Dependencies,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: None,
            filter_languages: Some(vec!["Rust".to_string()]),
        };
        let result = g.visualization_view(&req);
        assert_eq!(result.languages, vec!["Rust"]);
    }

    #[test]
    fn filtered_metadata_no_filters_uses_global() {
        // Without filters, metadata should reflect the full graph.
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
        g.set_edges(
            "/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/b.rs", Lang::Rust, vec![]);

        let req = make_req(Some("/a.rs"), 2, GraphDirection::Dependencies);
        let result = g.visualization_view(&req);
        assert_eq!(result.file_count, 2);
        assert_eq!(result.edge_count, 1);
    }

    #[test]
    fn overview_filtered_total_before_truncation() {
        // Build 5 nodes in ws_a, filter to ws_a, use max_nodes=2.
        // total_nodes_available should be 5 (before truncation), nodes.len() <= 2.
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string()];
        for i in 0..5 {
            g.set_edges(
                &format!("/ws_a/f{i}.rs"),
                Lang::Rust,
                vec![Edge {
                    target: EdgeTarget::File(format!("/ws_a/f{}.rs", (i + 1) % 5)),
                    kind: EdgeKind::Import,
                }],
            );
        }

        let req = VisualizationRequest {
            path: None,
            depth: 1,
            direction: GraphDirection::Both,
            max_nodes: 2,
            max_edges: 10,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        assert!(
            result.total_nodes_available >= 5,
            "total_nodes_available should be 5 before truncation, got {}",
            result.total_nodes_available
        );
        assert!(result.nodes.len() <= 2);
        assert!(result.nodes_truncated);
    }

    #[test]
    fn overview_filtered_degrees_only_allowed_edges() {
        // a→b (both in ws_a), c→b (in ws_b). Filter to ws_a.
        // b's in_degree in overview should be 1 (from a), not 2 (global).
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/ws_a".to_string(), "/ws_b".to_string()];
        g.set_edges(
            "/ws_a/a.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_a/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges(
            "/ws_b/c.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/ws_a/b.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/ws_a/b.rs", Lang::Rust, vec![]);

        let req = VisualizationRequest {
            path: None,
            depth: 1,
            direction: GraphDirection::Both,
            max_nodes: 100,
            max_edges: 100,
            filter_roots: Some(vec!["/ws_a".to_string()]),
            filter_languages: None,
        };
        let result = g.visualization_view(&req);
        let b = result.nodes.iter().find(|n| n.path == "/ws_a/b.rs");
        assert!(b.is_some(), "b.rs should be in filtered overview");
        assert_eq!(b.unwrap().in_degree, 1, "b's in_degree should count only ws_a edges");
        let a = result.nodes.iter().find(|n| n.path == "/ws_a/a.rs");
        assert!(a.is_some());
        assert_eq!(a.unwrap().out_degree, 1);
    }

    #[test]
    fn overview_stable_deterministic_ordering() {
        let mut g = ImportGraph::new();
        g.workspace_roots = vec!["/".to_string()];
        g.set_edges(
            "/x.rs",
            Lang::Rust,
            vec![Edge {
                target: EdgeTarget::File("/y.rs".into()),
                kind: EdgeKind::Import,
            }],
        );
        g.set_edges("/y.rs", Lang::Rust, vec![]);

        let req = make_req(None, 1, GraphDirection::Both);
        let r1 = g.visualization_view(&req);
        let r2 = g.visualization_view(&req);
        assert_eq!(r1.nodes.len(), r2.nodes.len());
        for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.in_degree, b.in_degree);
            assert_eq!(a.out_degree, b.out_degree);
        }
    }
}
