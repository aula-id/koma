//! Model-facing security meta-tools.
//!
//! Instead of advertising every `sec_*` schema on each turn, the parent exposes
//! four fixed tools. Concrete toolkit entries are discovered via `sec_catalog`
//! and executed via `sec_run` (inner name still goes through lane/scope gates).

use crate::dto::openrouter::request::{ToolDef, ToolFunctionDef};

/// Names advertised to the model when security mode is on (not Plan).
pub const META_TOOL_NAMES: &[&str] = &["sec_catalog", "sec_run", "sec_fs", "sec_runtime", "sec_lane"];

/// Meta tools that work before a lane is chosen: discovery, and choosing the lane.
pub const LANE_FREE: &[&str] = &["sec_catalog", "sec_lane"];

/// `true` for [`LANE_FREE`] tools.
pub fn is_lane_free(name: &str) -> bool {
    LANE_FREE.contains(&name)
}

/// `true` when `name` is one of the control-plane meta tools.
pub fn is_meta_tool(name: &str) -> bool {
    META_TOOL_NAMES.contains(&name)
}

fn object_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    let mut req = serde_json::Value::Array(
        required
            .iter()
            .map(|s| serde_json::Value::String((*s).into()))
            .collect(),
    );
    if required.is_empty() {
        req = serde_json::json!([]);
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": req,
        "additionalProperties": false,
    })
}

fn def(name: &str, description: &str, parameters: serde_json::Value) -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: ToolFunctionDef {
            name: name.into(),
            description: description.into(),
            parameters,
        },
    }
}

/// Fixed ToolDefs for the model tool list.
pub fn meta_tool_defs() -> Vec<ToolDef> {
    vec![
        def(
            "sec_lane",
            "Choose this session's cyber lane before using the security toolkit. \
research = local analysis only (no network, no target probes); \
red = may reach the network, probe in-scope targets and start listeners, no patches; \
blue = may reach the network and probe in-scope targets, may propose patches, no listeners; \
none = clear. Pick the narrowest lane the task needs. Takes effect for the next calls.",
            object_schema(
                serde_json::json!({
                    "lane": {
                        "type": "string",
                        "enum": ["research", "red", "blue", "none"],
                        "description": "Lane to switch to"
                    }
                }),
                &["lane"],
            ),
        ),
        def(
            "sec_catalog",
            "Browse the tools you can run right now: only tools inside the active guest images \
are listed. Start with op=categories (the default): each category with how many tools are ready. \
Then op=list with a category (or q to search) to see that category's tools. \
op=get returns one tool's full schema and how-to (requires name).",
            object_schema(
                serde_json::json!({
                    "op": {
                        "type": "string",
                        "enum": ["categories", "list", "get"],
                        "description": "categories (default) → list a category → get one tool"
                    },
                    "category": {
                        "type": "string",
                        "description": "Category name from op=categories; required for list unless q is given"
                    },
                    "name": {
                        "type": "string",
                        "description": "Concrete tool name for get (e.g. sec_http)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "page size for list (default 40, max 200)"
                    },
                    "cursor": {
                        "type": "integer",
                        "description": "list offset for pagination"
                    },
                    "q": {
                        "type": "string",
                        "description": "optional substring search over name/blurb/categories"
                    },
                    "lane": {
                        "type": "string",
                        "description": "optional lane filter (research|red|blue); default session lane"
                    }
                }),
                &[],
            ),
        ),
        def(
            "sec_run",
            "Execute one concrete security tool by name inside the isolation boundary. \
Discover names/schemas with sec_catalog first. Browser-class tools may run on the host; \
others prefer the koma-safety guest.",
            object_schema(
                serde_json::json!({
                    "name": {
                        "type": "string",
                        "description": "Concrete tool id (e.g. sec_hashid, sec_http)"
                    },
                    "args": {
                        "type": "object",
                        "description": "Arguments matching the tool's catalog schema",
                        "additionalProperties": true
                    }
                }),
                &["name"],
            ),
        ),
        def(
            "sec_fs",
            "Copy files into/out of the security guest or list guest paths. \
op=copy_in (host src → guest dest under /artifacts or /work), \
op=copy_out (guest src → host dest), op=ls (guest path). Boundary must be up.",
            object_schema(
                serde_json::json!({
                    "op": {
                        "type": "string",
                        "enum": ["copy_in", "copy_out", "ls"],
                        "description": "Filesystem sub-operation"
                    },
                    "src": { "type": "string", "description": "Source path" },
                    "dest": { "type": "string", "description": "Destination path" },
                    "path": { "type": "string", "description": "Guest path for ls (default artifacts)" }
                }),
                &["op"],
            ),
        ),
        def(
            "sec_runtime",
            "Control the koma-safety isolation boundary. \
op=status|health inspect; op=up start guest+proxy on an internal network (fail-closed); \
op=down tear down. Prefer sec_runtime up before heavy runs; sec_run may lazy-start. \
Network scope: op=net mode=scope targets=[host or host:port] asks the user to confirm \
those targets; once confirmed they are the scope every networked sec_run is checked against.",
            object_schema(
                serde_json::json!({
                    "op": {
                        "type": "string",
                        "enum": ["status", "up", "down", "health", "net", "rules"],
                        "description": "Runtime sub-operation"
                    },
                    "targets": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Scope targets for up/net, as host or host:port (no scheme, no path). New targets need the user's confirmation"
                    },
                    "exclusions": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional exclusion cells for up/net"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["off", "scope"],
                        "description": "For op=net: off=deny-all egress, scope=use targets"
                    }
                }),
                &["op"],
            ),
        ),
    ]
}

#[cfg(test)]
#[path = "meta_test.rs"]
mod tests;
