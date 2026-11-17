//! The hard tool deny list.
//!
//! A host that embeds koma (a sandboxed cell, a locked-down server) cannot rely
//! on a gateway to strip tools: `execute_tool` runs any known tool by name
//! whether it was offered or not, and a model can emit a call as plain text.
//! This list is the one place that says a tool is off, and every path a call
//! can take asks it: the model's tool list, `execute_tool`, the turn loop, the
//! sub-agent engine and the room.
//!
//! Patterns are tool names with `*` (any run of characters) and `?` (one
//! character), compared to the whole name, case-sensitively: `web_fetch`,
//! `computer_*`, `mcp__*`. The list is the union of two sources:
//!
//! - `KOMA_TOOL_DENY`, comma or whitespace separated, read on every check;
//! - `tool_policy.deny` in `config.json`, re-read when the file changes.
//!
//! Empty by default, so nothing changes until a caller opts in.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// Environment variable carrying the deny patterns.
pub const ENV_DENY: &str = "KOMA_TOOL_DENY";

/// Most patterns one list may hold, and the longest one.
pub const MAX_PATTERNS: usize = 256;
pub const MAX_PATTERN_CHARS: usize = 200;

/// Split a `KOMA_TOOL_DENY` style value into patterns.
pub fn parse_list(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Check a list a caller wants to store: bounded, printable, no spaces.
/// Returns it deduplicated, in the given order.
pub fn validate(patterns: &[String]) -> Result<Vec<String>, String> {
    if patterns.len() > MAX_PATTERNS {
        return Err(format!("deny has at most {MAX_PATTERNS} entries"));
    }
    let mut out: Vec<String> = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        if pattern.is_empty()
            || pattern.chars().count() > MAX_PATTERN_CHARS
            || pattern
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == ',')
        {
            return Err(format!(
                "a deny pattern is 1 to {MAX_PATTERN_CHARS} characters with no spaces, commas or control characters"
            ));
        }
        if !out.contains(pattern) {
            out.push(pattern.clone());
        }
    }
    Ok(out)
}

/// Whether `pattern` matches the whole of `name`.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) && p[pi] != '*' {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// What the config file last said, keyed by the file's identity so an
/// unchanged file costs one `stat` per check.
struct FileCache {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    deny: Vec<String>,
}

static FILE_CACHE: Mutex<Option<FileCache>> = Mutex::new(None);

/// `tool_policy.deny` out of a parsed `config.json`.
fn deny_of(doc: &serde_json::Value) -> Vec<String> {
    doc.get("tool_policy")
        .and_then(|p| p.get("deny"))
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `tool_policy.deny` from `config.json`. A file that is gone, or cannot be
/// read or parsed right now, keeps the last list read from that path: a
/// half-written file must not open tools that were closed a moment ago.
fn config_deny() -> Vec<String> {
    let Ok(dir) = crate::model::store::base_dir() else {
        return Vec::new();
    };
    let path = dir.join("config.json");
    let stamp = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok().map(|t| (t, m.len())));
    let mut cache = FILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = cache.as_ref() {
        if c.path == path && c.stamp == stamp {
            return c.deny.clone();
        }
    }
    let previous = cache
        .as_ref()
        .filter(|c| c.path == path)
        .map(|c| c.deny.clone());
    let read = if stamp.is_none() {
        // No file: no policy was ever written there.
        Some(Vec::new())
    } else {
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|doc| deny_of(&doc))
    };
    match read {
        Some(deny) => {
            *cache = Some(FileCache {
                path,
                stamp,
                deny: deny.clone(),
            });
            deny
        }
        None => previous.unwrap_or_default(),
    }
}

// A test list is per thread, so a test that closes a tool cannot close it for
// the tests running beside it. Tests that cross a thread use the env variable.
#[cfg(test)]
thread_local! {
    static TEST_DENY: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Every pattern in force: the environment and the config file.
pub fn current() -> Vec<String> {
    let mut out = Vec::new();
    #[cfg(test)]
    TEST_DENY.with(|t| {
        if let Some(extra) = t.borrow().as_ref() {
            out.extend(extra.iter().cloned());
        }
    });
    if let Ok(text) = std::env::var(ENV_DENY) {
        out.extend(parse_list(&text));
    }
    for pattern in config_deny() {
        if !out.contains(&pattern) {
            out.push(pattern);
        }
    }
    out
}

/// Whether a call to `name` must be refused.
pub fn is_denied(name: &str) -> bool {
    current().iter().any(|pattern| glob_match(pattern, name))
}

/// The tool result a refused call gets. It names the tool and says nothing
/// about who set the policy.
pub fn refusal(name: &str) -> String {
    format!("error: tool '{name}' is disabled by the tool policy and cannot be used")
}

/// Drop every denied name from an advertised list.
pub fn retain_allowed(names: &mut Vec<String>) {
    let patterns = current();
    if patterns.is_empty() {
        return;
    }
    names.retain(|name| !patterns.iter().any(|p| glob_match(p, name)));
}

/// Hold a test-only deny list on this thread until the guard drops.
#[cfg(test)]
pub struct TestGuard;

#[cfg(test)]
pub fn test_deny(patterns: &[&str]) -> TestGuard {
    TEST_DENY.with(|t| *t.borrow_mut() = Some(patterns.iter().map(|p| p.to_string()).collect()));
    TestGuard
}

#[cfg(test)]
impl Drop for TestGuard {
    fn drop(&mut self) {
        TEST_DENY.with(|t| *t.borrow_mut() = None);
    }
}

#[cfg(test)]
#[path = "policy_test.rs"]
mod tests;
