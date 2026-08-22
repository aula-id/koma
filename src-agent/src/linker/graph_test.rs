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
    // No-focus returns metadata-only: empty nodes/edges, no graph.
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    // No-focus contract: totals are ZERO.
    assert_eq!(result.total_nodes_available, 0);
    assert_eq!(result.total_edges_available, 0);
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
    // No-focus returns metadata only — empty graph nodes/edges.
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    // No-focus contract: totals ZERO.
    assert_eq!(result.total_nodes_available, 0);
    assert_eq!(result.total_edges_available, 0);
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
    assert_eq!(
        result.file_count, 2,
        "file_count should be 2 (a.rs + b.rs under ws_a)"
    );
    assert_eq!(
        result.edge_count, 1,
        "edge_count should be 1 (only a→b, both endpoints in ws_a)"
    );
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
    // Build 5 nodes in ws_a, filter to ws_a, no-focus.
    // No-focus returns metadata only; total_nodes_available reflects filtered count.
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
    // No-focus: empty graph, metadata only. Contract: totals ZERO.
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    assert_eq!(
        result.total_nodes_available, 0,
        "no-focus contract: total_nodes_available = 0"
    );
    assert_eq!(result.total_edges_available, 0);
    assert!(!result.nodes_truncated);
}

#[test]
fn overview_filtered_degrees_only_allowed_edges() {
    // a→b (both in ws_a), c→b (in ws_b). Filter to ws_a, no-focus.
    // No-focus returns metadata only; verify filtered file_count.
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
    // No-focus returns empty graph, but filtered metadata is correct.
    assert!(result.nodes.is_empty());
    assert_eq!(result.file_count, 2, "filtered file_count = a.rs + b.rs");
    assert_eq!(result.edge_count, 1, "filtered edge_count = a→b only");
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

// ─── NEW: strict one-hop depth=1 focus tests ──────────────────────────

#[test]
fn direct_star_depth1_neighbor_depth_is_one() {
    // Focus at depth 0, neighbors at depth 1.
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
        "/d.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);
    g.set_edges("/c.rs", Lang::Rust, vec![]);

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Both);
    let result = g.visualization_view(&req);
    assert_eq!(result.focus.as_deref(), Some("/a.rs"));
    // Focus node: depth 0
    let focus = result
        .nodes
        .iter()
        .find(|n| n.role == GraphNodeRole::Focus)
        .unwrap();
    assert_eq!(focus.depth_from_focus, Some(0));
    // Neighbors: depth 1
    for n in &result.nodes {
        if n.role != GraphNodeRole::Focus {
            assert_eq!(
                n.depth_from_focus,
                Some(1),
                "neighbor {} should have depth 1",
                n.path
            );
        }
    }
}

#[test]
fn direct_star_outgoing_only() {
    // Dependencies direction: only outgoing edges and targets.
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
        "/d.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);
    g.set_edges("/c.rs", Lang::Rust, vec![]);

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Dependencies);
    let result = g.visualization_view(&req);
    let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
    assert!(paths.contains(&"/a.rs"));
    assert!(paths.contains(&"/b.rs"));
    assert!(paths.contains(&"/c.rs"));
    assert!(
        !paths.contains(&"/d.rs"),
        "dependents should not appear in Dependencies mode"
    );
    assert_eq!(result.edges.len(), 2);
}

#[test]
fn direct_star_incoming_only() {
    // Dependents direction: only incoming edges and sources.
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
        "/c.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Dependents);
    let result = g.visualization_view(&req);
    let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
    assert!(paths.contains(&"/a.rs"));
    assert!(
        !paths.contains(&"/b.rs"),
        "dependencies should not appear in Dependents mode"
    );
    assert!(paths.contains(&"/c.rs"));
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].from, "/c.rs");
    assert_eq!(result.edges[0].to, "/a.rs");
}

#[test]
fn direct_star_reciprocal_both_arrows() {
    // A→B and B→A. Under Both, both edges survive with distinct from/to.
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
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Both);
    let result = g.visualization_view(&req);
    // Both nodes present.
    let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
    assert!(paths.contains(&"/a.rs"));
    assert!(paths.contains(&"/b.rs"));
    // Two distinct edges: a→b and b→a.
    assert_eq!(result.edges.len(), 2);
    assert_eq!(
        result.total_edges_available, 2,
        "total edges = 2 deduped pairs"
    );
    assert_eq!(
        result.total_nodes_available, 2,
        "total nodes = focus + 1 neighbor"
    );
    let has_ab = result
        .edges
        .iter()
        .any(|e| e.from == "/a.rs" && e.to == "/b.rs");
    let has_ba = result
        .edges
        .iter()
        .any(|e| e.from == "/b.rs" && e.to == "/a.rs");
    assert!(has_ab, "missing a→b edge");
    assert!(has_ba, "missing b→a edge");
    // b is both incoming (Dependent) and outgoing (Dependency).
    // Per spec: Dependency survives for reciprocal under Both.
    let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
    assert_eq!(
        b.role,
        GraphNodeRole::Dependency,
        "reciprocal neighbor should be Dependency under Both"
    );
}

#[test]
fn direct_star_reciprocal_deps_only() {
    // A→B and B→A. Under Dependencies, only A→B survives.
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
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Dependencies);
    let result = g.visualization_view(&req);
    let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
    assert_eq!(b.role, GraphNodeRole::Dependency);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].from, "/a.rs");
    assert_eq!(result.edges[0].to, "/b.rs");
}

#[test]
fn direct_star_reciprocal_dependents_only() {
    // A→B and B→A. Under Dependents, only B→A survives.
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
            target: EdgeTarget::File("/a.rs".into()),
            kind: EdgeKind::Import,
        }],
    );

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Dependents);
    let result = g.visualization_view(&req);
    let b = result.nodes.iter().find(|n| n.path == "/b.rs").unwrap();
    assert_eq!(b.role, GraphNodeRole::Dependent);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].from, "/b.rs");
    assert_eq!(result.edges[0].to, "/a.rs");
}

#[test]
fn direct_star_no_neighbor_neighbor_edges() {
    // A→B, A→C, B→C. Focus A depth=1: edges A→B, A→C, but NOT B→C.
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
            target: EdgeTarget::File("/c.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/c.rs", Lang::Rust, vec![]);

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Dependencies);
    let result = g.visualization_view(&req);
    // All 3 nodes present.
    assert_eq!(result.nodes.len(), 3);
    // Only edges touching focus: a→b, a→c. NOT b→c.
    let has_bc = result
        .edges
        .iter()
        .any(|e| e.from == "/b.rs" && e.to == "/c.rs");
    assert!(
        !has_bc,
        "neighbor-neighbor edge b→c should NOT appear at depth=1"
    );
    assert_eq!(result.edges.len(), 2);
}

#[test]
fn direct_star_caps_and_deterministic() {
    // Build focus with many neighbors, verify caps and stable ordering.
    let mut g = ImportGraph::new();
    g.workspace_roots = vec!["/".to_string()];
    let mut out_edges = Vec::new();
    for i in 0..10 {
        let path = format!("/n{i}.rs");
        g.set_edges(&path, Lang::Rust, vec![]);
        out_edges.push(Edge {
            target: EdgeTarget::File(path),
            kind: EdgeKind::Import,
        });
    }
    g.set_edges("/focus.rs", Lang::Rust, out_edges);

    let req = VisualizationRequest {
        path: Some("/focus.rs".to_string()),
        depth: 1,
        direction: GraphDirection::Dependencies,
        max_nodes: 5,
        max_edges: 3,
        filter_roots: None,
        filter_languages: None,
    };
    let r1 = g.visualization_view(&req);
    let r2 = g.visualization_view(&req);
    assert_eq!(r1.nodes.len(), 5, "node cap applied");
    assert_eq!(r1.edges.len(), 3, "edge cap applied");
    assert!(r1.nodes_truncated);
    assert!(r1.edges_truncated);
    // Focus is first.
    assert_eq!(r1.nodes[0].role, GraphNodeRole::Focus);
    // Deterministic.
    assert_eq!(r1.nodes.len(), r2.nodes.len());
    for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
        assert_eq!(a.path, b.path);
    }
}

#[test]
fn direct_star_metadata_only_no_focus() {
    // No-focus returns metadata only with empty graph.
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

    let req = make_req(None, 1, GraphDirection::Both);
    let result = g.visualization_view(&req);
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    assert_eq!(result.file_count, 2);
    // No-focus contract: totals are ZERO.
    assert_eq!(result.total_nodes_available, 0);
    assert_eq!(result.total_edges_available, 0);
    assert!(!result.nodes_truncated);
    assert!(!result.edges_truncated);
}

#[test]
fn direct_star_filter_neighbor_passes_filters() {
    // Focus passes filter, some neighbors do not.
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
        path: Some("/ws_a/a.rs".to_string()),
        depth: 1,
        direction: GraphDirection::Dependencies,
        max_nodes: 100,
        max_edges: 100,
        filter_roots: Some(vec!["/ws_a".to_string()]),
        filter_languages: None,
    };
    let result = g.visualization_view(&req);
    let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
    assert!(paths.contains(&"/ws_a/a.rs"));
    assert!(paths.contains(&"/ws_a/b.rs"));
    assert!(!paths.contains(&"/ws_b/c.rs"));
    assert_eq!(result.edges.len(), 1);
}

// ─── Duplicate edge storage does not inflate totals ────────────────────

#[test]
fn direct_star_duplicate_storage_edges_dedup() {
    // Simulate duplicate storage entries by calling set_edges twice
    // with the same target — the second call replaces the first in
    // ImportGraph::set_edges, so the edge_count stays correct.
    // But verify the visualisation totals are clean.
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
    // Re-set same edges (simulates a re-scan).
    g.set_edges(
        "/a.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/b.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);

    let req = make_req(Some("/a.rs"), 1, GraphDirection::Both);
    let result = g.visualization_view(&req);
    assert_eq!(
        result.edges.len(),
        1,
        "only one a→b edge despite duplicate storage"
    );
    assert_eq!(result.total_edges_available, 1);
    assert_eq!(result.total_nodes_available, 2);
}

#[test]
fn direct_star_totals_pre_cap() {
    // Verify total_nodes/total_edges are pre-cap values.
    let mut g = ImportGraph::new();
    g.workspace_roots = vec!["/".to_string()];
    g.set_edges(
        "/focus.rs",
        Lang::Rust,
        vec![
            Edge {
                target: EdgeTarget::File("/n1.rs".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::File("/n2.rs".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::File("/n3.rs".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::File("/n4.rs".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::File("/n5.rs".into()),
                kind: EdgeKind::Import,
            },
        ],
    );
    for i in 1..=5 {
        g.set_edges(&format!("/n{i}.rs"), Lang::Rust, vec![]);
    }

    let req = VisualizationRequest {
        path: Some("/focus.rs".to_string()),
        depth: 1,
        direction: GraphDirection::Dependencies,
        max_nodes: 3,
        max_edges: 2,
        filter_roots: None,
        filter_languages: None,
    };
    let result = g.visualization_view(&req);
    assert_eq!(
        result.total_nodes_available, 6,
        "total includes focus + 5 neighbors"
    );
    assert_eq!(
        result.total_edges_available, 5,
        "total includes all 5 edges before cap"
    );
    assert_eq!(result.nodes.len(), 3, "nodes capped to 3");
    assert_eq!(result.edges.len(), 2, "edges capped to 2");
    assert!(result.nodes_truncated);
    assert!(result.edges_truncated);
}

// ─── edit_context tests ──────────────────────────────────────────────

#[test]
fn edit_context_basic_chain() {
    // a → b → c, all in same root.
    let g = chain_graph();
    let ctx = g.edit_context("/a.rs");
    assert_eq!(ctx.imports, vec!["/b.rs"]);
    assert!(ctx.imported_by.is_empty());
    assert!(ctx.is_entry_point);
    assert!(!ctx.is_leaf);
    assert_eq!(ctx.transitive_dependents_count, 0);
    assert!(ctx.cross_boundary_deps.is_empty());
}

#[test]
fn edit_context_leaf_node() {
    let g = chain_graph();
    let ctx = g.edit_context("/c.rs");
    assert!(ctx.imports.is_empty());
    assert_eq!(ctx.imported_by, vec!["/b.rs"]);
    assert!(!ctx.is_entry_point);
    assert!(ctx.is_leaf);
}

#[test]
fn edit_context_entry_point_count() {
    // a → b, c → b. b is depended on by a and c.
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
        "/c.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/b.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);

    let ctx = g.edit_context("/b.rs");
    assert_eq!(ctx.imports, Vec::<String>::new());
    assert_eq!(ctx.imported_by.len(), 2);
    assert!(!ctx.is_entry_point);
    assert!(ctx.is_leaf);
    // Transitive: b itself excluded → 2 dependents (a, c), both entry points.
    assert_eq!(ctx.transitive_dependents_count, 2);
    assert_eq!(ctx.entry_point_count, 2);
}

#[test]
fn edit_context_cross_boundary() {
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

    let ctx = g.edit_context("/ws_a/a.rs");
    assert_eq!(ctx.cross_boundary_deps.len(), 1);
    assert_eq!(ctx.cross_boundary_deps[0].0, "/ws_b");
    assert_eq!(ctx.cross_boundary_deps[0].1, "/ws_b/b.rs");
}

#[test]
fn edit_context_unresolved_imports() {
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
                target: EdgeTarget::External("serde".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::External("tokio".into()),
                kind: EdgeKind::Import,
            },
        ],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);

    let ctx = g.edit_context("/a.rs");
    assert_eq!(ctx.unresolved_imports, vec!["serde", "tokio"]);
    assert!(!ctx.is_leaf, "has unresolved edges so not a leaf");
}

#[test]
fn edit_context_test_and_config_detection() {
    let mut g = ImportGraph::new();
    g.workspace_roots = vec!["/".to_string()];
    // main.rs → lib.rs → Cargo.toml; main_test.rs → main.rs.
    g.set_edges(
        "/src/main.rs",
        Lang::Rust,
        vec![
            Edge {
                target: EdgeTarget::File("/src/lib.rs".into()),
                kind: EdgeKind::Import,
            },
            Edge {
                target: EdgeTarget::File("/tests/main_test.rs".into()),
                kind: EdgeKind::Import,
            },
        ],
    );
    g.set_edges(
        "/tests/main_test.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/src/main.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges(
        "/src/lib.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/Cargo.toml".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/Cargo.toml", Lang::Unknown, vec![]);

    // main.rs: test file is a direct dep + dependent.
    let ctx = g.edit_context("/src/main.rs");
    assert!(
        ctx.related_tests
            .contains(&"/tests/main_test.rs".to_string()),
        "should detect test file: {:?}",
        ctx.related_tests
    );
    // Cargo.toml is NOT a direct dep/dependent of main.rs (it's lib.rs's dep).
    assert!(
        ctx.related_configs.is_empty(),
        "Cargo.toml should not appear as config for main.rs: {:?}",
        ctx.related_configs
    );

    // lib.rs: Cargo.toml is a direct dep → config detection.
    let ctx_lib = g.edit_context("/src/lib.rs");
    assert!(
        ctx_lib.related_configs.contains(&"/Cargo.toml".to_string()),
        "should detect config file for lib.rs: {:?}",
        ctx_lib.related_configs
    );
}

#[test]
fn edit_context_not_found_returns_empty() {
    let g = ImportGraph::new();
    let ctx = g.edit_context("/nonexistent.rs");
    assert!(ctx.imports.is_empty());
    assert!(ctx.imported_by.is_empty());
    assert!(ctx.is_entry_point);
    assert!(ctx.is_leaf);
    assert_eq!(ctx.transitive_dependents_count, 0);
}

#[test]
fn edit_context_is_leaf_false_when_only_external() {
    let mut g = ImportGraph::new();
    g.workspace_roots = vec!["/".to_string()];
    g.set_edges(
        "/a.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::External("serde".into()),
            kind: EdgeKind::Import,
        }],
    );

    let ctx = g.edit_context("/a.rs");
    assert!(ctx.imports.is_empty()); // no resolved file imports
    assert!(!ctx.is_leaf); // but has external edge
    assert_eq!(ctx.unresolved_imports, vec!["serde"]);
}

// ─── is_test_file / is_config_file heuristic tests ───────────────────

#[test]
fn test_file_heuristics() {
    assert!(super::is_test_file("/src/test_foo.rs"));
    assert!(super::is_test_file("/src/foo_test.rs"));
    assert!(super::is_test_file("/src/tests/bar.rs"));
    assert!(super::is_test_file("/src/test/bar.rs"));
    assert!(super::is_test_file("/test_foo.py"));
    assert!(super::is_test_file("/foo_test.py"));
    assert!(super::is_test_file("/foo.test.ts"));
    assert!(super::is_test_file("/foo.spec.tsx"));
    assert!(super::is_test_file("/foo_test.go"));
    assert!(super::is_test_file("/FooTest.java"));
    assert!(super::is_test_file("/FooTests.java"));
    assert!(super::is_test_file("/FooTest.php"));
    assert!(!super::is_test_file("/src/main.rs"));
    assert!(!super::is_test_file("/src/lib.rs"));
}

#[test]
fn config_file_heuristics() {
    assert!(super::is_config_file("/Cargo.toml"));
    assert!(super::is_config_file("/pyproject.toml"));
    assert!(super::is_config_file("/package.json"));
    assert!(super::is_config_file("/tsconfig.json"));
    assert!(super::is_config_file("/go.mod"));
    assert!(super::is_config_file("/build.gradle"));
    assert!(super::is_config_file("/composer.json"));
    assert!(super::is_config_file("/config.yaml"));
    assert!(super::is_config_file("/.env"));
    assert!(!super::is_config_file("/src/main.rs"));
}

// ─── Phase 1: remove_node edge_count fix tests ────────────────────────

#[test]
fn remove_node_edge_count_after_incoming_removal() {
    // A → B, B → C. Remove B: both edges should be gone.
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

    assert_eq!(g.edge_count, 2);
    g.remove_node("/b.rs");
    assert_eq!(g.edge_count, 0, "both edges should be removed");
    assert!(
        g.check_invariants().is_ok(),
        "invariants violated after remove_node"
    );
}

#[test]
fn remove_node_preserves_unrelated_edges() {
    // A → B, C → D. Remove B: A has no edges, C→D intact.
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
        "/c.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/d.rs".into()),
            kind: EdgeKind::Import,
        }],
    );

    assert_eq!(g.edge_count, 2);
    g.remove_node("/b.rs");
    assert_eq!(g.edge_count, 1, "C→D should survive");
    assert_eq!(g.dependents("/d.rs"), vec!["/c.rs"]);
    assert!(g.check_invariants().is_ok());
}

#[test]
fn remove_node_shared_target() {
    // A → C, B → C. Remove A: B→C intact, edge_count = 1.
    let mut g = ImportGraph::new();
    g.set_edges(
        "/a.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/c.rs".into()),
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

    assert_eq!(g.edge_count, 2);
    g.remove_node("/a.rs");
    assert_eq!(g.edge_count, 1, "B→C should survive");
    assert_eq!(g.dependents("/c.rs"), vec!["/b.rs"]);
    assert!(g.check_invariants().is_ok());
}

// ─── Phase 1: check_invariants tests ──────────────────────────────────

#[test]
fn check_invariants_clean_graph() {
    let mut g = ImportGraph::new();
    g.set_edges(
        "/a.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/b.rs".into()),
            kind: EdgeKind::Import,
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);
    assert!(g.check_invariants().is_ok());
}

#[test]
fn check_invariants_after_multiple_mutations() {
    let mut g = ImportGraph::new();
    // Build a chain.
    for i in 0..10 {
        let path = format!("/f{i}.rs");
        let target = format!("/f{}.rs", i + 1);
        if i < 9 {
            g.set_edges(
                &path,
                Lang::Rust,
                vec![Edge {
                    target: EdgeTarget::File(target),
                    kind: EdgeKind::Import,
                }],
            );
        } else {
            g.set_edges(&path, Lang::Rust, vec![]);
        }
    }
    assert!(g.check_invariants().is_ok());
    assert_eq!(g.edge_count, 9);

    // Remove a middle node.
    g.remove_node("/f5.rs");
    assert!(g.check_invariants().is_ok());
    // f4→f5 removed + f5→f6 removed = 2 fewer edges.
    assert_eq!(g.edge_count, 7);
}

// ─── Phase 1: EdgeKind::Structured variant ────────────────────────────

#[test]
fn edge_kind_structured_variant() {
    let k1 = EdgeKind::Structured {
        import_kind: crate::linker::reference::ImportKind::TypeOnly,
        condition: Some("cfg(test)".into()),
    };
    let k2 = EdgeKind::Structured {
        import_kind: crate::linker::reference::ImportKind::TypeOnly,
        condition: Some("cfg(test)".into()),
    };
    assert_eq!(k1, k2);
    assert_eq!(format!("{:?}", k1), format!("{:?}", k2));

    // Different condition → not equal.
    let k3 = EdgeKind::Structured {
        import_kind: crate::linker::reference::ImportKind::TypeOnly,
        condition: None,
    };
    assert_ne!(k1, k3);
}

#[test]
fn edge_kind_structured_with_graph() {
    let mut g = ImportGraph::new();
    g.set_edges(
        "/a.rs",
        Lang::Rust,
        vec![Edge {
            target: EdgeTarget::File("/b.rs".into()),
            kind: EdgeKind::Structured {
                import_kind: crate::linker::reference::ImportKind::ReExport,
                condition: None,
            },
        }],
    );
    g.set_edges("/b.rs", Lang::Rust, vec![]);

    assert_eq!(g.edge_count, 1);
    assert_eq!(g.dependencies("/a.rs"), vec!["/b.rs"]);
    assert_eq!(g.dependents("/b.rs"), vec!["/a.rs"]);
    assert!(g.check_invariants().is_ok());
}

// ─── Phase 1: source_refs and aggregate counts ────────────────────────

#[test]
fn source_refs_store_and_retrieve() {
    use crate::linker::reference::{ImportRef, Resolution, SourceRefs};
    let mut g = ImportGraph::new();

    g.ensure_node("/a.rs", Lang::Rust);
    let mut sr = SourceRefs::default();
    sr.push(
        ImportRef {
            specifier: "serde".into(),
            kind: crate::linker::reference::ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::External {
            package: "serde".into(),
        },
    );
    g.set_source_refs("/a.rs", sr);

    assert_eq!(g.source_refs.len(), 1);
    assert_eq!(g.source_refs["/a.rs"].external_count(), 1);
}

#[test]
fn aggregate_ref_counts() {
    use crate::linker::reference::{ImportRef, Resolution, SourceRefs};
    let mut g = ImportGraph::new();

    let mut sr1 = SourceRefs::default();
    sr1.push(
        ImportRef {
            specifier: "a".into(),
            kind: crate::linker::reference::ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::External {
            package: "a".into(),
        },
    );
    sr1.push(
        ImportRef {
            specifier: "b".into(),
            kind: crate::linker::reference::ImportKind::Dynamic,
            span: None,
            condition: None,
        },
        Resolution::Dynamic {
            expression: "b".into(),
        },
    );
    g.set_source_refs("/a.rs", sr1);

    let mut sr2 = SourceRefs::default();
    sr2.push(
        ImportRef {
            specifier: "c".into(),
            kind: crate::linker::reference::ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::Unresolved {
            reason: crate::linker::reference::UnresolvedReason::NotFound,
        },
    );
    sr2.push(
        ImportRef {
            specifier: "d".into(),
            kind: crate::linker::reference::ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::Ambiguous {
            candidates: vec!["/x.rs".into(), "/y.rs".into()],
        },
    );
    g.set_source_refs("/b.rs", sr2);

    let (ext, unres, amb, dyn_c) = g.aggregate_ref_counts();
    assert_eq!(ext, 1);
    assert_eq!(unres, 1);
    assert_eq!(amb, 1);
    assert_eq!(dyn_c, 1);
}

#[test]
fn clear_resets_source_refs() {
    use crate::linker::reference::SourceRefs;
    let mut g = ImportGraph::new();
    g.set_source_refs("/a.rs", SourceRefs::default());
    assert_eq!(g.source_refs.len(), 1);
    g.clear();
    assert!(g.source_refs.is_empty());
}
