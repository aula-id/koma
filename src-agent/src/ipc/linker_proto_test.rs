use super::*;

/// Verify that old protocol JSON (without newer optional fields) still
/// deserializes correctly — backward compatibility.
#[test]
fn visualization_request_backward_compat() {
    let json = r#"{"path":null,"depth":2,"direction":"both","max_nodes":100,"max_edges":100}"#;
    let req: VisualizationRequest = serde_json::from_str(json).unwrap();
    assert!(req.filter_roots.is_none());
    assert!(req.filter_languages.is_none());
}

#[test]
fn graph_view_node_backward_compat() {
    // Old node without workspace_root.
    let json = r#"{"path":"/a.rs","language":"Rust","out_degree":1,"in_degree":0,"role":"Focus","depth_from_focus":0}"#;
    let node: GraphViewNode = serde_json::from_str(json).unwrap();
    assert!(node.workspace_root.is_none());
}

#[test]
fn graph_view_result_backward_compat() {
    // Old result without available_roots.
    let json = r#"{"nodes":[],"edges":[],"focus":null,"generation":1,"file_count":0,"edge_count":0,"languages":[],"nodes_truncated":false,"edges_truncated":false,"total_nodes_available":0,"total_edges_available":0}"#;
    let result: GraphViewResult = serde_json::from_str(json).unwrap();
    assert!(result.available_roots.is_empty());
}

#[test]
fn edit_context_result_roundtrip() {
    let ctx = EditContextResult {
        imports: vec!["/a.rs".into()],
        imported_by: vec!["/b.rs".into()],
        transitive_dependents_count: 1,
        entry_point_count: 0,
        cross_boundary_deps: vec![],
        is_entry_point: false,
        is_leaf: false,
        unresolved_imports: vec!["serde".into()],
        related_tests: vec![],
        related_configs: vec![],
    };
    let json = serde_json::to_string(&ctx).unwrap();
    let back: EditContextResult = serde_json::from_str(&json).unwrap();
    assert_eq!(ctx.imports, back.imports);
    assert_eq!(ctx.unresolved_imports, back.unresolved_imports);
}
