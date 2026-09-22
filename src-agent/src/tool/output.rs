//! Post-process every tool result before it reaches the model.
//!
//! Large dumps (10k-line reads, chatty bash, fat MCP payloads) blow the
//! context window and force DRSS to cut the conversation — the model then
//! re-reads the same file and loops. Cap every result except sub-agent
//! reports. `read`, `bash`, `grep`, and `glob` use a wider window so one
//! call covers a normal file or a build log.
//! A repeated call is a notice only: the warning is queued beside the
//! result, never appended to it, so a parser cannot persist the suffix.

use super::ToolCtx;
use crate::config::{
    MAX_READ_CHARS, MAX_READ_LINES, MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES,
};
use crate::model::store::session_tool_tmp_dir;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Tools whose result is a sub-agent report (or a control message about one).
/// Those already have their own delivery cap; clipping them here would hide
/// the report the main agent is supposed to read.
pub fn is_subagent_output(name: &str) -> bool {
    matches!(name, "task" | "task_output" | "task_send" | "task_kill")
}

/// Repeat tracking is every tool except a sub-agent report. The notice is
/// queued beside the result, so a sentinel or side-effect string stays
/// byte-identical to what the tool returned.
pub fn repeat_tracked(name: &str) -> bool {
    !is_subagent_output(name)
}

/// `read`, `bash` (including `bash_output`), `grep`, and `glob` share the
/// wide window. Everything else uses the general tool cap.
fn output_caps(name: &str) -> (usize, usize) {
    if matches!(name, "read" | "bash" | "bash_output" | "grep" | "glob") {
        (MAX_READ_CHARS, MAX_READ_LINES)
    } else {
        (MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES)
    }
}

/// Fresh notice queue. The parent session holds one; a sub-agent spawn
/// replaces `ToolCtx::repeat_notices` with another so the two loops do not
/// share warnings.
pub fn new_repeat_notices() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// Take every queued repeat notice, joined by a blank line. `None` if empty.
pub fn drain_repeat_notices(notices: &Mutex<Vec<String>>) -> Option<String> {
    let mut guard = notices.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_empty() {
        return None;
    }
    Some(std::mem::take(&mut *guard).join("\n\n"))
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

    /// Sub-agent reports are not tracked. Every other tool is: a repeat is a
    /// notice, not a rewrite of the result.
    pub fn hit(&self, name: &str, args: &Value) -> u32 {
        if !repeat_tracked(name) {
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

/// Clip a tool result: **chars first**, then lines. `read`, `bash`, `grep`,
/// and `glob` use the wide window; every other tool uses the general cap. Sub-agent reports pass through unchanged. Preserves a leading
/// `MEDIA_WORKDIR:` sentinel so the download side-effect still fires. `spill`
/// is the session tmp path of the unclipped body, when we wrote one.
fn clip_tool_output(name: &str, raw: String, spill: Option<&Path>) -> String {
    if is_subagent_output(name) {
        return raw;
    }

    let (max_chars, max_lines) = output_caps(name);
    let (sentinel, body) = split_media_sentinel(raw);
    let orig_chars = body.chars().count();
    let orig_lines = line_count(&body);
    let mut text = if orig_chars > max_chars {
        body.chars().take(max_chars).collect()
    } else {
        body
    };
    if line_count(&text) > max_lines {
        text = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    }

    let truncated = orig_chars > max_chars || orig_lines > max_lines;
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
        let where_full = match spill {
            Some(path) => format!(
                " Full output: {} — read that file with offset/limit. Do not dump the whole file.",
                path.display()
            ),
            None => " Page with offset/limit, or grep one path/pattern. A large dump wipes the context window.".to_string(),
        };
        out.push_str(&format!(
            "\n[truncated: kept {kept_chars} chars / {kept_lines} lines of {orig_chars} chars / {orig_lines} lines \
             (max {max_chars} chars, then {max_lines} lines).{where_full}]"
        ));
    }
    out
}

/// Clip, spill the unclipped body to `<session>/tmp/` when truncated, then
/// queue a repeat warning when this exact call has been seen before.
/// The returned string is the tool result only — the notice is not part of it.
pub fn finish_tool_output(ctx: &ToolCtx, name: &str, args: &Value, raw: String) -> String {
    let count = ctx.call_track.hit(name, args);
    if is_subagent_output(name) {
        return raw;
    }
    let (max_chars, max_lines) = output_caps(name);
    let (_, body) = split_media_sentinel(raw.clone());
    let truncated = body.chars().count() > max_chars || line_count(&body) > max_lines;
    let spill = if truncated {
        existing_full_output(&body)
            .or_else(|| spill_full_output(ctx.session_dir.as_deref(), name, &body))
    } else {
        None
    };
    let out = clip_tool_output(name, raw, spill.as_deref());
    if repeat_tracked(name) && count >= 2 {
        ctx.repeat_notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(repeat_notice(name, count));
    }
    out
}

/// Reuse a bash-style `full-output: <path>` pointer when the tool already
/// teed the complete dump — no second copy.
fn existing_full_output(body: &str) -> Option<PathBuf> {
    for line in body.lines() {
        let Some(rest) = line.strip_prefix("full-output:") else {
            continue;
        };
        let path = rest.trim();
        if path.is_empty() {
            continue;
        }
        let p = PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Write the unclipped body to `<session_dir>/tmp/<epoch>_<tool>.txt`.
/// Best-effort: a spill failure must never break the tool result.
fn spill_full_output(session_dir: Option<&Path>, name: &str, body: &str) -> Option<PathBuf> {
    let session_dir = session_dir?;
    let dir = session_tool_tmp_dir(session_dir);
    std::fs::create_dir_all(&dir).ok()?;
    let epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = dir.join(format!("{epoch_ms}_{}.txt", tool_slug(name)));
    std::fs::write(&path, body).ok()?;
    gc_tmp_dir(&dir);
    Some(path.canonicalize().unwrap_or(path))
}

fn tool_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').chars().take(40).collect()
}

/// Keep at most 50 spills / 100 MiB in `<session>/tmp/`. Oldest (epoch-prefixed
/// names) go first. Silent on IO errors.
fn gc_tmp_dir(dir: &Path) {
    const MAX_COUNT: usize = 50;
    const MAX_BYTES: u64 = 100 * 1024 * 1024;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        files.push((name.to_string(), size));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total: u64 = files.iter().map(|(_, s)| s).sum();
    let mut count = files.len();
    for (name, size) in files.iter() {
        if count <= MAX_COUNT && total <= MAX_BYTES {
            break;
        }
        if std::fs::remove_file(dir.join(name)).is_ok() {
            total = total.saturating_sub(*size);
            count -= 1;
        }
    }
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
