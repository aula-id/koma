//! Model-facing security meta-tools.
//!
//! Instead of advertising every `sec_*` schema on each turn, the parent exposes
//! four fixed tools. Concrete toolkit entries are discovered via `sec_catalog`
//! and executed via `sec_run` (inner name still goes through lane/scope gates).

use crate::dto::openrouter::request::{ToolDef, ToolFunctionDef};

/// Names advertised to the model when security mode is on (not Plan).
pub const META_TOOL_NAMES: &[&str] = &["sec_catalog", "sec_run", "sec_fs", "sec_runtime"];

/// `true` when `name` is one of the four control-plane meta tools.
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

/// Fixed ToolDefs for the model tool list (≤4 entries).
pub fn meta_tool_defs() -> Vec<ToolDef> {
    vec![
        def(
            "sec_catalog",
            "Browse the security toolkit without dumping every schema into the prompt. \
op=categories lists domains; op=list lists tools (optional category; filter defaults to ready); \
op=get returns one tool's full schema and how-to (requires name).",
            object_schema(
                serde_json::json!({
                    "op": {
                        "type": "string",
                        "enum": ["categories", "list", "get"],
                        "description": "Catalog sub-operation"
                    },
                    "category": {
                        "type": "string",
                        "description": "Domain filter for list (web, crypto, pwn, web-re, …)"
                    },
                    "name": {
                        "type": "string",
                        "description": "Concrete tool name for get (e.g. sec_http)"
                    },
                    "filter": {
                        "type": "string",
                        "enum": ["all", "ready"],
                        "description": "list filter: default ready; pass all to include not-ready"
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
                &["op"],
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
op=down tear down. Prefer sec_runtime up before heavy runs; sec_run may lazy-start.",
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
                        "description": "Optional egress allowlist cells for up (host:ports)"
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
