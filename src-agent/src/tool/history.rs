//! Chat history search tool: `message_find` queries the session's FTS5
//! full-text index (`messages_fts`) so the model can look up past conversation
//! turns that have scrolled out of the context window.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use super::{Tool, ToolCtx};
use crate::model::msglog::search_messages;

/// Search the session's `messages.sqlite` full-text index for past
/// conversation turns matching the query. Returns ranked snippets.
pub struct MessageFind;

impl Tool for MessageFind {
    fn name(&self) -> &'static str {
        "message_find"
    }

    fn description(&self) -> &'static str {
        "Search the current session's chat history (messages.sqlite) for past \
         conversation turns matching the query. Uses full-text search (FTS5). \
         Returns up to 10 ranked snippets with message id, role, and content \
         context. Use this to recall past decisions, tradeoffs, error messages, \
         or facts discussed earlier in the conversation that may have scrolled \
         out of the context window."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search terms to find in past messages. Supports FTS5 syntax: multi-word AND, quoted phrases, etc."
                }
            },
            "required": ["query"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'query'"))?;

        let session_dir = match ctx.session_dir.as_ref() {
            Some(d) => d,
            None => bail!("no active session to search"),
        };

        let matches = search_messages(session_dir, query, 10);

        if matches.is_empty() {
            return Ok("(no matching messages found)".to_string());
        }

        let mut out = String::new();
        for m in &matches {
            let role_prefix = match m.role.as_str() {
                "user" => "user",
                "assistant" => "assistant",
                "tool" => "tool",
                "system" => "system",
                _ => "?",
            };
            // snippet() uses '' as markers (no highlighting), so we just show
            // the context fragment. Trim whitespace for compact output.
            let snip = m.snippet.trim();
            out.push_str(&format!("[{}] {}: {}\n", m.id, role_prefix, snip));
        }

        Ok(out)
    }
}
