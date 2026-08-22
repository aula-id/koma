//! `browser_evaluate` tool — execute JavaScript in a browser tab.
//!
//! Runs arbitrary JS in the page context. Returns serialized result.
//! Approval-gated: JavaScript execution can modify DOM state.

use super::ToolCtx;
use crate::internet::browser_daemon;
use anyhow::Result;
use serde_json::{json, Value};

pub struct BrowserEvaluate;

impl super::Tool for BrowserEvaluate {
    fn name(&self) -> &'static str {
        "browser_evaluate"
    }

    fn description(&self) -> &'static str {
        "Execute JavaScript in a browser tab's page context. Returns serialized result. \
         Accepts serializable input args. Full internet mode only. Approval-gated."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tab_id": {
                    "type": "string",
                    "description": "Tab ID to evaluate in. Defaults to active tab."
                },
                "script": {
                    "type": "string",
                    "description": "JavaScript code to execute. Must return a JSON-serializable value."
                },
                "args": {
                    "type": "array",
                    "description": "Optional array of arguments passed to the script.",
                    "items": {}
                }
            },
            "required": ["script"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'script'"))?;

        if script.trim().is_empty() {
            return Ok("error: script must not be empty".to_string());
        }

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok(
                "error: browser_evaluate requires internet mode `full`. Use `/internet full` to switch."
                    .to_string(),
            );
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: browser_evaluate requires the internet research environment. \
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

        let script_args = args.get("args").cloned().unwrap_or_else(|| json!([]));

        let mut params = json!({
            "script": script,
            "args": script_args,
        });
        if let Some(tid) = args.get("tab_id").and_then(Value::as_str) {
            params["tab_id"] = json!(tid);
        }

        let data = daemon.request("evaluate", params)?;

        // Cap the result at MAX_TOOL_OUTPUT_CHARS.
        let result_str = serde_json::to_string_pretty(&data).unwrap_or_else(|_| format!("{data}"));
        const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
        if result_str.chars().count() > MAX_CHARS {
            let truncated: String = result_str.chars().take(MAX_CHARS).collect();
            Ok(format!(
                "{truncated}\n\n... (result truncated at {MAX_CHARS} chars)"
            ))
        } else {
            Ok(result_str)
        }
    }
}

#[cfg(test)]
#[path = "browser_evaluate_test.rs"]
mod tests;
