//! `web_search_full` tool — Full-mode browser search.
//!
//! Submits a search URL to the scrapion browser backend (`python -m
//! scrapion_agent search --url <SEARCH_URL>`) which navigates in a real
//! Firefox, renders JavaScript, and extracts result titles / links / snippets.
//!
//! **Full mode only**: requires `InternetMode::Full` *and* the research
//! environment to be installed. Returns a clear error if either gate is not met
//! — there is **no** raw-HTTP fallback (use `web_search` for that).
//!
//! The caller supplies a fully-formed search URL (e.g.
//! `https://html.duckduckgo.com/html/?q=...`).  Use `web_search` (simple mode)
//! when you only have a query string.

use super::ToolCtx;
use crate::internet::scrapion_run;
use anyhow::Result;
use serde_json::{json, Value};

/// Full-mode browser search tool.
pub struct WebSearchFull;

impl super::Tool for WebSearchFull {
    fn name(&self) -> &'static str {
        "web_search_full"
    }

    fn description(&self) -> &'static str {
        "Search the web via a headless browser (renders JavaScript, beats Cloudflare). \
         Full internet mode only — requires `koma --internet-fullmode-install` and \
         internet mode set to `full`. Returns structured search results with titles, \
         links, and snippets. Supply either a `query` (the tool builds the URL using \
         the session's preferred search engine) or a fully-formed `url`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "A search query string (e.g. 'rust async runtime'). The tool builds the search URL using the session's preferred search engine."
                },
                "url": {
                    "type": "string",
                    "description": "A fully-formed search URL (query already embedded, e.g. https://html.duckduckgo.com/html/?q=hello). Overrides the preferred search engine."
                }
            }
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let url = if let Some(u) = args.get("url").and_then(Value::as_str) {
            if !u.starts_with("http://") && !u.starts_with("https://") {
                return Ok(format!(
                    "error: url must start with http:// or https://, got: {u}"
                ));
            }
            u.to_string()
        } else if let Some(q) = args.get("query").and_then(Value::as_str) {
            if q.trim().is_empty() {
                return Ok("error: query must not be empty".to_string());
            }
            let template = ctx
                .search_engine
                .as_deref()
                .unwrap_or(crate::model::settings::DEFAULT_SEARCH_ENGINE);
            match crate::model::settings::build_search_url(template, q) {
                Ok(u) => u,
                Err(e) => return Ok(format!("error: could not build search URL: {e}")),
            }
        } else {
            return Ok("error: provide either 'query' or 'url'".to_string());
        };

        // ── Full-mode gate ──────────────────────────────────────────────
        if ctx.internet_mode != crate::model::settings::InternetMode::Full {
            return Ok("error: web_search_full requires internet mode `full`. \
                 Use `/internet full` to switch, or use `web_search` for simple mode."
                .to_string());
        }
        if !crate::internet::is_installed() {
            return Ok(
                "error: web_search_full requires the internet research environment. \
                 Run `koma --internet-fullmode-install` to install it, or use `web_search`."
                    .to_string(),
            );
        }

        // ── Subprocess call ─────────────────────────────────────────────
        let stdout =
            scrapion_run(&["search", "--url", &url]).map_err(|e| anyhow::anyhow!("{e}"))?;

        let report: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("scrapion produced invalid JSON: {e}"))?;

        if let Some(err) = report.get("error").and_then(Value::as_str) {
            if !err.trim().is_empty() {
                return Ok(format!("web_search_full error: {}", err.trim()));
            }
        }

        let status = report.get("status").and_then(Value::as_str).unwrap_or("");
        if status != "success" {
            let err = report
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Ok(format!("web_search_full error: {err}"));
        }

        let results = match report.get("results").and_then(Value::as_array) {
            Some(arr) => arr,
            None => {
                return Ok(format!(
                    "web_search_full: no results array in response for {url}"
                ));
            }
        };

        if results.is_empty() {
            return Ok(format!("web_search_full: no results found for {url}"));
        }

        let mut out = String::new();
        for (i, r) in results.iter().enumerate() {
            let title = r.get("title").and_then(Value::as_str).unwrap_or("");
            let link = r.get("link").and_then(Value::as_str).unwrap_or("");
            let snippet = r.get("snippet").and_then(Value::as_str).unwrap_or("");
            if title.is_empty() && link.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "{}. {}\n   {}\n   {}\n",
                i + 1,
                title,
                link,
                snippet
            ));
        }

        if out.is_empty() {
            return Ok(format!("web_search_full: no extractable results for {url}"));
        }
        Ok(out.trim_end().to_string())
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
    fn web_search_full_rejects_simple_mode() {
        let ctx = make_ctx(InternetMode::Simple);
        let args = json!({"url": "https://html.duckduckgo.com/html/?q=hello"});
        let result = WebSearchFull.run(&ctx, &args).unwrap();
        assert!(result.contains("requires internet mode `full`"), "{result}");
    }

    #[test]
    fn web_search_full_rejects_bad_url() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"url": "ftp://example.com"});
        let result = WebSearchFull.run(&ctx, &args).unwrap();
        assert!(result.contains("must start with http"), "{result}");
    }

    #[test]
    fn web_search_full_missing_both_args() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({});
        let result = WebSearchFull.run(&ctx, &args).unwrap();
        assert!(result.contains("provide either"), "{result}");
    }

    #[test]
    fn web_search_full_empty_query() {
        let ctx = make_ctx(InternetMode::Full);
        let args = json!({"query": "  "});
        let result = WebSearchFull.run(&ctx, &args).unwrap();
        assert!(result.contains("must not be empty"), "{result}");
    }

    #[test]
    fn web_search_full_query_uses_default_engine() {
        // Use Simple mode so the mode gate fires BEFORE any network call,
        // making this test deterministic regardless of scrapion install state.
        // The URL is built from the query using the default engine template
        // (search_engine = None) before the gate check — if the URL building
        // failed, we'd get a different error.
        let mut ctx = make_ctx(InternetMode::Simple);
        ctx.search_engine = None;
        let args = json!({"query": "rust async"});
        let result = WebSearchFull.run(&ctx, &args).unwrap();
        assert!(
            result.contains("requires internet mode `full`"),
            "expected mode gate error, got: {result}"
        );
    }

    #[test]
    fn web_search_full_metadata() {
        assert_eq!(WebSearchFull.name(), "web_search_full");
        assert!(!WebSearchFull.description().is_empty());
        let params = WebSearchFull.parameters();
        // No required params — either query or url is accepted.
        assert!(params.get("properties").is_some());
    }
}
