//! Chat history search tool: `message_find` queries the session's chat
//! history via SQLite FTS5 full-text search on `messages.sqlite`.

use super::{Tool, ToolCtx};
use anyhow::{bail, Result};
use serde_json::{json, Value};

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
         Returns up to 10 ranked results with message id, role, and the first \
         300 characters of the matching message for coherent context. \
         Optionally filter by role (user, assistant, tool). \
         Call this when you are confused or missing context about a \
         past decision, error, tradeoff, or fact that may have scrolled out of \
         the context window — before guessing. Also call it when the user \
         explicitly asks you to recall, look up, find, or check something from \
         earlier in the conversation."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search terms to find in past messages. Multi-word queries are OR'd as prefix matches (e.g. \"foo bar\" matches messages containing \"foo*\" or \"bar*\")."
                },
                "role": {
                    "type": "string",
                    "description": "Optional role filter: \"user\" for user messages, \"assistant\" for assistant messages, \"tool\" for tool results. Omit to search all roles.",
                    "enum": ["user", "assistant", "tool"]
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

        let role_filter = args
            .get("role")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());

        let session_dir = match ctx.session_dir.as_ref() {
            Some(d) => d,
            None => bail!("no active session to search"),
        };

        let matches = crate::model::msglog::search_messages(session_dir, query, 10, role_filter);
        let out = format_matches(matches.iter().map(|m| {
            (
                m.id,
                m.role.as_str(),
                m.snippet.as_str(),
                m.created_at,
                m.reasoning.as_deref(),
            )
        }));

        if out.is_empty() {
            return Ok("(no matching messages found)".to_string());
        }
        Ok(out)
    }
}

fn format_matches<'a>(
    matches: impl Iterator<Item = (i64, &'a str, &'a str, i64, Option<&'a str>)>,
) -> String {
    let mut out = String::new();
    for (msg_id, role, content, _created_at, reasoning) in matches {
        let role_prefix = match role {
            "user" => "[user]",
            "assistant" => "[assistant]",
            "tool" => "[tool]",
            "system" => "[system]",
            _ => "[?]",
        };
        // Strip leading/trailing whitespace and truncate to keep output dense.
        let snippet = content.trim();
        let snippet = if snippet.len() > 300 {
            &snippet[..300]
        } else {
            snippet
        };
        out.push_str(&format!("{} #{}: {}\n\n", role_prefix, msg_id, snippet));
        // Append a thinking snippet for assistant messages that have reasoning.
        if let Some(thinking) = reasoning {
            let thinking = thinking.trim();
            if !thinking.is_empty() {
                let t = if thinking.len() > 300 {
                    &thinking[..300]
                } else {
                    thinking
                };
                out.push_str(&format!("  thinking: {}\n\n", t));
            }
        }
    }
    out
}
