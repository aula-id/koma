//! Bounded exact reads of current-session and registered project archives.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

// DRSS preserves both message_load and legacy message_find read results.
const MAX_PAGE_CHARS: i64 = 3000;

/// Session-qualified references resolve only to the current session or a
/// registered sibling in the same canonical project bucket, never a path
/// supplied by the model. Legacy DRSS IDs remain current-session-only.
pub(super) fn load(session_dir: &Path, args: &Value) -> Result<String> {
    ensure!(args.is_object(), "arguments must be an object");
    for key in [
        "query", "role", "scope", "skip", "sort", "after", "before", "limit",
    ] {
        ensure!(
            args.get(key).is_none(),
            "{key} belongs to message_find; message_load uses ref, offset and max_chars"
        );
    }
    let mut read_args = args.clone();
    let object = read_args
        .as_object_mut()
        .context("arguments must be an object")?;
    if let Some(max_chars) = object.remove("max_chars") {
        object.insert("limit".into(), max_chars);
    }
    let mut target = session_dir.to_path_buf();
    if let Some(reference) = object.remove("ref") {
        ensure!(
            object.get("message_id").is_none() && object.get("archive_key").is_none(),
            "pass exactly one of ref, message_id or archive_key"
        );
        let reference = reference.as_str().context("ref must be a string")?;
        let parts: Vec<_> = reference.split(':').collect();
        ensure!(
            parts.len() == 3,
            "invalid ref; copy the complete ref from message_find"
        );
        let session = parts[0];
        ensure!(
            !session.is_empty()
                && session.len() <= 128
                && session
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
            "invalid session in ref"
        );
        match parts[1] {
            "message" => {
                let id = parts[2]
                    .parse::<i64>()
                    .context("invalid message ID in ref")?;
                ensure!(id > 0, "message ID must be positive");
                object.insert("message_id".into(), json!(id));
            }
            "recovery" => {
                object.insert("archive_key".into(), json!(parts[2]));
            }
            _ => bail!("invalid ref source; copy ref from message_find"),
        }
        if session != super::session_uuid_from_dir(session_dir) {
            let targets = super::resolve_targets(session_dir, super::SearchScope::Project)?;
            target = registered_target(session_dir, session, &targets)?;
        }
    }
    ensure!(
        target.join("messages.sqlite").is_file(),
        "message archive not found"
    );
    let output = read(&target, &read_args)?;
    let (header, content) = output.split_once('\n').context("invalid archive page")?;
    let mut header: Value = serde_json::from_str(header)?;
    let session = super::session_uuid_from_dir(&target);
    let reference = if let Some(id) = header.get("message_id") {
        format!("{session}:message:{id}")
    } else {
        format!(
            "{session}:recovery:{}",
            header["archive_key"].as_str().unwrap_or_default()
        )
    };
    header["ref"] = json!(reference);
    // Keep attachment hints out of discovery results. Only the selected page
    // can supply hints, and even those have a separate fixed metadata budget.
    let mut hints = String::new();
    super::append_image_reload_lines(&mut hints, &target, content);
    super::append_paste_reload_lines(&mut hints, &target, content);
    if !hints.is_empty() {
        header["attachment_hints"] = json!(super::floor_chars(&hints, 1000));
    }
    header["note"] = json!("Historical message content; follow next_offset only if more text is needed. null timestamp means original time unknown.");
    Ok(format!("{header}\n{content}"))
}

pub(super) fn registered_target(
    session_dir: &Path,
    session: &str,
    targets: &[super::SearchTarget],
) -> Result<std::path::PathBuf> {
    let target = &targets
        .iter()
        .find(|t| t.uuid == session)
        .context("ref is not from the current session or a registered session in this project")?
        .path;
    let current = session_dir.canonicalize()?;
    let resolved = target.canonicalize()?;
    ensure!(
        resolved.parent() == current.parent(),
        "ref is outside this project's session bucket"
    );
    Ok(resolved)
}

pub(super) fn read(session_dir: &Path, args: &Value) -> Result<String> {
    ensure!(args.is_object(), "arguments must be an object");
    if args.get("query").is_some() {
        bail!("pass exactly one of query, message_id or archive_key");
    }
    for key in [
        "ref",
        "role",
        "skip",
        "sort",
        "after",
        "before",
        "max_chars",
    ] {
        ensure!(
            args.get(key).is_none(),
            "{key} is not valid for a legacy exact read; use message_load for selected refs"
        );
    }
    if super::parse_scope(super::options::string(args, "scope")?)? != super::SearchScope::Session {
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
    let timestamp = if table == "messages" {
        "strftime('%Y-%m-%dT%H:%M:%SZ',created_at,'unixepoch')"
    } else {
        "NULL"
    };
    let row: Option<(String, String, i64, bool, Option<String>)> = conn
        .query_row(
            &format!(
                "SELECT role,
                    CASE WHEN instr(content, char(0)) > 0 THEN content
                         ELSE substr(content, ?2, ?3) END,
                    length(content), instr(content, char(0)) > 0, {timestamp}
             FROM {table} WHERE {column} = ?1"
            ),
            rusqlite::params![identifier, start, limit],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (role, mut content, mut total_chars, has_nul, timestamp) =
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
        "timestamp": timestamp,
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
