//! `graph_query` tool — query the linker daemon's import graph.

use super::{Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};

pub struct GraphQuery;

impl Tool for GraphQuery {
    fn name(&self) -> &'static str {
        "graph_query"
    }
    fn description(&self) -> &'static str {
        "Query the code import graph for a project. Find dependencies, dependents, impact sets, and project summaries."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["dependencies", "dependents", "impact", "neighborhood", "summary", "rescan"],
                    "description": "What to query."
                },
                "path": {
                    "type": "string",
                    "description": "File path to query (required for dependencies/dependents/impact/neighborhood)."
                },
                "depth": {
                    "type": "integer",
                    "description": "Max depth for impact traversal (default 1, max 3)."
                }
            },
            "required": ["action"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, args: &Value) -> Result<String> {
        // Get action
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required argument 'action'"))?;

        // Get path (required for most actions)
        let path = args.get("path").and_then(|v| v.as_str());

        // Connect to the linker daemon
        let sock_path = crate::model::store::linker_daemon_sock_path()
            .map_err(|e| anyhow::anyhow!("cannot resolve linker socket: {e}"))?;

        let mut stream = match crate::ipc::SyncIpcStream::connect(&sock_path) {
            Ok(s) => s,
            Err(e) => {
                return Ok(format!(
                    "linker daemon not running (connect failed: {e}). \
                     The graph may not be ready yet."
                ));
            }
        };

        // Build the query
        use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest, LinkerResponse};

        let query = match action {
            "dependencies" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for dependencies"))?;
                LinkerQuery::Dependencies {
                    path: p.to_string(),
                }
            }
            "dependents" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for dependents"))?;
                LinkerQuery::Dependents {
                    path: p.to_string(),
                }
            }
            "impact" => {
                let p =
                    path.ok_or_else(|| anyhow::anyhow!("'path' is required for impact"))?;
                let depth = args
                    .get("depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                LinkerQuery::Impact {
                    path: p.to_string(),
                    depth: Some(depth.min(3)),
                }
            }
            "neighborhood" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for neighborhood"))?;
                LinkerQuery::Neighborhood {
                    path: p.to_string(),
                }
            }
            "summary" => LinkerQuery::Status,
            "rescan" => LinkerQuery::Rescan,
            _ => {
                return Ok(format!(
                    "unknown action: {action}. Use: dependencies, dependents, impact, neighborhood, summary, rescan"
                ))
            }
        };

        let req = LinkerRequest::Query(query);
        let payload =
            serde_json::to_vec(&req).map_err(|e| anyhow::anyhow!("serialize request: {e}"))?;

        // Write request (length-prefixed frame)
        use std::io::{Read, Write};
        let prefix = (payload.len() as u32).to_be_bytes();
        stream
            .write_all(&prefix)
            .map_err(|e| anyhow::anyhow!("write prefix: {e}"))?;
        stream
            .write_all(&payload)
            .map_err(|e| anyhow::anyhow!("write payload: {e}"))?;
        stream
            .flush()
            .map_err(|e| anyhow::anyhow!("flush: {e}"))?;

        // Read response (blocking frame reassembly)
        let mut frame_reader = crate::ipc::frame::FrameReader::new();
        let frame = loop {
            // Check buffered data first
            if let Some(f) = frame_reader
                .next_frame()
                .map_err(|e| anyhow::anyhow!("frame reassembly: {e}"))?
            {
                break f;
            }
            let mut buf = [0u8; 64 * 1024];
            let n = stream
                .read(&mut buf)
                .map_err(|e| anyhow::anyhow!("read response: {e}"))?;
            if n == 0 {
                return Ok("linker daemon closed connection".into());
            }
            frame_reader.push(&buf[..n]);
        };

        let resp: LinkerResponse = serde_json::from_slice(&frame)
            .map_err(|e| anyhow::anyhow!("deserialize response: {e}"))?;

        // Format response
        match resp {
            LinkerResponse::PathList { paths, total } => {
                let mut out = if paths.len() < total {
                    format!("({} of {} results)\n", paths.len(), total)
                } else {
                    String::new()
                };
                for p in &paths {
                    out.push_str(p);
                    out.push('\n');
                }
                if paths.is_empty() {
                    out = "no results".into();
                }
                Ok(out)
            }
            LinkerResponse::Summary {
                text,
                generation,
                file_count,
                edge_count,
                languages,
            } => {
                if text.is_empty() {
                    Ok(format!(
                        "graph: generation={generation}, {file_count} files, {edge_count} edges, langs=[{}]",
                        languages.join(", ")
                    ))
                } else {
                    Ok(text)
                }
            }
            LinkerResponse::Ack => Ok("acknowledged".into()),
            LinkerResponse::Error(e) => Ok(format!("linker error: {e}")),
            _ => Ok("unexpected response type".into()),
        }
    }
}
