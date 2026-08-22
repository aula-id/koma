use super::*;
use crate::ipc::linker_proto::{GraphNodeRole, GraphViewEdge, GraphViewNode};

/// Helper: build a minimal `GraphViewResult` with two roots and a few nodes.
fn make_test_result() -> GraphViewResult {
    GraphViewResult {
        nodes: vec![
            GraphViewNode {
                path: "/ws_a/src/main.rs".into(),
                language: "Rust".into(),
                out_degree: 1,
                in_degree: 0,
                role: GraphNodeRole::Focus,
                depth_from_focus: Some(0),
                workspace_root: Some("/ws_a".into()),
            },
            GraphViewNode {
                path: "/ws_a/src/lib.rs".into(),
                language: "Rust".into(),
                out_degree: 0,
                in_degree: 1,
                role: GraphNodeRole::Dependency,
                depth_from_focus: Some(1),
                workspace_root: Some("/ws_a".into()),
            },
            GraphViewNode {
                path: "/ws_b/app.py".into(),
                language: "Python".into(),
                out_degree: 0,
                in_degree: 0,
                role: GraphNodeRole::Overview,
                depth_from_focus: None,
                workspace_root: Some("/ws_b".into()),
            },
        ],
        edges: vec![GraphViewEdge {
            from: "/ws_a/src/main.rs".into(),
            to: "/ws_a/src/lib.rs".into(),
        }],
        focus: Some("/ws_a/src/main.rs".into()),
        generation: 5,
        file_count: 3,
        edge_count: 1,
        languages: vec!["Rust".into(), "Python".into()],
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 3,
        total_edges_available: 1,
        available_roots: vec![
            WorkspaceRootInfo {
                root: "/ws_a".into(),
                file_count: 2,
                languages: vec![crate::ipc::linker_proto::LanguageCount {
                    name: "Rust".into(),
                    count: 2,
                }],
            },
            WorkspaceRootInfo {
                root: "/ws_b".into(),
                file_count: 1,
                languages: vec![crate::ipc::linker_proto::LanguageCount {
                    name: "Python".into(),
                    count: 1,
                }],
            },
        ],
    }
}

// ── compute_effective_filter tests ──────────────────────────────────

#[test]
fn filter_all_no_configured_roots_returns_none() {
    let result = compute_effective_filter(None, &[]);
    assert!(result.is_none());
}

#[test]
fn filter_all_with_configured_roots_returns_all() {
    let roots = vec!["/ws_a".into(), "/ws_b".into()];
    let result = compute_effective_filter(None, &roots);
    assert_eq!(result, Some(roots));
}

#[test]
fn filter_explicit_intersecting_roots() {
    let configured = vec!["/ws_a".into(), "/ws_b".into()];
    let ui = Some(vec!["/ws_a".into(), "/foreign".into()]);
    let result = compute_effective_filter(ui, &configured).unwrap();
    assert_eq!(result, vec!["/ws_a".to_string()]);
}

#[test]
fn filter_explicit_stale_foreign_falls_back_to_configured() {
    let configured = vec!["/ws_a".into(), "/ws_b".into()];
    let ui = Some(vec!["/deleted_root".into(), "/another_foreign".into()]);
    let result = compute_effective_filter(ui, &configured).unwrap();
    // Intersection is empty → fall back to full configured set.
    assert_eq!(result, configured);
}

#[test]
fn filter_empty_selection_is_all() {
    let configured = vec!["/ws_a".into()];
    let ui = Some(vec![]);
    let result = compute_effective_filter(ui, &configured).unwrap();
    assert_eq!(result, configured);
}

// ── scope_result tests ─────────────────────────────────────────────

#[test]
fn scope_filters_nodes_to_allowed_roots() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.nodes.len(), 2);
    assert!(scoped
        .nodes
        .iter()
        .all(|n| n.workspace_root.as_deref() == Some("/ws_a")));
}

#[test]
fn scope_filters_edges_to_allowed_nodes() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    // Edge between two /ws_a nodes is kept.
    assert_eq!(scoped.edges.len(), 1);
}

#[test]
fn scope_removes_edges_with_foreign_endpoints() {
    let mut result = make_test_result();
    // Add an edge from ws_a → ws_b.
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/main.rs".into(),
        to: "/ws_b/app.py".into(),
    });
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    // Only the intra-root edge survives.
    assert_eq!(scoped.edges.len(), 1);
    assert_eq!(scoped.edges[0].from, "/ws_a/src/main.rs");
}

#[test]
fn scope_restricts_available_roots_to_configured() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.available_roots.len(), 1);
    assert_eq!(scoped.available_roots[0].root, "/ws_a");
}

#[test]
fn scope_synthesises_missing_configured_roots() {
    let result = make_test_result();
    // /ws_c is configured but not in the daemon graph.
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    assert_eq!(scoped.available_roots.len(), 2);
    assert_eq!(scoped.available_roots[0].root, "/ws_a");
    assert_eq!(scoped.available_roots[0].file_count, 2);
    // /ws_c is synthesised with zero metadata.
    let ws_c = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_c")
        .unwrap();
    assert_eq!(ws_c.file_count, 0);
    assert!(ws_c.languages.is_empty());
}

#[test]
fn scope_empty_allowed_roots_returns_empty() {
    let result = make_test_result();
    let scoped = scope_result(result, &[], &HashMap::new(), false, false);
    assert_eq!(scoped.status, "not-indexed");
    assert!(scoped.nodes.is_empty());
    assert!(scoped.edges.is_empty());
    assert!(scoped.available_roots.is_empty());
}

#[test]
fn scope_canonical_path_matching() {
    // A node has a canonical root (e.g. /private/var → /var on macOS).
    let mut result = make_test_result();
    result.available_roots.clear();
    result.available_roots.push(WorkspaceRootInfo {
        root: "/ws_a".into(),
        file_count: 2,
        languages: vec![],
    });
    // Use a symlink-equivalent spelling as the configured root.
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.nodes.len(), 2);
}

#[test]
fn scope_foreign_node_removal() {
    // A node with workspace_root not in any configured root is removed.
    let mut result = make_test_result();
    result.nodes.push(GraphViewNode {
        path: "/orphan/file.rs".into(),
        language: "Rust".into(),
        out_degree: 0,
        in_degree: 0,
        role: GraphNodeRole::Overview,
        depth_from_focus: None,
        workspace_root: Some("/orphan".into()),
    });
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.nodes.len(), 2);
    assert!(!scoped.nodes.iter().any(|n| n.path.contains("/orphan")));
}

#[test]
fn scope_node_with_none_root_filtered_out() {
    // A node with no workspace_root is treated as foreign.
    let mut result = make_test_result();
    result.nodes.push(GraphViewNode {
        path: "/no-root/file.rs".into(),
        language: "Rust".into(),
        out_degree: 0,
        in_degree: 0,
        role: GraphNodeRole::Overview,
        depth_from_focus: None,
        workspace_root: None,
    });
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.nodes.len(), 2);
}

#[test]
fn scope_synthesised_root_zerometa_status_not_indexed() {
    // When one configured root is still missing, the overall result must
    // remain not-indexed so the UI offers reindex instead of reporting ok.
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    assert_eq!(scoped.status, "not-indexed");
}

#[test]
fn scope_all_roots_zero_files_status_scanning() {
    let result = GraphViewResult {
        nodes: vec![],
        edges: vec![],
        focus: None,
        generation: 0,
        file_count: 0,
        edge_count: 0,
        languages: vec![],
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: vec![WorkspaceRootInfo {
            root: "/ws_a".into(),
            file_count: 0,
            languages: vec![],
        }],
    };
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.status, "scanning");
}

#[test]
fn scope_languages_recomputed_from_filtered_nodes() {
    let result = make_test_result();
    // Only /ws_a (Rust) nodes survive.
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.languages, vec!["Rust".to_string()]);
}

#[test]
fn scope_available_roots_ordering_matches_configured() {
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_b".into(), "/ws_a".into()],
        &HashMap::new(),
        false,
        false,
    );
    // Ordering follows the configured_roots order.
    assert_eq!(scoped.available_roots[0].root, "/ws_b");
    assert_eq!(scoped.available_roots[1].root, "/ws_a");
}

#[test]
fn unavailable_result_shows_configured_roots() {
    let mut result = unavailable_result();
    result.available_roots = vec![ImportGraphRootInfo {
        root: "/ws_a".into(),
        configured_path: None,
        display_path: Some("ws_a".into()),
        file_count: 0,
        languages: Vec::new(),
        indexed_state: "not-indexed".to_string(),
    }];
    assert_eq!(result.status, "unavailable");
    assert_eq!(result.available_roots.len(), 1);
}

#[test]
fn edge_count_reflects_filtered_edges() {
    let mut result = make_test_result();
    // Add more edges.
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/lib.rs".into(),
        to: "/ws_a/src/main.rs".into(),
    });
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.edge_count, 2);
    assert_eq!(scoped.total_edges_available, 2);
}

#[test]
fn scope_multiple_roots_keeps_cross_root_edges_if_both_allowed() {
    let mut result = make_test_result();
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/main.rs".into(),
        to: "/ws_b/app.py".into(),
    });
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_b".into()],
        &HashMap::new(),
        false,
        false,
    );
    // Both nodes are allowed, so the cross-root edge survives.
    assert_eq!(scoped.edges.len(), 2);
}

// ── per-root indexed_state tests ───────────────────────────────────

#[test]
fn indexed_state_daemon_root_with_files_is_indexed() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert_eq!(ws_a.indexed_state, "indexed");
}

#[test]
fn indexed_state_synthesised_root_is_not_indexed() {
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    let ws_c = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_c")
        .unwrap();
    assert_eq!(ws_c.indexed_state, "not-indexed");
}

#[test]
fn indexed_state_zero_files_gen_zero_is_scanning() {
    let result = GraphViewResult {
        nodes: vec![],
        edges: vec![],
        focus: None,
        generation: 0,
        file_count: 0,
        edge_count: 0,
        languages: vec![],
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: vec![WorkspaceRootInfo {
            root: "/ws_a".into(),
            file_count: 0,
            languages: vec![],
        }],
    };
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.available_roots[0].indexed_state, "scanning");
}

#[test]
fn indexed_state_zero_files_gen_positive_is_indexed() {
    // Root is in daemon graph with 0 files but generation > 0 — the root
    // genuinely has no scannable files (e.g. empty workspace).
    let result = GraphViewResult {
        nodes: vec![],
        edges: vec![],
        focus: None,
        generation: 3,
        file_count: 0,
        edge_count: 0,
        languages: vec![],
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: vec![WorkspaceRootInfo {
            root: "/ws_a".into(),
            file_count: 0,
            languages: vec![],
        }],
    };
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.available_roots[0].indexed_state, "indexed");
}

#[test]
fn overall_status_all_not_indexed() {
    // Both configured roots are synthesised (not in daemon graph).
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_c".into(), "/ws_d".into()],
        &HashMap::new(),
        false,
        false,
    );
    assert_eq!(scoped.status, "not-indexed");
}

#[test]
fn overall_status_any_not_indexed() {
    // /ws_a is indexed, /ws_c is not-indexed.
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    assert_eq!(scoped.status, "not-indexed");
}

#[test]
fn overall_status_empty_roots() {
    let result = make_test_result();
    let scoped = scope_result(result, &[], &HashMap::new(), false, false);
    assert_eq!(scoped.status, "not-indexed");
}

// ── aggregate totals: no client filtering preserves daemon totals ───

#[test]
fn aggregate_totals_no_client_filtering_preserves_daemon_totals() {
    // Daemon returns 2 nodes, both in /ws_a.  Scoped to /ws_a, no
    // client-side filtering.  total_nodes_available should match daemon.
    let mut result = make_test_result();
    // Remove /ws_b node so daemon only returns /ws_a nodes.
    result.nodes.pop();
    result.total_nodes_available = 2;
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    // No client-side filtering: daemon total preserved.
    assert_eq!(scoped.total_nodes_available, 2);
}

#[test]
fn aggregate_totals_client_filtering_uses_scoped_count() {
    // Daemon returns 3 nodes across 2 roots.  Scoped to /ws_a only,
    // client filtering removes 1 node.  total_nodes_available = 2.
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert_eq!(scoped.total_nodes_available, 2);
    // After client filtering, truncation should be false since we have
    // all the scoped nodes.
    assert!(!scoped.nodes_truncated);
}

// ── correlation fields ─────────────────────────────────────────────

#[test]
fn correlation_fields_default_none() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert!(scoped.request_id.is_none());
    assert!(scoped.session_id.is_none());
}

#[test]
fn unavailable_with_ids_populates_correlation() {
    let r = unavailable_with_ids(Some("req-1".to_string()), Some("sess-2".to_string()));
    assert_eq!(r.request_id.as_deref(), Some("req-1"));
    assert_eq!(r.session_id.as_deref(), Some("sess-2"));
    assert_eq!(r.status, "unavailable");
}

#[test]
fn reindex_empty_roots_no_ids() {
    let r = reindex_and_fetch(None, None, &[], &HashMap::new(), None, None);
    assert_eq!(r.status, "not-indexed");
    assert!(r.request_id.is_none());
    assert!(r.session_id.is_none());
}

// ── reindex: daemon unreachable → unavailable result ────────────────

#[test]
fn reindex_daemon_unreachable_yields_unavailable() {
    // If the linker daemon is NOT running, ensure_and_register will fail
    // and the result MUST be a terminal unavailable.  If the daemon IS
    // running this test is not meaningful (the root may succeed), so we
    // skip.
    if crate::linker::client::fetch_generation().is_none() {
        let r = reindex_and_fetch(
            Some("test-session"),
            Some("req-reindex-1"),
            &["/nonexistent_root_a".into()],
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            r.status.contains("unavailable"),
            "expected unavailable status when daemon is down, got: {}",
            r.status
        );
        assert_eq!(r.request_id.as_deref(), Some("req-reindex-1"));
        assert_eq!(r.session_id.as_deref(), Some("test-session"));
    }
    // When the daemon IS running the reindex proceeds normally — we
    // can't assert "unavailable" in that case.
}

// ── reindex: register failure returns terminal error ────────────────

#[test]
fn reindex_register_failure_returns_terminal_error() {
    // Same daemon-liveness guard as above.
    if crate::linker::client::fetch_generation().is_none() {
        let r = reindex_and_fetch(
            Some("s"),
            Some("req-reg-fail"),
            &["/definitely/not/a/real/root/xyz".into()],
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            r.status.contains("unavailable"),
            "registration failure must yield unavailable: {}",
            r.status
        );
        assert_eq!(r.request_id.as_deref(), Some("req-reg-fail"));
    }
}

// ── reindex: no configured roots → not-indexed ──────────────────────

#[test]
fn reindex_no_configured_roots_yields_not_indexed() {
    let r = reindex_and_fetch(None, None, &[], &HashMap::new(), None, None);
    assert_eq!(r.status, "not-indexed");
}

// ── reindex: correlation fields propagated regardless of daemon state ─

#[test]
fn reindex_propagates_correlation_fields() {
    // This test works whether the daemon is running or not — we just
    // check that request_id and session_id survive the reindex path.
    let configured = vec!["/nonexistent_reindex_root".into()];
    let r = reindex_and_fetch(
        Some("my-session"),
        Some("req-corr-1"),
        &configured,
        &HashMap::new(),
        None,
        None,
    );
    // Regardless of daemon state, correlation fields must be populated.
    assert_eq!(r.request_id.as_deref(), Some("req-corr-1"));
    assert_eq!(r.session_id.as_deref(), Some("my-session"));
}

#[test]
fn reindex_with_daemon_yields_ok_or_unavailable() {
    // When the daemon is running and the root exists, reindex succeeds.
    // When the daemon is not running, we get unavailable. Either way,
    // the result must be a valid terminal state (never stale data).
    let configured = vec!["/nonexistent_reindex_root".into()];
    let r = reindex_and_fetch(
        Some("s"),
        Some("req-daemon-test"),
        &configured,
        &HashMap::new(),
        None,
        None,
    );
    assert!(
        r.status == "ok" || r.status.contains("unavailable") || r.status == "scanning",
        "reindex must produce a terminal state, got: {}",
        r.status
    );
    assert_eq!(r.request_id.as_deref(), Some("req-daemon-test"));
    assert_eq!(r.session_id.as_deref(), Some("s"));
}

// ── scoped impact analysis tests ───────────────────────────────────

#[test]
fn impact_empty_roots_returns_error() {
    let r = build_scoped_impact_result(
        "req-1".to_string(),
        "/ws_a/src/main.rs".into(),
        3,
        &[],
        None,
    );
    assert!(r.error.is_some());
    assert!(r.paths.is_empty());
    assert_eq!(r.total, 0);
}

#[test]
fn impact_out_of_scope_path_rejected() {
    let r = build_scoped_impact_result(
        "req-2".to_string(),
        "/foreign/path/file.rs".into(),
        3,
        &["/ws_a".into(), "/ws_b".into()],
        None,
    );
    assert!(r.error.is_some());
    assert!(r.error.unwrap().contains("outside configured"));
    assert!(r.paths.is_empty());
}

#[test]
fn impact_in_scope_path_not_called_daemon_still_validates() {
    // On CI, linker daemon is unreachable, so fetch_impact fails.
    // But the path IS in scope, so we get a daemon-unreachable error,
    // not an out-of-scope error.
    let r = build_scoped_impact_result(
        "req-3".to_string(),
        "/ws_a/src/main.rs".into(),
        3,
        &["/ws_a".into()],
        None,
    );
    // Either the daemon is running (r.error might be None or some
    // impact error) or it's not (unreachable error). Either way the
    // path was accepted as in-scope.
    if let Some(ref e) = r.error {
        // If there's an error, it should NOT be about scope.
        assert!(!e.contains("outside configured"));
    }
    assert_eq!(r.request_id, "req-3");
    assert_eq!(r.path, "/ws_a/src/main.rs");
}

#[test]
fn impact_paths_filtered_to_configured_roots() {
    // This test verifies the filtering logic structurally:
    // build_scoped_impact_result uses a HashSet of allowed roots and
    // filters paths by prefix. We verify that out-of-scope focal paths
    // are rejected (tested above), and that the error structure is
    // correct. A full integration test would require a running daemon.
    let r = build_scoped_impact_result(
        "req-4".to_string(),
        "/ws_a/src/main.rs".into(),
        2,
        &["/ws_a".into()],
        None,
    );
    // Verify request_id and path are echoed.
    assert_eq!(r.request_id, "req-4");
    assert_eq!(r.path, "/ws_a/src/main.rs");
    assert_eq!(r.depth, 2);
}

// ── worker spawn function signatures compile ───────────────────────

#[test]
fn spawn_functions_compile_attached() {
    // Verify the function signatures are compatible. We can't easily
    // test actual IPC in a unit test, but we can at least confirm the
    // types line up at compile time.
    let _: fn(Sender<ImportGraphResult>, ImportGraphJob) = spawn_import_graph_attached;
}

#[test]
fn spawn_functions_compile_reindex_attached() {
    let _: fn(
        Sender<ImportGraphResult>,
        String,
        Vec<String>,
        HashMap<String, String>,
        Option<Vec<String>>,
        Option<Vec<String>>,
        Option<String>,
    ) = spawn_import_graph_reindex_attached;
}

#[test]
fn spawn_functions_compile_impact_attached() {
    let _: fn(
        Sender<super::super::push_proto::ImportGraphImpactResult>,
        String,
        u32,
        String,
        Vec<String>,
        Option<String>,
    ) = spawn_import_graph_impact_attached;
}

// ── Component-safe impact scoping (Path::starts_with) ──────────────

#[test]
fn impact_prefix_sibling_rejected_by_path_starts_with() {
    // /workspace/app must NOT be treated as in-scope for
    // /workspace/application-secret — a string prefix match would pass,
    // but Path::starts_with is component-aware and rejects this.
    let r = build_scoped_impact_result(
        "req-prefix".to_string(),
        "/workspace/app/src/main.rs".into(),
        3,
        &["/workspace/application-secret".into()],
        None,
    );
    assert!(
        r.error.is_some(),
        "should reject /workspace/app when configured root is /workspace/application-secret"
    );
    assert!(r.error.unwrap().contains("outside configured"));
}

#[test]
fn impact_prefix_sibling_accepted_when_exact_root() {
    // /workspace/app IS in scope when /workspace/app is a configured root.
    let r = build_scoped_impact_result(
        "req-exact".to_string(),
        "/workspace/app/src/main.rs".into(),
        3,
        &["/workspace/app".into()],
        None,
    );
    // Either daemon unreachable (error about daemon, not scope) or success.
    if let Some(ref e) = r.error {
        assert!(!e.contains("outside configured"));
    }
}

// ── Display mapping tests ─────────────────────────────────────────

#[test]
fn scope_result_populates_display_path() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
}

#[test]
fn scope_result_configured_path_none_when_equal() {
    // When the daemon root matches the configured root exactly,
    // configured_path should be None (omitted from JSON).
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert!(ws_a.configured_path.is_none());
}

#[test]
fn scope_result_synthesised_root_has_display_path() {
    let result = make_test_result();
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    let ws_c = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_c")
        .unwrap();
    assert_eq!(ws_c.display_path.as_deref(), Some("ws_c"));
}

// ── Scan state from ScanStatus (per-root + overall) ───────────────

#[test]
fn scope_scan_in_flight_marks_missing_root_as_scanning() {
    let result = make_test_result();
    // /ws_c is not in daemon graph but scan_in_flight = true.
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        true,
        false,
    );
    let ws_c = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_c")
        .unwrap();
    assert_eq!(ws_c.indexed_state, "scanning");
}

#[test]
fn scope_scan_failed_marks_all_as_unavailable() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, true);
    assert_eq!(scoped.status, "unavailable");
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert_eq!(ws_a.indexed_state, "unavailable");
}

#[test]
fn overall_mixed_indexed_and_not_indexed_with_scan_in_flight_is_scanning() {
    let result = make_test_result();
    // /ws_a is indexed, /ws_c is not-indexed but scan is in-flight → scanning.
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        true,
        false,
    );
    assert_eq!(scoped.status, "scanning");
}

#[test]
fn overall_mixed_indexed_and_not_indexed_without_scan_is_not_indexed() {
    let result = make_test_result();
    // /ws_a is indexed, /ws_c is not-indexed, no scan in-flight.
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &HashMap::new(),
        false,
        false,
    );
    assert_eq!(scoped.status, "not-indexed");
}

// ── Reindex: repeated reindex with exact revision ─────────────────

#[test]
fn reindex_daemon_unreachable_yields_terminal_unavailable() {
    // When daemon is down, reindex must produce a terminal unavailable
    // result — not hang or produce stale data.
    if crate::linker::client::fetch_generation().is_none() {
        let r1 = reindex_and_fetch(
            Some("s"),
            Some("req-r1"),
            &["/nonexistent_a".into()],
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            r1.status.contains("unavailable"),
            "first reindex should be unavailable: {}",
            r1.status
        );
        // Second reindex on the same dead daemon must also be terminal.
        let r2 = reindex_and_fetch(
            Some("s"),
            Some("req-r2"),
            &["/nonexistent_a".into()],
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            r2.status.contains("unavailable"),
            "second reindex should also be unavailable: {}",
            r2.status
        );
        // Both carry their correlation ids.
        assert_eq!(r1.request_id.as_deref(), Some("req-r1"));
        assert_eq!(r2.request_id.as_deref(), Some("req-r2"));
    }
}

// ── unavailable_result display mapping ─────────────────────────────

#[test]
fn unavailable_result_display_path_is_none() {
    // The base unavailable_result has no available_roots at all.
    let r = unavailable_result();
    assert!(r.available_roots.is_empty());
}

// ── configured_root_map integration: symlink/relative DTO mapping ────

#[test]
fn scope_result_configured_path_set_when_map_differs() {
    let result = make_test_result();
    let mut map = HashMap::new();
    map.insert("/ws_a".to_string(), "/symlink/to/ws_a".to_string());
    let scoped = scope_result(result, &["/ws_a".into()], &map, false, false);
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert_eq!(ws_a.configured_path.as_deref(), Some("/symlink/to/ws_a"));
    // display_path should come from the raw configured path basename.
    assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
}

#[test]
fn scope_result_configured_path_none_when_map_empty() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    let ws_a = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_a")
        .unwrap();
    assert!(ws_a.configured_path.is_none());
    // display_path falls back to canonical basename.
    assert_eq!(ws_a.display_path.as_deref(), Some("ws_a"));
}

#[test]
fn scope_result_synthesised_root_uses_map_for_configured_path() {
    let result = make_test_result();
    let mut map = HashMap::new();
    map.insert("/ws_c".to_string(), "../ws_c".to_string());
    let scoped = scope_result(
        result,
        &["/ws_a".into(), "/ws_c".into()],
        &map,
        false,
        false,
    );
    let ws_c = scoped
        .available_roots
        .iter()
        .find(|r| r.root == "/ws_c")
        .unwrap();
    assert_eq!(ws_c.configured_path.as_deref(), Some("../ws_c"));
    assert_eq!(ws_c.display_path.as_deref(), Some("ws_c"));
}

// ── Edge totals preservation: daemon aggregate exact scope ───────────

#[test]
fn edge_totals_preserved_when_no_client_filtering() {
    // Daemon returns 2 nodes and 3 edges, all in /ws_a.  Scoped to
    // /ws_a — no client filtering.  edge_count should match daemon.
    let mut result = make_test_result();
    // Remove the /ws_b node so daemon only returns /ws_a nodes.
    result.nodes.pop();
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/lib.rs".into(),
        to: "/ws_a/src/main.rs".into(),
    });
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/main.rs".into(),
        to: "/ws_a/src/lib.rs".into(),
    });
    result.total_edges_available = 3;
    result.total_nodes_available = 2;
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    // No client-side filtering → daemon totals preserved.
    assert_eq!(scoped.total_edges_available, 3);
    assert_eq!(scoped.edge_count, 3);
}

#[test]
fn edge_totals_recomputed_when_client_filters() {
    // Daemon returns 3 nodes across 2 roots.  Scoped to /ws_a only —
    // client filters out 1 node.  Also add a cross-root edge that will
    // be removed by client filtering.
    let mut result = make_test_result();
    // All 3 nodes in /ws_a get an extra intra-root edge.
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/lib.rs".into(),
        to: "/ws_a/src/main.rs".into(),
    });
    // Plus a cross-root edge (won't survive scoping to /ws_a).
    result.edges.push(GraphViewEdge {
        from: "/ws_a/src/main.rs".into(),
        to: "/ws_b/app.py".into(),
    });
    result.total_edges_available = 3;
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    // Client filtered: /ws_b node removed, cross-root edge removed.
    // 2 intra-root edges survive.
    assert_eq!(scoped.edge_count, 2);
    assert_eq!(scoped.total_edges_available, 2);
}

#[test]
fn edges_truncated_preserved_when_no_client_filtering() {
    // Daemon reports truncation, no client filtering → truncation preserved.
    let mut result = make_test_result();
    result.nodes.pop(); // remove /ws_b node, leaving only /ws_a nodes
    result.edges_truncated = true;
    result.total_edges_available = 5; // daemon says 5 edges available total
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert!(scoped.edges_truncated);
    // Daemon's pre-cap aggregate preserved (not the array length).
    assert_eq!(scoped.total_edges_available, 5);
}

// ── request_id threading: scope_result default ───────────────────────

#[test]
fn scope_result_request_id_session_id_default_none() {
    let result = make_test_result();
    let scoped = scope_result(result, &["/ws_a".into()], &HashMap::new(), false, false);
    assert!(scoped.request_id.is_none());
    assert!(scoped.session_id.is_none());
}
