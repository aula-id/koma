//! `web_screenshot` tool — Full-mode browser screenshot.
//!
//! Captures a PNG screenshot via the scrapion browser backend. When `tab_id` is
//! provided, screenshots an existing persistent browser tab; otherwise
//! navigates a one-shot subprocess.
//!
//! **Full mode only**: requires `InternetMode::Full` *and* the research
//! environment to be installed.
//!
//! The PNG is saved under `<workspace>/.screenshoot/`.

use super::ToolCtx;
use crate::internet::{browser_daemon, scrapion_run};
use anyhow::Result;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Full-mode browser screenshot tool.
pub struct WebScreenshot;

impl super::Tool for WebScreenshot {
    fn name(&self) -> &'static str {
        "web_screenshot"
    }

    fn description(&self) -> &'static str {
        "Capture a screenshot via a headless browser (renders JavaScript, beats Cloudflare). \
         Full internet mode only. Supply `url` for a one-shot page, or `tab_id` for an existing \
         tab. Optional viewport params: `width` (default 1920), `height` (default 1080), \
         `delay_ms` (default 300), `full_page` (default false — set true to auto-scroll and \
         capture entire page). Saves PNG to `.screenshoot/`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full URL to screenshot (must start with http:// or https://)."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Existing tab ID to screenshot. If provided with a url, navigates first."
                },
                "width": {
                    "type": "integer",
                    "description": "Viewport width in pixels (default 1920)."
                },
                "height": {
                    "type": "integer",
                    "description": "Viewport height in pixels (default 1080)."
                },
                "delay_ms": {
                    "type": "integer",
                    "description": "Delay in milliseconds after navigation before capture (default 300)."
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture the full scrollable page (default true)."
                }
            }
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let url = args.get("url").and_then(Value::as_str);
        let tab_id = args.get("tab_id").and_then(Value::as_str);

        if url.is_none() && tab_id.is_none() {
            return Ok("error: provide either 'url' or 'tab_id'".to_string());
        }

        if let Some(u) = url {
            if !u.starts_with("http://") && !u.starts_with("https://") {
                return Ok(format!("error: url must start with http:// or https://, got: {u}"));
            }
        }

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok("error: web_screenshot requires internet mode `full`. \
                 Use `/internet full` to enable full-mode browser tools."
                .to_string());
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: web_screenshot requires the internet research environment. \
                 Run `koma --internet-fullmode-install` to install it."
                    .to_string(),
            );
        }

        // ── Build output path ───────────────────────────────────────────
        let screenshoot_dir = ctx.workspace.join(".screenshoot");
        std::fs::create_dir_all(&screenshoot_dir)
            .map_err(|e| anyhow::anyhow!("failed to create .screenshoot dir: {e}"))?;

        let target_url = url.unwrap_or("");
        let filename = screenshot_filename(if target_url.is_empty() { "tab" } else { target_url });
        let output_path = screenshoot_dir.join(&filename);
        let output_str = output_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("output path is not valid UTF-8"))?;

        let width = args
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(1920) as u32;
        let height = args
            .get("height")
            .and_then(Value::as_u64)
            .unwrap_or(1080) as u32;
        let delay_ms = args
            .get("delay_ms")
            .and_then(Value::as_u64)
            .unwrap_or(300) as u32;
        let full_page = args
            .get("full_page")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // ── Daemon path (tab_id provided) ──────────────────────────────
        if let Some(tid) = tab_id {
            let session_dir = ctx
                .session_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no active session directory"))?;
            let daemon = browser_daemon::get_or_start(session_dir)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

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
        let mut ss_args: Vec<&str> = vec![
            "screenshot",
            "--url",
            target_url,
            "--output",
            output_str,
        ];
        if width != 1920 || height != 1080 {
            ss_args.extend_from_slice(&["--width", &w]);
            ss_args.extend_from_slice(&["--height", &h]);
        }
        if !full_page {
            ss_args.push("--no-full-page");
        }
        let stdout = scrapion_run(&ss_args)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let report: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("scrapion produced invalid JSON: {e}"))?;

        if let Some(err) = report.get("error").and_then(Value::as_str) {
            if !err.trim().is_empty() {
                return Ok(format!("web_screenshot error: {}", err.trim()));
            }
        }

        let status = report.get("status").and_then(Value::as_str).unwrap_or("");
        if status != "success" {
            let err = report
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Ok(format!("web_screenshot error: {err}"));
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
                "web_screenshot: screenshot reported success but file not found at {}",
                output_path.display()
            ));
        }
    };

    let size = meta.len();
    let saved = output_path.display().to_string();

    // Register in the screenshot catalog.
    let stem = filename.strip_suffix(".png").unwrap_or(filename);
    let catalog_msg = match crate::model::screenshot_catalog::register_screenshot(
        &ctx.workspace,
        stem,
        url,
    ) {
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
            scratch_dir: None,
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
    fn web_screenshot_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"url": "https://example.com"});
        let result = WebScreenshot.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn web_screenshot_rejects_bad_url() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"url": "ftp://example.com"});
        let result = WebScreenshot.run(&ctx, &args).unwrap();
        assert!(result.contains("must start with http"), "{result}");
    }

    #[test]
    fn web_screenshot_missing_both_args() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({});
        let result = WebScreenshot.run(&ctx, &args).unwrap();
        assert!(result.contains("provide either"), "{result}");
    }

    #[test]
    fn web_screenshot_metadata() {
        assert_eq!(WebScreenshot.name(), "web_screenshot");
        assert!(!WebScreenshot.description().is_empty());
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
