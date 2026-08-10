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
        "Query the code import graph for a project. Find dependencies, dependents, impact sets, and project summaries. \
         PREFERRED for dependency/impact analysis over grep or glob — it is instant and complete."
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
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        use crate::ipc::linker_proto::{LinkerQuery, LinkerRequest};

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required argument 'action'"))?;

        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| crate::linker::client::normalize_query_path(p, &ctx.workspaces));

        // Build the request.
        let req = match action {
            "dependencies" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for dependencies"))?;
                LinkerRequest::Query(LinkerQuery::Dependencies { path: p })
            }
            "dependents" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for dependents"))?;
                LinkerRequest::Query(LinkerQuery::Dependents { path: p })
            }
            "impact" => {
                let p =
                    path.ok_or_else(|| anyhow::anyhow!("'path' is required for impact"))?;
                let depth = args
                    .get("depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                LinkerRequest::Query(LinkerQuery::Impact {
                    path: p,
                    depth: Some(depth.min(3)),
                })
            }
            "neighborhood" => {
                let p = path
                    .ok_or_else(|| anyhow::anyhow!("'path' is required for neighborhood"))?;
                LinkerRequest::Query(LinkerQuery::Neighborhood { path: p })
            }
            "summary" => LinkerRequest::Summary,
            "rescan" => LinkerRequest::Query(LinkerQuery::Rescan),
            _ => {
                return Ok(format!(
                    "unknown action: {action}. Use: dependencies, dependents, impact, neighborhood, summary, rescan"
                ))
            }
        };

        // Send via the persistent connection pool.
        use crate::ipc::linker_proto::LinkerResponse;
        let resp = match crate::linker::client::connect_and_send(&req) {
            Some(r) => r,
            None => {
                return Ok(
                    "linker daemon not running or unreachable. The graph may not be ready yet."
                        .into(),
                )
            }
        };

        // Format the response.
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
