//! Chat history search tool: `message_find` queries the session's chat
//! history via SQLite FTS5 full-text search on `messages.sqlite`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Duration;

use super::{Tool, ToolCtx};
use anyhow::{bail, Result};
use serde_json::{json, Value};

/// Hard wall-clock budget for one search. On timeout the turn unparks with an
/// error and a deterministic FTS/panic diagnosis + repair runs (no AI).
const MESSAGE_FIND_TIMEOUT: Duration = Duration::from_secs(20);

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
         Query is limited to 5 words (extra terms dropped). Times out at 20s. \
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
                    "description": "At most 5 search words (extra words ignored). Multi-word queries are OR'd as prefix matches (e.g. \"foo bar\" → foo* OR bar*). Prefer short precise terms."
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
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let session_dir = match ctx.session_dir.as_ref() {
            Some(d) => d.clone(),
            None => bail!("no active session to search"),
        };
        let session_dir_for_repair = session_dir.clone();

        let query_owned = query.to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("message-find".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    crate::model::msglog::search_messages(
                        &session_dir,
                        &query_owned,
                        10,
                        role_filter.as_deref(),
                    )
                }));
                let _ = tx.send(outcome);
            })
            .map_err(|e| anyhow::anyhow!("message_find spawn failed: {e}"))?;

        match rx.recv_timeout(MESSAGE_FIND_TIMEOUT) {
            Ok(Ok(Ok(matches))) => {
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
            Ok(Ok(Err(e))) => {
                // Surface DB/FTS errors instead of mapping them to "no matches".
                Err(anyhow::anyhow!("message_find failed: {e}"))
            }
            Ok(Err(payload)) => {
                let msg = panic_payload_message(&payload);
                crate::model::store::append_global_error_log(
                    "message_find",
                    &format!("worker panic: {msg}"),
                );
                let repair = crate::model::msglog::diagnose_and_repair_message_find(
                    &session_dir_for_repair,
                );
                Err(anyhow::anyhow!(
                    "message_find panicked: {msg}\n{repair}"
                ))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                crate::model::store::append_global_error_log(
                    "message_find",
                    "timed out after 20s — running deterministic diagnosis/repair",
                );
                // Worker may still be running; abandon it and repair the archive.
                let repair = crate::model::msglog::diagnose_and_repair_message_find(
                    &session_dir_for_repair,
                );
                Err(anyhow::anyhow!(
                    "message_find timed out after 20s\n{repair}\n\
                     (retry with ≤5 precise words if needed)"
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow::anyhow!("message_find worker dropped without a result"))
            }
        }
    }
}

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "unknown panic payload".into()
}

/// Truncate on a char boundary so multi-byte UTF-8 never panics the deferred
/// tool thread (which would leave the round stuck on a running message_find).
fn floor_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
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
        let snippet = floor_chars(content.trim(), 300);
        out.push_str(&format!("{} #{}: {}\n\n", role_prefix, msg_id, snippet));
        // Append a thinking snippet for assistant messages that have reasoning.
        if let Some(thinking) = reasoning {
            let thinking = thinking.trim();
            if !thinking.is_empty() {
                let t = floor_chars(thinking, 300);
                out.push_str(&format!("  thinking: {}\n\n", t));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::floor_chars;

    #[test]
    fn floor_chars_does_not_split_multibyte_at_boundary() {
        // Build a string where a multi-byte char sits near the cut.
        let mut s = String::new();
        while s.len() < 298 {
            s.push('a');
        }
        s.push('─'); // 3-byte box drawing (U+2500)
        s.push_str("tail");
        let cut = floor_chars(&s, 300);
        assert!(cut.is_char_boundary(cut.len()));
        assert!(!cut.ends_with('\u{FFFD}'));
        // 298 'a's + maybe the box char if counted as one char within 300.
        assert!(cut.chars().count() <= 300);
    }
}
