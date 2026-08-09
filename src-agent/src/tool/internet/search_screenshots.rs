//! `search_screenshots` tool — search the screenshot catalog by keyword.

use super::ToolCtx;
use crate::model::screenshot_catalog;
use anyhow::Result;
use serde_json::{json, Value};

/// Search the screenshot catalog by keyword (read-only).
pub struct SearchScreenshots;

impl super::Tool for SearchScreenshots {
    fn name(&self) -> &'static str {
        "search_screenshots"
    }

    fn description(&self) -> &'static str {
        "Search the screenshot catalog by description, tags, or URL. Returns matching \
         screenshots sorted by relevance."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search terms to match against screenshot descriptions, tags, and URLs"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default 10, max 50)"
                }
            },
            "required": ["query"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'query'"))?;

        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).min(50))
            .unwrap_or(10);

        let total = screenshot_catalog::list_records(&ctx.workspace).len();
        if total == 0 {
            return Ok("no screenshots captured yet — use `web_screenshot` to capture one"
                .to_string());
        }

        let results = screenshot_catalog::search_records(&ctx.workspace, query, max_results);
        if results.is_empty() {
            return Ok(format!("no screenshots matching '{query}'"));
        }

        let mut out = String::new();
        for (i, r) in results.iter().enumerate() {
            let date_part = &r.captured[..10.min(r.captured.len())];
            let desc_excerpt = if r.description.len() > 100 {
                format!("{}…", &r.description[..100])
            } else {
                r.description.clone()
            };
            out.push_str(&format!(
                "{}. {}.png — {} — {} — {}\n",
                i + 1,
                r.stem,
                r.url,
                date_part,
                desc_excerpt,
            ));
        }
        Ok(out)
    }
}
