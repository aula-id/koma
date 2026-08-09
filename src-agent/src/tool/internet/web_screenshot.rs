//! `web_screenshot` tool — Full-mode browser screenshot.
//!
//! Captures a full-page PNG screenshot of a URL via the scrapion browser backend
//! (`python -m scrapion_agent screenshot --url <URL> --output <PATH>`).
//!
//! **Full mode only**: requires `InternetMode::Full` *and* the research
//! environment to be installed. Returns a clear error if either gate is not met
//! — there is **no** raw-HTTP fallback.
//!
//! The PNG is saved under `<workspace>/.screenshoot/` (note: exact directory
//! name is `.screenshoot`).  The filename is derived from a sanitised form of
//! the URL hostname + path, suffixed with a millisecond timestamp to avoid
//! collisions.

use super::ToolCtx;
use crate::internet::scrapion_run;
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
        "Capture a full-page screenshot of a URL via a headless browser (renders JavaScript, \
         beats Cloudflare). Full internet mode only — requires `koma --internet-fullmode-install` \
         and internet mode set to `full`. Saves a PNG to `.screenshoot/` in the project root."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full URL to screenshot (must start with http:// or https://)."
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

        let filename = screenshot_filename(url);
        let output_path = screenshoot_dir.join(&filename);
        let output_str = output_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("output path is not valid UTF-8"))?;

        // ── Subprocess call ─────────────────────────────────────────────
        let stdout = scrapion_run(&["screenshot", "--url", url, "--output", output_str])
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

        // Verify the file was actually written.
        let meta = match std::fs::metadata(&output_path) {
            Ok(m) => m,
            Err(_) => {
                return Ok(format!(
                    "web_screenshot: screenshot command reported success but file not found at {}",
                    output_path.display()
                ));
            }
        };

        let size = meta.len();
        let saved = output_path.display().to_string();

        // Register in the screenshot catalog.
        let stem = filename.strip_suffix(".png").unwrap_or(&filename);
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
}

/// Derive a safe filename from a URL for the screenshot.
///
/// Pattern: `{sanitised_host}_{sanitised_path_tokens}_{millis}.png`
fn screenshot_filename(url: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    // Extract host + path, strip protocol.
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Split on first '/' to get host and path.
    let (host, path_part) = match stripped.find('/') {
        Some(idx) => (&stripped[..idx], &stripped[idx + 1..]),
        None => (stripped, ""),
    };

    // Sanitise: replace non-alphanumeric with underscores, collapse multiples.
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
    fn web_screenshot_missing_url_arg() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({});
        let result = WebScreenshot.run(&ctx, &args);
        assert!(result.is_err(), "should error on missing url");
    }

    #[test]
    fn web_screenshot_metadata() {
        assert_eq!(WebScreenshot.name(), "web_screenshot");
        assert!(!WebScreenshot.description().is_empty());
        let params = WebScreenshot.parameters();
        assert_eq!(params["required"], json!(["url"]));
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
