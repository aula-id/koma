//! `browser_interact` tool — perform user-like browser interactions.
//!
//! Supports: click, fill, press, select, scroll, wait, screenshot.
//! Uses Playwright locators for auto-waiting.

use super::ToolCtx;
use crate::internet::{browser_daemon, scrapion_run};
use anyhow::Result;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct BrowserInteract;

impl super::Tool for BrowserInteract {
    fn name(&self) -> &'static str {
        "browser_interact"
    }

    fn description(&self) -> &'static str {
        "Perform user-like browser interactions: click, fill, press key, select option, scroll, \
         wait for selector/URL/response, or capture a screenshot. Uses Playwright locators with \
         auto-waiting. Full internet mode only. Approval-gated."
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
                    "enum": ["click", "fill", "press", "select", "scroll", "wait", "screenshot"],
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
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to before screenshot (screenshot action only)."
                },
                "width": {
                    "type": "integer",
                    "description": "Viewport width in pixels for screenshot (default 1920)."
                },
                "height": {
                    "type": "integer",
                    "description": "Viewport height in pixels for screenshot (default 1080)."
                },
                "delay_ms": {
                    "type": "integer",
                    "description": "Delay in milliseconds after navigation before screenshot capture (default 300)."
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture the full scrollable page (auto-scrolls to trigger animations first, default false)."
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

        // ── Screenshot action (pure validation before env gate) ───────────
        // These checks are pure (no daemon needed), so do them before the
        // is_installed() gate to allow tests to verify error messages on CI.
        if action == "screenshot" {
            let url = args.get("url").and_then(Value::as_str);
            let tab_id = args.get("tab_id").and_then(Value::as_str);

            if url.is_none() && tab_id.is_none() {
                return Ok("error: screenshot requires either 'url' or 'tab_id'".to_string());
            }

            if let Some(u) = url {
                if !u.starts_with("http://") && !u.starts_with("https://") {
                    return Ok(format!(
                        "error: url must start with http:// or https://, got: {u}"
                    ));
                }
            }

            if !crate::internet::is_installed() {
                return Ok(
                    "error: browser_interact requires the internet research environment. \
                     Run `koma --internet-fullmode-install`."
                        .to_string(),
                );
            }

            return self.run_screenshot(ctx, args);
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

        let daemon =
            browser_daemon::get_or_start(session_dir).map_err(|e| anyhow::anyhow!("{e}"))?;

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
            let amount = args.get("amount").and_then(Value::as_u64).unwrap_or(500);
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

        let _data = daemon.request("interact", params)?;

        // Format result.
        match action {
            "click" => Ok(format!("clicked on {}", locator.unwrap_or("element"))),
            "fill" => Ok(format!(
                "filled {} with \"{}\"",
                locator.unwrap_or("element"),
                value.unwrap_or("")
            )),
            "press" => Ok(format!("pressed key {}", value.unwrap_or("?"))),
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
                args.get("what")
                    .and_then(Value::as_str)
                    .unwrap_or("condition"),
                value.unwrap_or("")
            )),
            _ => Ok(format!("action '{action}' completed")),
        }
    }
}

impl BrowserInteract {
    /// Handle the `screenshot` action — absorbs the former `web_screenshot` tool.
    /// Input validation (url/tab_id presence, URL scheme) is done in `run()` before
    /// the environment gate so tests can verify error messages without the daemon.
    fn run_screenshot(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let url = args.get("url").and_then(Value::as_str);
        let tab_id = args.get("tab_id").and_then(Value::as_str);

        let width = args.get("width").and_then(Value::as_u64).unwrap_or(1920) as u32;
        let height = args.get("height").and_then(Value::as_u64).unwrap_or(1080) as u32;
        let delay_ms = args.get("delay_ms").and_then(Value::as_u64).unwrap_or(300) as u32;
        let full_page = args
            .get("full_page")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // ── Build output path ───────────────────────────────────────────
        let screenshoot_dir = ctx.workspace.join(".screenshoot");
        std::fs::create_dir_all(&screenshoot_dir)
            .map_err(|e| anyhow::anyhow!("failed to create .screenshoot dir: {e}"))?;

        let target_url = url.unwrap_or("");
        let filename = screenshot_filename(if target_url.is_empty() {
            "tab"
        } else {
            target_url
        });
        let output_path = screenshoot_dir.join(&filename);
        let output_str = output_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("output path is not valid UTF-8"))?;

        // ── Daemon path (tab_id provided) ──────────────────────────────
        if let Some(tid) = tab_id {
            let session_dir = ctx
                .session_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no active session directory"))?;
            let daemon =
                browser_daemon::get_or_start(session_dir).map_err(|e| anyhow::anyhow!("{e}"))?;

            // If URL given, navigate first.
            if !target_url.is_empty() {
                browser_daemon::validate_url_safe(target_url)?;
                let _ = daemon.request("navigate", json!({"tab_id": tid, "url": target_url}))?;
            }

            let _data = daemon.request(
                "screenshot",
                json!({
                    "tab_id": tid,
                    "output_path": output_str,
                    "width": width,
                    "height": height,
                    "delay_ms": delay_ms,
                    "full_page": full_page,
                }),
            )?;

            return finish_screenshot(ctx, &output_path, &filename, target_url);
        }

        // ── One-shot subprocess path (url only) ────────────────────────
        let w = width.to_string();
        let h = height.to_string();
        let mut ss_args: Vec<&str> =
            vec!["screenshot", "--url", target_url, "--output", output_str];
        if width != 1920 || height != 1080 {
            ss_args.extend_from_slice(&["--width", &w]);
            ss_args.extend_from_slice(&["--height", &h]);
        }
        if !full_page {
            ss_args.push("--no-full-page");
        }
        let stdout = scrapion_run(&ss_args).map_err(|e| anyhow::anyhow!("{e}"))?;

        let report: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("scrapion produced invalid JSON: {e}"))?;

        if let Some(err) = report.get("error").and_then(Value::as_str) {
            if !err.trim().is_empty() {
                return Ok(format!("screenshot error: {}", err.trim()));
            }
        }

        let status = report.get("status").and_then(Value::as_str).unwrap_or("");
        if status != "success" {
            let err = report
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Ok(format!("screenshot error: {err}"));
        }

        finish_screenshot(ctx, &output_path, &filename, target_url)
    }
}

fn finish_screenshot(
    ctx: &ToolCtx,
    output_path: &std::path::Path,
    filename: &str,
    url: &str,
) -> Result<String> {
    // Verify the file was written.
    let meta = match std::fs::metadata(output_path) {
        Ok(m) => m,
        Err(_) => {
            return Ok(format!(
                "screenshot: reported success but file not found at {}",
                output_path.display()
            ));
        }
    };

    let size = meta.len();
    let saved = output_path.display().to_string();

    // Register in the screenshot catalog.
    let stem = filename.strip_suffix(".png").unwrap_or(filename);
    let catalog_msg =
        match crate::model::screenshot_catalog::register_screenshot(&ctx.workspace, stem, url) {
            Ok(stem) => format!(
                "catalog: registered as {stem} — description pending. \
             Use `load_screenshot` to inspect, then `describe_screenshot` to add a description."
            ),
            Err(e) => format!("catalog: registration failed ({e})"),
        };

    Ok(format!(
        "screenshot saved: {saved}\nsize: {size} bytes\nurl: {url}\n{catalog_msg}"
    ))
}

/// Derive a safe filename from a URL for the screenshot.
fn screenshot_filename(url: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    let (host, path_part) = match stripped.find('/') {
        Some(idx) => (&stripped[..idx], &stripped[idx + 1..]),
        None => (stripped, ""),
    };

    let sanitise = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .replace("__", "_")
            .trim_matches('_')
            .chars()
            .take(60)
            .collect::<String>()
    };

    let host_clean = sanitise(host);
    let path_clean = sanitise(path_part);

    if path_clean.is_empty() {
        format!("{host_clean}_{millis}.png")
    } else {
        format!("{host_clean}_{path_clean}_{millis}.png")
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

    #[test]
    fn screenshot_rejects_bad_url() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"action": "screenshot", "url": "ftp://example.com"});
        let result = BrowserInteract.run(&ctx, &args).unwrap();
        assert!(result.contains("must start with http"), "{result}");
    }

    #[test]
    fn screenshot_missing_both_args() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"action": "screenshot"});
        let result = BrowserInteract.run(&ctx, &args).unwrap();
        assert!(result.contains("requires either"), "{result}");
    }

    #[test]
    fn screenshot_filename_basic() {
        let f = screenshot_filename("https://example.com/page");
        assert!(f.ends_with(".png"));
        assert!(f.contains("example_com"));
        assert!(f.contains("page"));
    }

    #[test]
    fn screenshot_filename_no_path() {
        let f = screenshot_filename("https://example.com");
        assert!(f.ends_with(".png"));
        assert!(f.contains("example_com"));
    }

    #[test]
    fn screenshot_filename_sanitises_special_chars() {
        let f = screenshot_filename("https://example.com/a/b/c?q=1&r=2");
        assert!(!f.contains('?'));
        assert!(!f.contains('='));
        assert!(f.ends_with(".png"));
    }
}
