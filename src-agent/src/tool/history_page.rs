//! Bounded, current-session message reads for exact archive recall.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

// Exact reads are bounded; DRSS preserves message_find results on the live rail.
const MAX_PAGE_CHARS: i64 = 3000;

pub(super) fn read(session_dir: &Path, args: &Value) -> Result<String> {
    if args.get("query").is_some() {
        bail!("pass exactly one of query, message_id or archive_key");
    }
    if super::parse_scope(args.get("scope").and_then(Value::as_str))? != super::SearchScope::Session
    {
        bail!("exact archive reads are scoped to the current session; project ids are ambiguous");
    }
    let key = args.get("archive_key");
    if key.is_some() && args.get("message_id").is_some() {
        bail!("pass exactly one of query, message_id or archive_key");
    }
    let (table, column, identifier, reference) = if let Some(key) = key {
        let key = key
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .context("archive_key must be a 64-character recovery key")?;
        (
            "drss_recovery",
            "archive_key",
            rusqlite::types::Value::Text(key.to_string()),
            json!({"archive_key": key}),
        )
    } else {
        let id = args
            .get("message_id")
            .and_then(Value::as_i64)
            .filter(|id| *id > 0)
            .context("message_id must be a positive integer")?;
        (
            "messages",
            "id",
            rusqlite::types::Value::Integer(id),
            json!({"message_id": id}),
        )
    };
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
            &format!(
                "SELECT role,
                    CASE WHEN instr(content, char(0)) > 0 THEN content
                         ELSE substr(content, ?2, ?3) END,
                    length(content), instr(content, char(0)) > 0
             FROM {table} WHERE {column} = ?1"
            ),
            rusqlite::params![identifier, start, limit],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (role, mut content, mut total_chars, has_nul) =
        row.context("archive reference not found in the current session")?;
    if has_nul {
        total_chars = content.chars().count() as i64;
        let offset = usize::try_from(offset).context("offset is too large")?;
        content = content.chars().skip(offset).take(limit as usize).collect();
    }
    if offset > total_chars {
        bail!("offset exceeds message length ({total_chars} characters)");
    }
    let end = offset + content.chars().count() as i64;
    let mut header = json!({
        "role": role,
        "offset": offset,
        "total_chars": total_chars,
        "next_offset": if end < total_chars { Some(end) } else { None },
    });
    header
        .as_object_mut()
        .unwrap()
        .extend(reference.as_object().unwrap().clone());
    // Plain content avoids JSON escaping expanding a 3000-character page past
    // the stub threshold (for example a page containing many newlines).
    Ok(format!("{header}\n{content}"))
}
