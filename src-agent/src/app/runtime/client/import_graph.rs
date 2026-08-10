//! Off-thread worker for the import-graph visualization `HostCtl` handler.
//!
//! Calls `linker::client::fetch_graph_view()` on a std::thread (blocking IPC),
//! sends the result back over an `mpsc` channel to be drained by `push_loop`.

use std::sync::mpsc::Sender;

/// Workspace-relative import-graph view result for the GUI.
/// Matches the fields of `GraphViewResult` but is self-contained (no linker
/// daemon types leak into the GUI push contract).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphResult {
    pub nodes: Vec<ImportGraphNode>,
    pub edges: Vec<ImportGraphEdge>,
    pub focus: Option<String>,
    pub generation: u64,
    pub file_count: usize,
    pub edge_count: usize,
    pub languages: Vec<String>,
    pub nodes_truncated: bool,
    pub edges_truncated: bool,
    pub total_nodes_available: usize,
    pub total_edges_available: usize,
    pub available_roots: Vec<ImportGraphRootInfo>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphNode {
    pub path: String,
    pub language: String,
    pub out_degree: usize,
    pub in_degree: usize,
    pub role: String,
    pub depth_from_focus: Option<u32>,
    pub workspace_root: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ImportGraphEdge {
    pub from: String,
    pub to: String,
}

/// Per-root workspace metadata for filter pickers.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphRootInfo {
    pub root: String,
    pub file_count: usize,
    pub languages: Vec<ImportGraphLangCount>,
}

/// Language with count for per-root breakdown.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGraphLangCount {
    pub name: String,
    pub count: usize,
}

/// Convert a daemon `GraphViewResult` into the workspace-relative GUI DTO.
pub fn from_daemon_result(result: crate::ipc::linker_proto::GraphViewResult) -> ImportGraphResult {
    ImportGraphResult {
        nodes: result
            .nodes
            .into_iter()
            .map(|n| ImportGraphNode {
                path: n.path,
                language: n.language,
                out_degree: n.out_degree,
                in_degree: n.in_degree,
                role: format!("{:?}", n.role),
                depth_from_focus: n.depth_from_focus,
                workspace_root: n.workspace_root,
            })
            .collect(),
        edges: result
            .edges
            .into_iter()
            .map(|e| ImportGraphEdge {
                from: e.from,
                to: e.to,
            })
            .collect(),
        focus: result.focus,
        generation: result.generation,
        file_count: result.file_count,
        edge_count: result.edge_count,
        languages: result.languages,
        nodes_truncated: result.nodes_truncated,
        edges_truncated: result.edges_truncated,
        total_nodes_available: result.total_nodes_available,
        total_edges_available: result.total_edges_available,
        available_roots: result
            .available_roots
            .into_iter()
            .map(|r| ImportGraphRootInfo {
                root: r.root,
                file_count: r.file_count,
                languages: r
                    .languages
                    .into_iter()
                    .map(|l| ImportGraphLangCount {
                        name: l.name,
                        count: l.count,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Spawn an off-thread `HostCtl::ImportGraph` worker (attached mode).
/// Calls `fetch_graph_view` on a std::thread, sends the result over the channel.
pub fn spawn_import_graph_attached(
    tx: Sender<ImportGraphResult>,
    path: Option<String>,
    depth: u32,
    direction: crate::ipc::linker_proto::GraphDirection,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
) {
    std::thread::spawn(move || {
        let req = crate::ipc::linker_proto::VisualizationRequest {
            path,
            depth,
            direction,
            max_nodes: 200,
            max_edges: 400,
            filter_roots,
            filter_languages,
        };
        let result = match crate::linker::client::fetch_graph_view(&req) {
            Some(r) => from_daemon_result(r),
            None => empty_result(),
        };
        let _ = tx.send(result);
    });
}

/// Spawn an off-thread `HostCtl::ImportGraph` worker (detached mode).
/// Calls `fetch_graph_view` on a std::thread, pushes the result directly.
pub fn spawn_import_graph(
    push: impl Fn(String) + Send + 'static,
    path: Option<String>,
    depth: u32,
    direction: crate::ipc::linker_proto::GraphDirection,
    filter_roots: Option<Vec<String>>,
    filter_languages: Option<Vec<String>>,
) {
    std::thread::spawn(move || {
        let req = crate::ipc::linker_proto::VisualizationRequest {
            path,
            depth,
            direction,
            max_nodes: 200,
            max_edges: 400,
            filter_roots,
            filter_languages,
        };
        let result = match crate::linker::client::fetch_graph_view(&req) {
            Some(r) => from_daemon_result(r),
            None => empty_result(),
        };
        let env = super::push_proto::PushEnvelope::ImportGraph(result);
        super::render::emit(&push, &env);
    });
}

/// An empty result for when the linker daemon is unreachable.
fn empty_result() -> ImportGraphResult {
    ImportGraphResult {
        nodes: Vec::new(),
        edges: Vec::new(),
        focus: None,
        generation: 0,
        file_count: 0,
        edge_count: 0,
        languages: Vec::new(),
        nodes_truncated: false,
        edges_truncated: false,
        total_nodes_available: 0,
        total_edges_available: 0,
        available_roots: Vec::new(),
    }
}
