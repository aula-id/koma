//! `web_page` tool — Full-mode browser page fetch.
//!
//! Fetches a URL through the scrapion browser backend (`python -m scrapion_agent
//! page --url <URL>`) which renders JavaScript and passes Cloudflare challenges.
//!
//! **Full mode only**: requires `InternetMode::Full` *and* the research
//! environment to be installed. Returns a clear error if either gate is not met
//! — there is **no** raw-HTTP fallback (use `web_fetch` for that).

use super::ToolCtx;
use crate::internet::scrapion_run;
use anyhow::Result;
use serde_json::{json, Value};

/// Full-mode browser page tool.
pub struct WebPage;

impl super::Tool for WebPage {
    fn name(&self) -> &'static str {
        "web_page"
    }

    fn description(&self) -> &'static str {
        "Fetch a web page via a headless browser (renders JavaScript, beats Cloudflare). \
         Full internet mode only — requires `koma --internet-fullmode-install` and \
         internet mode set to `full`. Returns markdown content."
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

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok("error: web_page requires internet mode `full`. \
                 Use `/internet full` to switch, or use `web_fetch` for simple mode."
                .to_string());
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: web_page requires the internet research environment. \
                 Run `koma --internet-fullmode-install` to install it, or use `web_fetch`."
                    .to_string(),
            );
        }

        // ── Subprocess call ─────────────────────────────────────────────
        let stdout = scrapion_run(&["page", "--url", url]).map_err(|e| anyhow::anyhow!("{e}"))?;

        // Parse the JSON response.
        let report: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("scrapion produced invalid JSON: {e}"))?;

        // Check for error at top level.
        if let Some(err) = report.get("error").and_then(Value::as_str) {
            if !err.trim().is_empty() {
                return Ok(format!("web_page error: {}", err.trim()));
            }
        }

        let status = report.get("status").and_then(Value::as_str).unwrap_or("");
        if status != "success" {
            let err = report
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Ok(format!("web_page error: {err}"));
        }

        let content = report.get("content").and_then(Value::as_str).unwrap_or("");
        let title = report.get("title").and_then(Value::as_str).unwrap_or("");

        if content.trim().is_empty() {
            return Ok(format!("web_page: page returned empty content for {url}"));
        }

        // Cap content.
        const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
        let trimmed = content.trim();
        let (body, truncated) = if trimmed.chars().count() > MAX_CHARS {
            let cut: String = trimmed.chars().take(MAX_CHARS).collect();
            (cut, true)
        } else {
            (trimmed.to_string(), false)
        };

        let mut out = format!("source: {url}");
        if !title.is_empty() {
            out.push_str(&format!("\ntitle: {title}"));
        }
        out.push_str(&format!("\n\n{body}"));
        if truncated {
            out.push_str(&format!("\n\n... (content truncated at {MAX_CHARS} chars)"));
        }
        Ok(out)
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
    fn web_page_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"url": "https://example.com"});
        let result = WebPage.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn web_page_rejects_bad_url() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"url": "ftp://example.com"});
        let result = WebPage.run(&ctx, &args).unwrap();
        assert!(result.contains("must start with http"), "{result}");
    }

    #[test]
    fn web_page_missing_url_arg() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({});
        let result = WebPage.run(&ctx, &args);
        assert!(result.is_err(), "should error on missing url");
    }

    #[test]
    fn web_page_metadata() {
        assert_eq!(WebPage.name(), "web_page");
        assert!(!WebPage.description().is_empty());
        let params = WebPage.parameters();
        assert_eq!(params["required"], json!(["url"]));
    }
}
