//! `describe_screenshot` tool — write description/tags to a screenshot catalog record.

use super::ToolCtx;
use crate::model::screenshot_catalog;
use anyhow::Result;
use serde_json::{json, Value};

/// Add or update a screenshot's description and tags.
pub struct DescribeScreenshot;

impl super::Tool for DescribeScreenshot {
    fn name(&self) -> &'static str {
        "describe_screenshot"
    }

    fn description(&self) -> &'static str {
        "Add or update a visual description and tags for an existing screenshot in the catalog."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "screenshot": {
                    "type": "string",
                    "description": "Screenshot stem or filename to describe"
                },
                "description": {
                    "type": "string",
                    "description": "Concise visual description of the screenshot content"
                },
                "tags": {
                    "type": "string",
                    "description": "Comma-separated tags (optional)"
                }
            },
            "required": ["screenshot", "description"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let name = args
            .get("screenshot")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'screenshot'"))?;

        let description = args
            .get("description")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'description'"))?;

        let tags = args
            .get("tags")
            .and_then(Value::as_str)
            .unwrap_or("");

        // Resolve the screenshot path for validation.
        let resolved = screenshot_catalog::resolve_screenshot_path(&ctx.workspace, name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "screenshot '{name}' not found or not a valid PNG under .screenshoot/"
                )
            })?;

        // Extract stem from the resolved path.
        let stem = resolved
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        screenshot_catalog::update_description(&ctx.workspace, stem, description, tags)?;

        let preview = if description.len() > 80 {
            format!("{}…", &description[..80])
        } else {
            description.to_string()
        };
        Ok(format!(
            "describe_screenshot: updated {stem} — \"{preview}\""
        ))
    }
}
