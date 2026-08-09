//! `load_screenshot` tool — load a screenshot PNG for visual inspection.
//!
//! The `run()` method validates and resolves the path. The ACTUAL image
//! attachment injection happens in the runtime interception layer
//! (`intercepts::screenshot`), because `Tool::run()` cannot produce
//! image-bearing tool results.

use super::ToolCtx;
use crate::model::screenshot_catalog;
use anyhow::Result;
use serde_json::{json, Value};

/// Load a screenshot for visual inspection.
pub struct LoadScreenshot;

impl super::Tool for LoadScreenshot {
    fn name(&self) -> &'static str {
        "load_screenshot"
    }

    fn description(&self) -> &'static str {
        "Load a screenshot PNG into the conversation for visual inspection. \
         The image will be attached as a visual message."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "screenshot": {
                    "type": "string",
                    "description": "Screenshot stem or filename to load for visual inspection"
                }
            },
            "required": ["screenshot"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let name = args
            .get("screenshot")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'screenshot'"))?;

        let resolved = screenshot_catalog::resolve_screenshot_path(&ctx.workspace, name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "screenshot '{name}' not found or not a valid PNG under .screenshoot/"
                )
            })?;

        let stem = resolved
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        Ok(format!(
            "load_screenshot: resolved {stem} → {}",
            resolved.display()
        ))
    }
}
