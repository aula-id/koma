//! Tool-call box-label mapping + quote-less signature formatting for the
//! transcript. Split out of [`super::transcript`] for file size; no behaviour
//! change — both `tool_box_label` and `format_tool_signature` keep their
//! existing `pub(crate)` visibility and are re-exported from `transcript`
//! so `crate::view::chat::transcript::{tool_box_label, format_tool_signature}`
//! call sites (the GUI push-projection in `app::runtime::client::render`)
//! keep resolving unchanged.

use super::helpers::truncate_chars;

/// Map a tool's function name to a short box LABEL, or `None` when the tool's
/// result should NOT be boxed (terse-status tools keep the compact one-liner).
/// MCP (`mcp__…`) and security (`sec_…`) tool families collapse to one label each.
pub(crate) fn tool_box_label(name: &str) -> Option<&'static str> {
    if name.starts_with("mcp__") {
        return Some("mcp");
    }
    if name.starts_with("sec_") {
        return Some("sec");
    }
    Some(match name {
        "bash" => "bash",
        "read" => "read",
        "grep" => "grep",
        "glob" => "glob",
        "dir_list" => "dir",
        "git_operator" | "git_cred" | "git_worktree" => "git",
        "web_fetch" | "web_search" => "web",
        "recall" => "memory",
        _ => return None, // None → not boxed, keep the single-line rendering
    })
}

/// Render a tool call as a clean, quote-less signature for the transcript header:
/// `bash(ls src-agent/)`, `git_operator(log --oneline -5)`, `grep(fn main)`,
/// `read(Cargo.toml)`. Display-only; the real JSON sent to the model is untouched.
/// Unmapped tools (mcp__*, sec_*, future) fall back to their object values, or the
/// raw args if parsing fails.
pub(crate) fn format_tool_signature(name: &str, args_json: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
    let inner = tool_signature_inner(name, &v).unwrap_or_else(|| generic_inner(&v, args_json));
    // Collapse newlines/runs of whitespace so the header stays one line, then cap.
    let flat = inner.split_whitespace().collect::<Vec<_>>().join(" ");
    let capped = truncate_chars(&flat, 60);
    format!("{name}({capped})")
}

/// The salient argument(s) for a known tool, positional and quote-less. `None`
/// means "not specially mapped" → caller uses the generic fallback.
fn tool_signature_inner(name: &str, v: &serde_json::Value) -> Option<String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let arr = |k: &str| {
        v.get(k).and_then(|x| x.as_array()).map(|a| {
            a.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join(" ")
        })
    };
    match name {
        "bash" => s("command"),
        "git_operator" => arr("args"),
        "git_cred" => {
            let action = s("action")?;
            Some(match s("key") {
                Some(k) => format!("{action} {k}"),
                None => action,
            })
        }
        "git_worktree" => {
            let action = s("action")?;
            let extra = s("path").or_else(|| s("name")).or_else(|| s("branch"));
            Some(match extra {
                Some(e) => format!("{action} {e}"),
                None => action,
            })
        }
        "read" | "write" | "edit" | "delete" | "cd" => s("path"),
        "dir_list" => s("path").or_else(|| arr("paths")),
        "grep" | "glob" => s("pattern"),
        "web_fetch" | "web_download" => s("url"),
        "web_search" => s("query"),
        "remember" => s("slug").or_else(|| s("description")),
        "forget" | "recall" => s("slug"),
        "task" => {
            let agent = s("agent")?;
            Some(match s("prompt") {
                Some(p) => format!("{agent}: {p}"),
                None => agent,
            })
        }
        "checklist" => v
            .get("todos")
            .and_then(|x| x.as_array())
            .map(|a| format!("{} todos", a.len())),
        "bash_output" | "bash_kill" => s("job_id"),
        "task_output" | "task_kill" => v.get("id").map(|x| match x {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }),
        "task_send" => {
            let id = v.get("agent_id").map(|x| match x {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })?;
            Some(match s("message") {
                Some(m) => format!("#{id}: {m}"),
                None => format!("#{id}"),
            })
        }
        _ => None,
    }
}

/// Generic fallback for unmapped tools: the object's scalar/array values joined,
/// or the raw args string if it isn't a JSON object / failed to parse.
fn generic_inner(v: &serde_json::Value, raw: &str) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let parts: Vec<String> = map
                .values()
                .filter_map(|val| match val {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::Array(a) => Some(
                        a.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join(" "),
                    ),
                    _ => None,
                })
                .collect();
            if parts.is_empty() { raw.to_string() } else { parts.join(", ") }
        }
        _ => raw.to_string(),
    }
}
