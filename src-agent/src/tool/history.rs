//! Chat history search tool: `message_find` queries the session's chat
//! history. Tries SurrealDB first (hybrid search when synced, FTS-only
//! fallback when not), then falls back to SQLite FTS5.
//!
//! The SurrealDB layer is a fire-and-forget background sync from the
//! SQLite message log. Until the sync completes, SurrealDB returns empty
//! or partial results — the tool transparently falls back to SQLite FTS5
//! in that case, logging "Surreal Empty" to error.log for observability.

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

        // Try SurrealDB first. If it returns results, use those.
        // If not (sync hasn't completed, or DB doesn't exist), fall back
        // to SQLite FTS5 without logging any error — this is normal
        // operation during sync.
        let matches = {
            let surreal_hits =
                crate::model::surreal::search_messages(session_dir, query, 10, role_filter);
            if !surreal_hits.is_empty() {
                format_matches(
                    surreal_hits
                        .iter()
                        .map(|m| (m.id, m.role.as_str(), m.snippet.as_str(), m.created_at)),
                )
            } else {
                // SurrealDB returned empty — log and transparent fallback to SQLite FTS5.
                crate::model::store::append_global_error_log(
                    "Surreal Empty",
                    &format!("search_messages returned 0 hits for query: {query}"),
                );
                let fts5_hits =
                    crate::model::msglog::search_messages(session_dir, query, 10, role_filter);
                format_matches(
                    fts5_hits
                        .iter()
                        .map(|m| (m.id, m.role.as_str(), m.snippet.as_str(), m.created_at)),
                )
            }
        };

        if matches.is_empty() {
            return Ok("(no matching messages found)".to_string());
        }
        Ok(matches)
    }
}

fn format_matches<'a>(matches: impl Iterator<Item = (i64, &'a str, &'a str, i64)>) -> String {
    let mut out = String::new();
    for (msg_id, role, content, _created_at) in matches {
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
    }
    out
}
