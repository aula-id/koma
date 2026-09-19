//! Bounded, current-session message reads for exact archive recall.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

// Keep retrieval results below DRSS's heavy-message threshold so reading a
// stub's target does not immediately produce another stub of the same result.
const MAX_PAGE_CHARS: i64 = 3000;

pub(super) fn read(session_dir: &Path, args: &Value) -> Result<String> {
    if args.get("query").is_some() {
        bail!("pass message_id or query, not both");
    }
    if super::parse_scope(args.get("scope").and_then(Value::as_str))? != super::SearchScope::Session
    {
        bail!("message_id is scoped to the current session; project ids are ambiguous");
    }
    let id = args
        .get("message_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .context("message_id must be a positive integer")?;
    let integer = |key: &str, default: i64| -> Result<i64> {
        match args.get(key) {
            None => Ok(default),
            Some(v) => v
                .as_i64()
                .with_context(|| format!("{key} must be an integer")),
        }
    };
    let offset = integer("offset", 0)?;
    let limit = integer("limit", MAX_PAGE_CHARS)?;
    if offset < 0 || !(1..=MAX_PAGE_CHARS).contains(&limit) {
        bail!("offset must be nonnegative and limit must be between 1 and {MAX_PAGE_CHARS}");
    }
    let start = offset.checked_add(1).context("offset is too large")?;
    let conn = crate::model::msglog::open(session_dir)?;
    // SQLite text slicing stops at embedded NUL. Use its bounded character
    // slice normally, with a Rust character-slice fallback for that rare case.
    // Never read the separate reasoning column.
    let row: Option<(String, String, i64, bool)> = conn
        .query_row(
            "SELECT role,
                    CASE WHEN instr(content, char(0)) > 0 THEN content
                         ELSE substr(content, ?2, ?3) END,
                    length(content), instr(content, char(0)) > 0
             FROM messages WHERE id = ?1",
            rusqlite::params![id, start, limit],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (role, mut content, mut total_chars, has_nul) =
        row.context("message_id not found in the current session")?;
    if has_nul {
        total_chars = content.chars().count() as i64;
        let offset = usize::try_from(offset).context("offset is too large")?;
        content = content.chars().skip(offset).take(limit as usize).collect();
    }
    if offset > total_chars {
        bail!("offset exceeds message length ({total_chars} characters)");
    }
    let end = offset + content.chars().count() as i64;
    let header = json!({
        "message_id": id,
        "role": role,
        "offset": offset,
        "total_chars": total_chars,
        "next_offset": if end < total_chars { Some(end) } else { None },
    });
    // Plain content avoids JSON escaping expanding a 3000-character page past
    // the stub threshold (for example a page containing many newlines).
    Ok(format!("{header}\n{content}"))
}
