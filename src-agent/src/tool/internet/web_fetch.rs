//! `web_fetch` tool: fetch a URL and return readable markdown content.
//!
//! This tool is internet-mode-aware. In `Simple` mode (and whenever the Full
//! research environment is not installed) it runs the raw-HTTP path: a blocking
//! `reqwest` GET on a freshly spawned `std::thread` (so it never touches the
//! tokio runtime, which would panic on `blocking`), then readability extraction
//! + HTML→markdown.
//!
//! In `Full` mode WITH the research environment installed, it upgrades the
//! backend: the URL is fetched through the vendored scrapion (`scrapion_agent`,
//! a real Firefox driven by Playwright), which renders JavaScript and gets past
//! Cloudflare. The tool name and arguments are unchanged. ANY failure of the
//! browser path (subprocess error, timeout, no successful result, bad JSON)
//! falls back to the raw-HTTP path, so `web_fetch` always returns something
//! useful.

use super::{http_get_blocking, looks_like_cloudflare, Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::Duration;

/// Outer wait budget for the scrapion subprocess. It launches Firefox and
/// renders the page, so this is deliberately generous.
const SCRAPION_TIMEOUT_SECS: u64 = 150;

/// Per-result content cap (characters) for the browser-fetched content handed
/// back to the model.
const SCRAPION_MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;

/// Fetch a web page and return its main content as markdown.
pub struct WebFetch;

impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch a web page by URL and return its main readable content as markdown. \
        Use when you already have a specific URL to read."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full URL to fetch (must start with http:// or https://)."
                }
            },
            "required": ["url"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'url'"))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(format!(
                "error: url must start with http:// or https://, got: {url}"
            ));
        }

        // Full mode + installed research env: try the browser backend first.
        // On ANY failure, fall through to the raw-HTTP path below so the tool
        // always returns useful content.
        if ctx.internet_mode == crate::model::settings::InternetMode::Full
            && crate::internet::is_installed()
        {
            if let Ok(content) = scrapion_fetch(url) {
                return Ok(content);
            }
            // else: fall through to the raw-HTTP path.
        }

        raw_http_fetch(url)
    }
}

/// The simple-tier raw-HTTP fetch: blocking GET (off the tokio thread) →
/// readability → markdown, capped at [`crate::config::MAX_TOOL_OUTPUT_CHARS`].
/// Unchanged behaviour from the original `web_fetch`.
fn raw_http_fetch(url: &str) -> Result<String> {
    let (status, body) = match http_get_blocking(url, Duration::from_secs(30)) {
        Ok(v) => v,
        Err(e) => return Ok(format!("error: {e}")),
    };

    if looks_like_cloudflare(status, &body) {
        return Ok(format!("blocked: Cloudflare challenge, skipped {url}"));
    }

    if !(200..300).contains(&status) {
        return Ok(format!("error: HTTP {status} for {url}"));
    }

    // Try dom_smoothie readability extraction; fall back to full HTML on failure.
    let readable_html = extract_readable(&body, url);

    // Convert HTML to markdown.
    let markdown = html2md::rewrite_html(&readable_html, false);

    // Trim and cap at MAX_CHARS.
    const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
    let trimmed = markdown.trim();
    let (content, truncated) = if trimmed.chars().count() > MAX_CHARS {
        let cut: String = trimmed.chars().take(MAX_CHARS).collect();
        (cut, true)
    } else {
        (trimmed.to_string(), false)
    };

    let mut out = format!("source: {url}\n\n{content}");
    if truncated {
        out.push_str(&format!("\n\n... (content truncated at {MAX_CHARS} chars)"));
    }
    Ok(out)
}

/// Fetch `url` via the vendored scrapion (`python -m scrapion_agent --json <url>`),
/// which renders the page in a real Firefox. Returns the page content as markdown
/// on success, or `Err(...)` on any failure so the caller can fall back to raw HTTP.
///
/// scrapion's `InputHandler` detects a URL (vs a search query) and switches to
/// single-URL mode → Playwright Firefox → markdown. stdout is always a single
/// JSON document: a `Report` on success, or `{"error": ...}` on failure. We take
/// the FIRST result whose `status == "success"` and return its `content`, capped
/// at [`SCRAPION_MAX_CHARS`]. A top-level `error`, no successful result, a
/// non-zero exit, or unparseable output all yield `Err`.
///
/// Concurrency: the subprocess is launched + waited inside a freshly spawned
/// `std::thread` (no tokio context) and its `Output` returned via an `mpsc`
/// channel guarded by `recv_timeout` — the same pattern as `http_get_blocking`.
fn scrapion_fetch(url: &str) -> Result<String, String> {
    // Resolve the venv Python + install dir; a path-resolution failure is just a
    // fallback trigger (the caller will use raw HTTP).
    let python = crate::internet::venv_python()
        .map_err(|e| format!("cannot locate research environment: {e}"))?;
    let dir = crate::internet::internet_dir()
        .map_err(|e| format!("cannot locate research environment: {e}"))?;

    // Run `python -m scrapion_agent --json <url>` with cwd = install dir, inside
    // a dedicated OS thread (no tokio context), guarded by recv_timeout.
    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    let url_arg = url.to_string();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new(&python);
        cmd.arg("-m")
            .arg("scrapion_agent")
            .arg("--json")
            .arg(&url_arg)
            .current_dir(&dir);
        // No console flash on Windows — see `tool::shell::no_console_window`'s
        // docs for the `FreeConsole()` causal chain this guards against.
        crate::tool::shell::no_console_window(&mut cmd);
        let out = cmd.output();
        let _ = tx.send(out);
    });

    let output = match rx.recv_timeout(Duration::from_secs(SCRAPION_TIMEOUT_SECS)) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("scrapion failed to launch: {e}")),
        // The thread still owns the child; we can't reap it here, but we return a
        // clear error so the caller falls back instead of stalling.
        Err(_) => return Err(format!("scrapion timed out after {SCRAPION_TIMEOUT_SECS}s")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Non-zero exit ⇒ fall back. Keep a short stderr tail for the error string.
    if !output.status.success() {
        return Err(format!(
            "scrapion exited non-zero: {}",
            tail_chars(stderr.trim(), 300)
        ));
    }

    // stdout is contractually a single JSON document. A parse failure is treated
    // as a fallback trigger rather than a panic.
    let report: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("scrapion produced invalid JSON: {e}"))?;

    // A top-level `error` field short-circuits to a fallback.
    if let Some(err) = report.get("error").and_then(Value::as_str) {
        if !err.trim().is_empty() {
            return Err(format!(
                "scrapion reported: {}",
                first_chars(err.trim(), 300)
            ));
        }
    }

    // Take the FIRST result whose status == "success" and return its content.
    let results = report
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "scrapion returned no results".to_string())?;
    let success = results
        .iter()
        .find(|r| r.get("status").and_then(Value::as_str) == Some("success"));
    let content = match success {
        Some(r) => r.get("content").and_then(Value::as_str).unwrap_or(""),
        None => return Err("scrapion returned no successful result".to_string()),
    };
    if content.trim().is_empty() {
        return Err("scrapion returned empty content".to_string());
    }

    // Cap the content, noting truncation.
    let trimmed = content.trim();
    let (body, truncated) = if trimmed.chars().count() > SCRAPION_MAX_CHARS {
        let cut: String = trimmed.chars().take(SCRAPION_MAX_CHARS).collect();
        (cut, true)
    } else {
        (trimmed.to_string(), false)
    };

    let mut out = format!("source: {url}\n\n{body}");
    if truncated {
        out.push_str(&format!(
            "\n\n... (content truncated at {SCRAPION_MAX_CHARS} chars)"
        ));
    }
    Ok(out)
}

/// First `n` chars of `s` (char-boundary safe), no note appended.
fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Last `n` chars of `s` (char-boundary safe), no note appended.
fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    s.chars().skip(count - n).collect()
}

/// Try dom_smoothie readability on the HTML; return the article content HTML on
/// success, or the original HTML unchanged when readability fails.
fn extract_readable(html: &str, url: &str) -> String {
    // dom_smoothie requires an absolute URL; the caller already validated it.
    match dom_smoothie::Readability::new(html, Some(url), None) {
        Ok(mut r) => match r.parse() {
            Ok(article) => article.content.to_string(),
            Err(_) => html.to_string(),
        },
        Err(_) => html.to_string(),
    }
}
