//! In-memory directed import graph.
//!
//! Nodes are canonical file paths; edges represent import/module relationships
//! extracted by tree-sitter. A reverse index enables fast dependents/impact
//! queries without traversal.

use std::collections::{HashMap, VecDeque};

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
}
