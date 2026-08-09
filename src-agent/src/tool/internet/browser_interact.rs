//! `browser_interact` tool — perform user-like browser interactions.
//!
//! Supports: click, fill, press, select, scroll, wait.
//! Uses Playwright locators for auto-waiting.

use super::ToolCtx;
use crate::internet::browser_daemon;
use anyhow::Result;
use serde_json::{json, Value};

pub struct BrowserInteract;

impl super::Tool for BrowserInteract {
    fn name(&self) -> &'static str {
        "browser_interact"
    }

    fn description(&self) -> &'static str {
        "Perform user-like browser interactions: click, fill, press key, select option, scroll, \
         wait for selector/URL/response. Uses Playwright locators with auto-waiting. \
         Full internet mode only. Approval-gated."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tab_id": {
                    "type": "string",
                    "description": "Tab ID to interact with. Defaults to active tab."
                },
                "action": {
                    "type": "string",
                    "enum": ["click", "fill", "press", "select", "scroll", "wait"],
                    "description": "The interaction to perform."
                },
                "locator": {
                    "type": "string",
                    "description": "Element locator (CSS selector, role name, or text). Used by click/fill/select."
                },
                "locator_type": {
                    "type": "string",
                    "enum": ["css", "role", "text"],
                    "description": "How to interpret the locator. Default: css."
                },
                "value": {
                    "type": "string",
                    "description": "Value to fill/select, key to press, or wait target."
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down"],
                    "description": "Scroll direction (for scroll action). Default: down."
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount in pixels (for scroll action). Default: 500."
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
                "error: browser_interact requires internet mode `full`. Use `/internet full` to switch."
                    .to_string(),
            );
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: browser_interact requires the internet research environment. \
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

        // Build interaction params.
        let locator = args.get("locator").and_then(Value::as_str);
        let locator_type = args
            .get("locator_type")
            .and_then(Value::as_str)
            .unwrap_or("css");
        let value = args.get("value").and_then(Value::as_str);

        let mut action_params = json!({});
        if let Some(loc) = locator {
            action_params["locator"] = json!(loc);
            action_params["locator_type"] = json!(locator_type);
        }
        if let Some(val) = value {
            action_params["value"] = json!(val);
        }
        if action == "scroll" {
            let direction = args
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("down");
            let amount = args
                .get("amount")
                .and_then(Value::as_u64)
                .unwrap_or(500);
            action_params["direction"] = json!(direction);
            action_params["amount"] = json!(amount);
        }

        let mut params = json!({
            "action": action,
            "params": action_params,
        });
        if let Some(tid) = args.get("tab_id").and_then(Value::as_str) {
            params["tab_id"] = json!(tid);
        }

        let data = daemon.request("interact", params)?;

        // Format result.
        match action {
            "click" => Ok(format!(
                "clicked on {}",
                locator.unwrap_or("element")
            )),
            "fill" => Ok(format!(
                "filled {} with \"{}\"",
                locator.unwrap_or("element"),
                value.unwrap_or("")
            )),
            "press" => Ok(format!(
                "pressed key {}",
                value.unwrap_or("?")
            )),
            "select" => Ok(format!(
                "selected \"{}\" in {}",
                value.unwrap_or(""),
                locator.unwrap_or("element")
            )),
            "scroll" => {
                let dir = args
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("down");
                Ok(format!("scrolled {dir}"))
            }
            "wait" => Ok(format!(
                "waited for {} {}",
                args.get("what").and_then(Value::as_str).unwrap_or("condition"),
                value.unwrap_or("")
            )),
            _ => Ok(format!("action '{action}' completed")),
        }
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
    fn browser_interact_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"action": "click", "locator": "#btn"});
        let result = BrowserInteract.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn browser_interact_metadata() {
        assert_eq!(BrowserInteract.name(), "browser_interact");
        assert!(!BrowserInteract.description().is_empty());
    }
}
