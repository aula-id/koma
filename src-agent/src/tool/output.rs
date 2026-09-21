//! Post-process every tool result before it reaches the model.
//!
//! Large dumps (10k-line reads, chatty bash, fat MCP payloads) blow the
//! context window and force DRSS to cut the conversation — the model then
//! re-reads the same file and loops. Cap every result except sub-agent
//! reports, and warn when the exact same call is repeated.

use super::ToolCtx;
use crate::config::{MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Tools whose result is a sub-agent report (or a control message about one).
/// Those already have their own delivery cap; clipping them here would hide
/// the report the main agent is supposed to read.
pub fn is_subagent_output(name: &str) -> bool {
    matches!(name, "task" | "task_output" | "task_send" | "task_kill")
}

/// Per-session counter of exact tool calls (name + canonical arguments).
#[derive(Default)]
pub struct CallTrack {
    counts: Mutex<HashMap<String, u32>>,
}

impl CallTrack {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Increment and return the new count for this exact name+args pair.
    /// Sub-agent tools are not tracked.
    pub fn hit(&self, name: &str, args: &Value) -> u32 {
        if is_subagent_output(name) {
            return 1;
        }
        let key = fingerprint(name, args);
        let mut guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let n = guard.entry(key).or_insert(0);
        *n = n.saturating_add(1);
        *n
    }
}

/// Stable identity for "same exact grep / command / offset/limit".
pub fn fingerprint(name: &str, args: &Value) -> String {
    format!("{name}:{}", canonical_json(args))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner = keys
                .into_iter()
                .map(|k| format!("{}:{}", k, canonical_json(&map[k])))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        Value::Array(items) => {
            let inner = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

/// Clip a tool result: **chars first** (20k), then 20 lines. Sub-agent
/// reports pass through unchanged. Preserves a leading `MEDIA_WORKDIR:`
/// sentinel so the download side-effect still fires.
pub fn clip_tool_output(name: &str, raw: String) -> String {
    if is_subagent_output(name) {
        return raw;
    }

    let (sentinel, body) = split_media_sentinel(raw);
    let orig_chars = body.chars().count();
    let orig_lines = line_count(&body);
    let mut text = if orig_chars > MAX_TOOL_OUTPUT_CHARS {
        body.chars().take(MAX_TOOL_OUTPUT_CHARS).collect()
    } else {
        body
    };
    if line_count(&text) > MAX_TOOL_OUTPUT_LINES {
        text = text
            .lines()
            .take(MAX_TOOL_OUTPUT_LINES)
            .collect::<Vec<_>>()
            .join("\n");
    }

    let truncated = orig_chars > MAX_TOOL_OUTPUT_CHARS || orig_lines > MAX_TOOL_OUTPUT_LINES;
    let mut out = String::new();
    if let Some(line) = sentinel {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&text);
    if truncated {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        let kept_chars = text.chars().count();
        let kept_lines = line_count(&text);
        out.push_str(&format!(
            "\n[truncated: kept {kept_chars} chars / {kept_lines} lines of {orig_chars} chars / {orig_lines} lines \
             (max {MAX_TOOL_OUTPUT_CHARS} chars, then {MAX_TOOL_OUTPUT_LINES} lines). \
             Page with offset/limit, or grep one path/pattern. A large dump wipes the context window.]"
        ));
    }
    out
}

/// Clip, then append a repeat warning when this exact call has been seen before.
pub fn finish_tool_output(ctx: &ToolCtx, name: &str, args: &Value, raw: String) -> String {
    let count = ctx.call_track.hit(name, args);
    let mut out = clip_tool_output(name, raw);
    if count >= 2 && !is_subagent_output(name) {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&repeat_notice(name, count));
    }
    out
}

fn repeat_notice(name: &str, count: u32) -> String {
    match name {
        "read" => format!(
            "[repeat: you already read this exact path/offset/limit {count} times. \
             Change offset/limit, or grep a different pattern.]"
        ),
        "grep" => format!(
            "[repeat: you already ran this exact grep {count} times. \
             Change the pattern, path, or glob — grep one file at a time.]"
        ),
        "bash" => format!(
            "[repeat: you already ran this exact command {count} times. \
             Change the command, or pipe through head/grep.]"
        ),
        other => format!(
            "[repeat: you already ran this exact {other} {count} times with the same arguments. \
             Change offset/limit, the pattern, or the command.]"
        ),
    }
}

fn split_media_sentinel(raw: String) -> (Option<String>, String) {
    let Some(first) = raw.lines().next() else {
        return (None, raw);
    };
    if !first.starts_with("MEDIA_WORKDIR:") {
        return (None, raw);
    }
    let sentinel = first.to_string();
    let rest = raw
        .find('\n')
        .map(|i| raw[i + 1..].to_string())
        .unwrap_or_default();
    (Some(sentinel), rest)
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
mod tests;
