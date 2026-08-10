//! `browser_inspect` tool — inspect DOM, console, or network in full internet mode.
//!
//! Read-only: no approval gate. Returns bounded HTML, console history, or
//! network history from a tab.

use super::ToolCtx;
use crate::internet::browser_daemon;
use anyhow::Result;
use serde_json::{json, Value};

pub struct BrowserInspect;

impl super::Tool for BrowserInspect {
    fn name(&self) -> &'static str {
        "browser_inspect"
    }

    fn description(&self) -> &'static str {
        "Inspect a browser tab's current DOM HTML, console history, or network history. \
         Full internet mode only. Returns bounded output. Use tab_id or defaults to active tab."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tab_id": {
                    "type": "string",
                    "description": "Tab ID to inspect. Defaults to active tab."
                },
                "what": {
                    "type": "string",
                    "enum": ["html", "console", "network"],
                    "description": "What to inspect: 'html' for current DOM, 'console' for console messages, 'network' for network requests."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 50000)."
                }
            },
            "required": ["what"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let what = args
            .get("what")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'what'"))?;

        if !["html", "console", "network"].contains(&what) {
            return Ok(format!("error: unknown inspect target '{what}'"));
        }

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok(
                "error: browser_inspect requires internet mode `full`. Use `/internet full` to switch."
                    .to_string(),
            );
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: browser_inspect requires the internet research environment. \
                 Run `koma --internet-fullmode-install`."
                    .to_string(),
            );
        }

        let session_dir = ctx
            .session_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no active session directory"))?;

        let daemon = browser_daemon::get_or_start(session_dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut params = json!({"what": what});
        if let Some(tid) = args.get("tab_id").and_then(Value::as_str) {
            params["tab_id"] = json!(tid);
        }

        let data = daemon.request("inspect", params)?;

        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(50_000) as usize;

        match what {
            "html" => {
                let html = data.get("html").and_then(Value::as_str).unwrap_or("");
                let url = data.get("url").and_then(Value::as_str).unwrap_or("");
                let title = data.get("title").and_then(Value::as_str).unwrap_or("");
                let truncated = truncate_to_chars(html, max_chars);
                let mut out = format!("source: {url}\ntitle: {title}\n\n{truncated}");
                if html.len() > max_chars {
                    out.push_str(&format!(
                        "\n\n... (HTML truncated at {max_chars} chars, full size: {} chars)",
                        html.len()
                    ));
                }
                Ok(out)
            }
            "console" => {
                let entries = data.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
                let count = entries.len();
                let mut out = format!("console ({count} entries):\n");
                for entry in entries.iter().take(max_chars / 100 + 1) {
                    let typ = entry.get("type").and_then(Value::as_str).unwrap_or("log");
                    let text = entry.get("text").and_then(Value::as_str).unwrap_or("");
                    out.push_str(&format!("[{typ}] {text}\n"));
                }
                let rendered = truncate_to_chars(&out, max_chars);
                Ok(rendered)
            }
            "network" => {
                let entries = data.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
                let count = entries.len();
                let mut out = format!("network ({count} entries):\n");
                for entry in entries.iter().take(max_chars / 120 + 1) {
                    let method = entry.get("method").and_then(Value::as_str).unwrap_or("?");
                    let url = entry.get("url").and_then(Value::as_str).unwrap_or("?");
                    let status = entry
                        .get("status")
                        .and_then(Value::as_u64)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    out.push_str(&format!("{method} {status} {url}\n"));
                }
                let rendered = truncate_to_chars(&out, max_chars);
                Ok(rendered)
            }
            _ => unreachable!(),
        }
    }
}

fn truncate_to_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::settings::InternetMode;
    use crate::tool::{Tool, ToolCtx};
    use std::sync::{Arc, RwLock};

    fn make_ctx(internet_mode: InternetMode) -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            workspaces: vec![std::env::temp_dir()],
            dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            internet_mode,
            ssh_key: None,
            skill_registry: None,
            active_skill_names: None,
            mcp_manager: None,
            sec_manager: None,
            bash_saving: false,
            bash_log_dir: None,
            session_dir: None,
            active_skill_dirs: vec![],
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
        }
    }

    #[test]
    fn browser_inspect_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"what": "html"});
        let result = BrowserInspect.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn browser_inspect_rejects_bad_what() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"what": "cookies"});
        let result = BrowserInspect.run(&ctx, &args).unwrap();
        assert!(result.contains("unknown inspect target"), "{result}");
    }

    #[test]
    fn browser_inspect_metadata() {
        assert_eq!(BrowserInspect.name(), "browser_inspect");
        assert!(!BrowserInspect.description().is_empty());
    }
}
