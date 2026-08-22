//! In-memory directed import graph.
//!
//! Nodes are canonical file paths; edges represent import/module relationships
//! extracted by tree-sitter. A reverse index enables fast dependents/impact
//! queries without traversal.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ipc::linker_proto::{
    EditContextResult, GraphDirection, GraphNodeRole, GraphViewEdge, GraphViewNode,
    GraphViewResult, LanguageCount, VisualizationRequest, WorkspaceRootInfo,
};
use crate::linker::reference::{ImportKind, SourceRefs};

/// Shared caps + filter knobs for focused graph views.
struct ViewCaps<'a> {
    max_nodes: usize,
    max_edges: usize,
    filter_roots: &'a Option<Vec<String>>,
    filter_languages: &'a Option<Vec<String>>,
}

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Edge {
    pub target: EdgeTarget,
    /// Reserved for future mod-edge discrimination; always `Import` today.
    #[allow(dead_code)]
    pub kind: EdgeKind,
}

impl Edge {
    fn is_resolved(&self) -> bool {
        matches!(self.target, EdgeTarget::File(_))
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// `use`, `import`, `require`, etc.
    Import,
    /// `mod` declaration (Rust-specific).
    #[allow(dead_code)]
    Mod,
    /// Structured import carrying semantic kind and optional condition.
    /// Added in phase 1 (Full-Fidelity Multi-Language Import Graph).
    #[allow(dead_code)]
    Structured {
        /// Semantic import kind.
        import_kind: ImportKind,
        /// Optional compilation condition (e.g. `cfg(test)`).
        condition: Option<String>,
    },
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
    /// Total resolved edges tracked.
    pub edge_count: usize,
    /// Monotonically increasing scan generation counter.
    pub generation: u64,
    /// Registered workspace roots (sorted, canonical absolute paths).
    pub workspace_roots: Vec<String>,
    /// Per-source-file structured import references (phase 1 foundation).
    /// Stores ImportRef + Resolution pairs separately from graph edges so
    /// that traversal remains resolved-file-only while full import info is
    /// preserved for later phases.
    pub source_refs: HashMap<String, SourceRefs>,
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
        self.source_refs.clear();
        self.generation += 1;
    }

    /// Ensure a node exists for the given path and language.
    pub fn ensure_node(&mut self, path: &str, lang: Lang) {
        self.nodes.entry(path.to_string()).or_insert_with(|| Node {
            lang,
            path: path.to_string(),
        });
    }

    /// Replace all edges for a source file. Edges are stably deduplicated by
    /// `(target, kind)`. Only resolved file edges contribute to `edge_count`
    /// and the reverse traversal index.
    pub fn set_edges(&mut self, source: &str, lang: Lang, new_edges: Vec<Edge>) {
        self.ensure_node(source, lang);

        if let Some(old_edges) = self.edges.remove(source) {
            let old_targets: HashSet<&str> = old_edges
                .iter()
                .filter_map(|edge| match &edge.target {
                    EdgeTarget::File(target) => Some(target.as_str()),
                    EdgeTarget::External(_) => None,
                })
                .collect();
            for target in old_targets {
                if let Some(sources) = self.reverse.get_mut(target) {
                    sources.retain(|existing| existing != source);
                    if sources.is_empty() {
                        self.reverse.remove(target);
                    }
                }
            }
            self.edge_count = self
                .edge_count
                .saturating_sub(old_edges.iter().filter(|edge| edge.is_resolved()).count());
        }

        let mut seen = HashSet::new();
        let mut edges = Vec::with_capacity(new_edges.len());
        for edge in new_edges {
            if seen.insert((edge.target.clone(), edge.kind.clone())) {
                edges.push(edge);
            }
        }

        let mut targets = HashSet::new();
        for edge in &edges {
            if let EdgeTarget::File(target) = &edge.target {
                self.edge_count += 1;
                if targets.insert(target.as_str()) {
                    let sources = self.reverse.entry(target.clone()).or_default();
                    if !sources.iter().any(|existing| existing == source) {
                        sources.push(source.to_string());
                    }
                }
            }
        }
        self.edges.insert(source.to_string(), edges);
        self.file_count = self.nodes.len();
    }

    /// Remove a node, its structured refs, and all incoming/outgoing resolved
    /// edges while preserving external diagnostics on other sources.
    pub fn remove_node(&mut self, path: &str) {
        if let Some(outgoing) = self.edges.remove(path) {
            let targets: HashSet<&str> = outgoing
                .iter()
                .filter_map(|edge| match &edge.target {
                    EdgeTarget::File(target) => Some(target.as_str()),
                    EdgeTarget::External(_) => None,
                })
                .collect();
            for target in targets {
                if let Some(sources) = self.reverse.get_mut(target) {
                    sources.retain(|source| source != path);
                    if sources.is_empty() {
                        self.reverse.remove(target);
                    }
                }
            }
            self.edge_count = self
                .edge_count
                .saturating_sub(outgoing.iter().filter(|edge| edge.is_resolved()).count());
        }

        if let Some(incoming) = self.reverse.remove(path) {
            for source in incoming {
                if let Some(edges) = self.edges.get_mut(&source) {
                    let removed = edges
                        .iter()
                        .filter(|edge| matches!(&edge.target, EdgeTarget::File(target) if target == path))
                        .count();
                    edges.retain(
                        |edge| !matches!(&edge.target, EdgeTarget::File(target) if target == path),
                    );
                    self.edge_count = self.edge_count.saturating_sub(removed);
                }
            }
        }

        self.source_refs.remove(path);
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

    /// Direct dependencies of a file (unique resolved target paths in edge order).
    pub fn dependencies(&self, path: &str) -> Vec<&str> {
        self.edges
            .get(path)
            .map(|es| {
                let mut seen = HashSet::new();
                es.iter()
                    .filter_map(|e| match &e.target {
                        EdgeTarget::File(f) => Some(f.as_str()),
                        EdgeTarget::External(_) => None,
                    })
                    .filter(|target| seen.insert(*target))
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

    /// Compute rich edit-context intelligence for a file in a single pass.
    ///
    /// Returns structured data for L3 footer enrichment: direct imports,
    /// dependents, transitive impact, cross-boundary deps, unresolved
    /// imports, entry-point/leaf status, and related test/config files.
    pub fn edit_context(&self, path: &str) -> EditContextResult {
        let imports: Vec<String> = self
            .dependencies(path)
            .into_iter()
            .map(String::from)
            .collect();
        let imported_by: Vec<String> = self
            .dependents(path)
            .into_iter()
            .map(String::from)
            .collect();

        let is_entry_point = imported_by.is_empty();

        // is_leaf: no outgoing file edges AND no external edges.
        let is_leaf = {
            let has_file_out = self
                .edges
                .get(path)
                .is_some_and(|es| es.iter().any(|e| matches!(&e.target, EdgeTarget::File(_))));
            let has_ext_out = self.edges.get(path).is_some_and(|es| {
                es.iter()
                    .any(|e| matches!(&e.target, EdgeTarget::External(_)))
            });
            !has_file_out && !has_ext_out
        };

        // Transitive dependents at depth 2 (excludes self).
        let transitive = self.impact(path, 2);
        let transitive_dependents_count = transitive.len().saturating_sub(1);

        // Entry-point count among transitive dependents (cap at 100 to avoid O(n²)).
        let entry_point_count = {
            let mut count = 0usize;
            for p in transitive.iter().take(101) {
                if *p == path {
                    continue;
                }
                if count >= 100 {
                    break;
                }
                if self.reverse.get(*p).is_none_or(|v| v.is_empty()) {
                    count += 1;
                }
            }
            count
        };

        // Cross-boundary deps: direct deps whose workspace root differs.
        let my_root = self.resolve_root(path);
        let cross_boundary_deps: Vec<(String, String)> = imports
            .iter()
            .filter_map(|dep| {
                let dep_root = self.resolve_root(dep);
                match (my_root, dep_root) {
                    (Some(mr), Some(dr)) if mr != dr => Some((dr.to_string(), dep.clone())),
                    _ => None,
                }
            })
            .collect();

        // External / unresolved imports.
        let unresolved_imports: Vec<String> = self
            .edges
            .get(path)
            .map(|es| {
                es.iter()
                    .filter_map(|e| match &e.target {
                        EdgeTarget::External(name) => Some(name.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Collect all dep + dependent paths for test/config detection.
        let mut all_neighbors: Vec<&str> = Vec::new();
        all_neighbors.extend(imports.iter().map(|s| s.as_str()));
        all_neighbors.extend(imported_by.iter().map(|s| s.as_str()));

        let related_tests: Vec<String> = all_neighbors
            .iter()
            .filter(|p| is_test_file(p))
            .take(3)
            .map(|s| s.to_string())
            .collect();

        let related_configs: Vec<String> = all_neighbors
            .iter()
            .filter(|p| is_config_file(p))
            .take(3)
            .map(|s| s.to_string())
            .collect();

        EditContextResult {
            imports,
            imported_by,
            transitive_dependents_count,
            entry_point_count,
            cross_boundary_deps,
            is_entry_point,
            is_leaf,
            unresolved_imports,
            related_tests,
            related_configs,
        }
    }

    /// Check if a file path looks like a test file (multi-language heuristic).
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
            (self.file_count, self.edge_count, self.languages())
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
            None => {
                // No-focus: return metadata/availableRoots only — no graph nodes/edges.
                // The GUI always focuses a file; overview grid is not used.
                // Contract: totals are ZERO (not filtered file count).
                GraphViewResult {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    focus: None,
                    total_nodes_available: 0,
                    total_edges_available: 0,
                    nodes_truncated: false,
                    edges_truncated: false,
                    ..base
                }
            }
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
                    // For depth==1, use the dedicated direct-star builder
                    // (strict immediate neighborhood, no neighbor-neighbor leakage).
                    let caps = ViewCaps {
                        max_nodes,
                        max_edges,
                        filter_roots: &filter_roots,
                        filter_languages: &filter_languages,
                    };
                    if depth == 1 {
                        self.direct_star_view(base, &key, &req.direction, caps)
                    } else {
                        self.focus_view(base, &key, &req.direction, depth, caps)
                    }
                }
            },
        }
    }

    /// Build an overview (no focal file): top fan-in + entry points, all Overview role.
    /// Only includes nodes passing the active filters.
    /// Ranks candidates after filtering across the full graph so that nodes with
    /// high filtered fan-in are not lost behind unfiltered global top-N truncation.
    ///
    /// Note: the current GUI always focuses a file; this is retained for
    /// test coverage and potential future use.
    #[allow(dead_code)]
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
                                        edges.iter().any(
                                            |e| matches!(&e.target, EdgeTarget::File(f) if f == *p),
                                        )
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
                                    edges.iter().any(
                                        |e| matches!(&e.target, EdgeTarget::File(f) if f == **p),
                                    )
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

    /// Build a strict one-hop direct-star view for GUI visualization.
    ///
    /// Node set: focus + filter-passing direct outgoing targets + filter-passing
    /// direct incoming sources. Dedup canonical paths; focus first under cap.
    ///
    /// Edge set: every actual directed edge touching focus among retained nodes.
    /// Preserve reciprocal pairs (focus->X and X->focus). Exclude
    /// neighbor-neighbor edges. Sort+dedup pairs.
    ///
    /// `total_nodes_available` and `total_edges_available` are pre-cap
    /// direct-star totals; truncation flags are accurate; returned edges have
    /// retained endpoints only; in/out degree from final returned edge set.
    fn direct_star_view(
        &self,
        base: GraphViewResult,
        key: &str,
        direction: &GraphDirection,
        caps: ViewCaps<'_>,
    ) -> GraphViewResult {
        let ViewCaps {
            max_nodes,
            max_edges,
            filter_roots,
            filter_languages,
        } = caps;
        // Collect direct outgoing targets (dependencies) that pass filters.
        let mut outgoing: Vec<String> = Vec::new();
        if matches!(
            direction,
            GraphDirection::Dependencies | GraphDirection::Both
        ) {
            if let Some(edges) = self.edges.get(key) {
                for edge in edges {
                    if let EdgeTarget::File(target) = &edge.target {
                        if self.passes_filters(target, filter_roots, filter_languages) {
                            outgoing.push(target.clone());
                        }
                    }
                }
            }
        }

        // Collect direct incoming sources (dependents) that pass filters.
        let mut incoming: Vec<String> = Vec::new();
        if matches!(direction, GraphDirection::Dependents | GraphDirection::Both) {
            if let Some(sources) = self.reverse.get(key) {
                for source in sources {
                    if self.passes_filters(source, filter_roots, filter_languages) {
                        incoming.push(source.clone());
                    }
                }
            }
        }

        // Dedup the combined neighbor set (a neighbor may appear in both).
        let mut all_neighbors: Vec<String> =
            outgoing.iter().chain(incoming.iter()).cloned().collect();
        all_neighbors.sort();
        all_neighbors.dedup();

        // Determine role: if a neighbor appears in BOTH outgoing and incoming,
        // pick deterministic existing-compatible role (Dependency survives).
        let outgoing_set: HashSet<&str> = outgoing.iter().map(|s| s.as_str()).collect();
        let incoming_set: HashSet<&str> = incoming.iter().map(|s| s.as_str()).collect();

        // ── Build direction-allowed, sorted+deduped edge pairs BEFORE any caps ──
        let mut raw_edges: Vec<(String, String)> = Vec::new();

        // Outgoing edges (focus→dependency) only when Dependencies or Both.
        if matches!(
            direction,
            GraphDirection::Dependencies | GraphDirection::Both
        ) {
            if let Some(edges) = self.edges.get(key) {
                for edge in edges {
                    if let EdgeTarget::File(target) = &edge.target {
                        if all_neighbors.contains(target) {
                            raw_edges.push((key.to_string(), target.clone()));
                        }
                    }
                }
            }
        }
        // Incoming edges (dependent→focus) only when Dependents or Both.
        if matches!(direction, GraphDirection::Dependents | GraphDirection::Both) {
            if let Some(sources) = self.reverse.get(key) {
                for source in sources {
                    if all_neighbors.contains(source) {
                        raw_edges.push((source.to_string(), key.to_string()));
                    }
                }
            }
        }

        raw_edges.sort();
        raw_edges.dedup();

        // Total BEFORE caps — accurate deduped pair count.
        let total_nodes_available = 1 + all_neighbors.len();
        let total_edges_available = raw_edges.len();

        // Build sortable node list: focus first, then sort neighbors by
        // relevant degree (out_degree for outgoing, in_degree for incoming)
        // then lex path.
        let mut sortable: Vec<(usize, usize, String)> = Vec::new();
        sortable.push((0, 0, key.to_string()));

        for n in &all_neighbors {
            let rank = if outgoing_set.contains(n.as_str()) {
                1
            } else {
                2
            };
            let degree = if outgoing_set.contains(n.as_str()) && !incoming_set.contains(n.as_str())
            {
                self.edges.get(n).map_or(0, |es| es.len())
            } else if incoming_set.contains(n.as_str()) {
                self.reverse.get(n).map_or(0, |v| v.len())
            } else {
                0
            };
            sortable.push((rank, usize::MAX - degree, n.clone()));
        }
        sortable.sort();

        // Apply node cap.
        let nodes_truncated = sortable.len() > max_nodes;
        sortable.truncate(max_nodes);

        let node_set: HashSet<&str> = sortable.iter().map(|(_, _, p)| p.as_str()).collect();

        // Filter pre-computed edges to only those within the capped node set, then apply edge cap.
        let mut filtered_edges: Vec<(String, String)> = raw_edges
            .into_iter()
            .filter(|(from, to)| node_set.contains(from.as_str()) && node_set.contains(to.as_str()))
            .collect();
        // edges_truncated = edges were omitted by either node cap (endpoint removed) or edge cap.
        let edges_truncated =
            filtered_edges.len() < total_edges_available || filtered_edges.len() > max_edges;
        if filtered_edges.len() > max_edges {
            filtered_edges.truncate(max_edges);
        }

        // Count in/out degree within the view for each node from the final edge set.
        let mut in_deg: HashMap<&str, usize> = HashMap::new();
        let mut out_deg: HashMap<&str, usize> = HashMap::new();
        for (from, to) in &filtered_edges {
            *out_deg.entry(from.as_str()).or_insert(0) += 1;
            *in_deg.entry(to.as_str()).or_insert(0) += 1;
        }

        let nodes: Vec<GraphViewNode> = sortable
            .into_iter()
            .map(|(_, _, p)| {
                let lang = self
                    .nodes
                    .get(&p)
                    .map(|n| format!("{:?}", n.lang))
                    .unwrap_or_default();
                let role = if p == key {
                    GraphNodeRole::Focus
                } else if outgoing_set.contains(p.as_str()) {
                    GraphNodeRole::Dependency
                } else if incoming_set.contains(p.as_str()) {
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
                    depth_from_focus: Some(if p == key { 0 } else { 1 }),
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

    /// Build a focus view: BFS from `key` in the requested direction(s).
    /// Filters are applied: filtered-out nodes are not discovered or traversed through.
    fn focus_view(
        &self,
        base: GraphViewResult,
        key: &str,
        direction: &GraphDirection,
        depth: u32,
        caps: ViewCaps<'_>,
    ) -> GraphViewResult {
        let ViewCaps {
            max_nodes,
            max_edges,
            filter_roots,
            filter_languages,
        } = caps;
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

    // ─── Phase 1: structured reference support ───────────────────────────

    /// Compute aggregate counts from per-source structured references.
    /// Returns (external, unresolved, ambiguous, dynamic).
    #[allow(dead_code)] // Used by daemon summary; retained for future IPC queries.
    pub fn aggregate_ref_counts(&self) -> (usize, usize, usize, usize) {
        let mut ext = 0usize;
        let mut unres = 0usize;
        let mut amb = 0usize;
        let mut dyn_count = 0usize;
        for sr in self.source_refs.values() {
            ext += sr.external_count();
            unres += sr.unresolved_count();
            amb += sr.ambiguous_count();
            dyn_count += sr.dynamic_count();
        }
        (ext, unres, amb, dyn_count)
    }

    /// Verify graph counts, semantic uniqueness, exact reverse membership, and
    /// structured-reference source ownership.
    #[allow(dead_code)] // Called via debug_assert! in watcher; retained for future tooling.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.file_count != self.nodes.len() {
            return Err(format!(
                "file_count {} != nodes.len() {}",
                self.file_count,
                self.nodes.len()
            ));
        }

        let actual_resolved = self
            .edges
            .values()
            .flatten()
            .filter(|edge| edge.is_resolved())
            .count();
        if self.edge_count != actual_resolved {
            return Err(format!(
                "edge_count {} != resolved edge count {}",
                self.edge_count, actual_resolved
            ));
        }

        for (source, edges) in &self.edges {
            if !self.nodes.contains_key(source) {
                return Err(format!("edge source is not a node: {source}"));
            }
            let mut semantic = HashSet::new();
            for edge in edges {
                if !semantic.insert((&edge.target, &edge.kind)) {
                    return Err(format!("duplicate semantic edge from {source}: {edge:?}"));
                }
                if let EdgeTarget::File(target) = &edge.target {
                    let occurrences = self
                        .reverse
                        .get(target)
                        .map_or(0, |sources| sources.iter().filter(|s| *s == source).count());
                    if occurrences != 1 {
                        return Err(format!(
                            "reverse[{target}] contains source {source} {occurrences} times"
                        ));
                    }
                }
            }
        }

        for (target, sources) in &self.reverse {
            let mut unique = HashSet::new();
            for source in sources {
                if !unique.insert(source) {
                    return Err(format!("duplicate reverse source {source} for {target}"));
                }
                let corresponds = self.edges.get(source).is_some_and(|edges| {
                    edges.iter().any(
                        |edge| matches!(&edge.target, EdgeTarget::File(path) if path == target),
                    )
                });
                if !corresponds {
                    return Err(format!("stale reverse edge {source} -> {target}"));
                }
            }
        }

        for source in self.source_refs.keys() {
            if !self.nodes.contains_key(source) {
                return Err(format!("source refs belong to unknown node: {source}"));
            }
        }
        Ok(())
    }

    /// Set structured import references for a source file.
    pub fn set_source_refs(&mut self, source: &str, refs: SourceRefs) {
        self.source_refs.insert(source.to_string(), refs);
    }

    /// Atomically set both edges and structured refs for a source file.
    ///
    /// This ensures graph edges and SourceRefs are always consistent — callers
    /// never see a state where one is updated but not the other.
    pub fn set_edges_and_refs(
        &mut self,
        source: &str,
        lang: Lang,
        new_edges: Vec<Edge>,
        refs: SourceRefs,
    ) {
        self.set_edges(source, lang, new_edges);
        self.set_source_refs(source, refs);
    }
}

/// Check if a file path looks like a test file (multi-language heuristic).
fn is_test_file(path: &str) -> bool {
    let p = path.replace('\\', "/");
    let name = p.rsplit('/').next().unwrap_or(&p);

    // Rust: test_*.rs, *_test.rs, paths under /tests/ or /test/
    if name.starts_with("test_") && name.ends_with(".rs") {
        return true;
    }
    if name.ends_with("_test.rs") {
        return true;
    }
    // Python: test_*.py, *_test.py
    if name.starts_with("test_") && name.ends_with(".py") {
        return true;
    }
    if name.ends_with("_test.py") {
        return true;
    }
    // TypeScript/JS: *.test.ts, *.spec.ts, *.test.tsx, *.spec.tsx, *.test.js, *.spec.js
    if name.contains(".test.") || name.contains(".spec.") {
        return true;
    }
    // Go: *_test.go
    if name.ends_with("_test.go") {
        return true;
    }
    // Java: *Test.java, *Tests.java
    if (name.ends_with("Test.java") || name.ends_with("Tests.java")) && name.len() > 9 {
        return true;
    }
    // PHP: *Test.php
    if name.ends_with("Test.php") && name.len() > 8 {
        return true;
    }
    // Path segment check for /tests/ or /test/
    if p.contains("/tests/") || p.contains("/test/") {
        return true;
    }
    false
}

/// Check if a file path looks like a config file.
fn is_config_file(path: &str) -> bool {
    let p = path.replace('\\', "/");
    let name = p.rsplit('/').next().unwrap_or(&p);
    matches!(
        name,
        "Cargo.toml"
            | "pyproject.toml"
            | "package.json"
            | "tsconfig.json"
            | "go.mod"
            | "build.gradle"
            | "composer.json"
    ) || name.starts_with("config.")
        || name.starts_with(".env")
}

#[cfg(test)]
#[path = "graph_test.rs"]
mod tests;
