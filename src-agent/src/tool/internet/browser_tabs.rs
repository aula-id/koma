//! `browser_tabs` tool — manage browser tabs in full internet mode.
//!
//! Actions: `open`, `list`, `navigate`, `close`, `select`.
//! Communicates with the persistent browser daemon via Unix socket.

use super::ToolCtx;
use crate::internet::browser_daemon;
use anyhow::Result;
use serde_json::{json, Value};

pub struct BrowserTabs;

impl super::Tool for BrowserTabs {
    fn name(&self) -> &'static str {
        "browser_tabs"
    }

    fn description(&self) -> &'static str {
        "Manage browser tabs in full internet mode. Actions: open (opens+ navigates to URL), \
         list (shows all tabs with URLs and titles), navigate (navigates tab to URL), \
         close (closes a tab), select (sets active tab). Returns tab IDs, URLs, and titles. \
         Full internet mode only."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "list", "navigate", "close", "select"],
                    "description": "The action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL for open/navigate actions (must start with http:// or https://)."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Tab ID for navigate/close/select actions. Defaults to active tab."
                }
            },
            "required": ["action"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'action'"))?;

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok(
                "error: browser_tabs requires internet mode `full`. Use `/internet full` to switch."
                    .to_string(),
            );
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: browser_tabs requires the internet research environment. \
                 Run `koma --internet-fullmode-install`."
                    .to_string(),
            );
        }

        let session_dir = ctx
            .session_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no active session directory"))?;

        let daemon =
            browser_daemon::get_or_start(session_dir).map_err(|e| anyhow::anyhow!("{e}"))?;

        match action {
            "open" => {
                let url = args
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("url is required for open"))?;
                browser_daemon::validate_url_safe(url)?;
                let data = daemon.request("open", json!({"url": url}))?;
                Ok(format_tab_result("opened", &data))
            }
            "list" => {
                let data = daemon.request("list", json!({}))?;
                let tabs = data
                    .get("tabs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if tabs.is_empty() {
                    return Ok("no open tabs".to_string());
                }
                let mut out = String::new();
                for (i, tab) in tabs.iter().enumerate() {
                    let tid = tab.get("tab_id").and_then(Value::as_str).unwrap_or("?");
                    let url = tab.get("url").and_then(Value::as_str).unwrap_or("");
                    let title = tab.get("title").and_then(Value::as_str).unwrap_or("");
                    let active = tab.get("active").and_then(Value::as_bool).unwrap_or(false);
                    let marker = if active { " *" } else { "" };
                    out.push_str(&format!(
                        "{}. [{}]{} {}\n{}",
                        i + 1,
                        tid,
                        marker,
                        title,
                        url
                    ));
                    if i < tabs.len() - 1 {
                        out.push('\n');
                    }
                }
                Ok(out)
            }
            "navigate" => {
                let url = args
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("url is required for navigate"))?;
                browser_daemon::validate_url_safe(url)?;
                let mut params = json!({"url": url});
                if let Some(tid) = args.get("tab_id").and_then(Value::as_str) {
                    params["tab_id"] = json!(tid);
                }
                let data = daemon.request("navigate", params)?;
                Ok(format_tab_result("navigated", &data))
            }
            "close" => {
                let mut params = json!({});
                if let Some(tid) = args.get("tab_id").and_then(Value::as_str) {
                    params["tab_id"] = json!(tid);
                }
                let _data = daemon.request("close", params)?;
                Ok("tab closed".to_string())
            }
            "select" => {
                let tab_id = args
                    .get("tab_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("tab_id is required for select"))?;
                let _data = daemon.request("select", json!({"tab_id": tab_id}))?;
                Ok(format!("tab {} selected", tab_id))
            }
            _ => Ok(format!("error: unknown action '{action}'")),
        }
    }
}

fn format_tab_result(verb: &str, data: &Value) -> String {
    let tab_id = data.get("tab_id").and_then(Value::as_str).unwrap_or("?");
    let url = data.get("url").and_then(Value::as_str).unwrap_or("");
    let title = data.get("title").and_then(Value::as_str).unwrap_or("");
    format!("{verb} tab {tab_id}\nurl: {url}\ntitle: {title}")
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
    fn browser_tabs_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"action": "list"});
        let result = BrowserTabs.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn browser_tabs_missing_action() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({});
        let result = BrowserTabs.run(&ctx, &args);
        assert!(result.is_err(), "should error on missing action");
    }

    #[test]
    fn browser_tabs_metadata() {
        assert_eq!(BrowserTabs.name(), "browser_tabs");
        assert!(!BrowserTabs.description().is_empty());
        let params = BrowserTabs.parameters();
        assert_eq!(params["required"], json!(["action"]));
    }
}
