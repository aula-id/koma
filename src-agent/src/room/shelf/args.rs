//! Argument helpers for shelf tools. Models send loose JSON, so every field is
//! checked and the error names the field.

use serde_json::{Map, Value};

use super::ShelfError;

pub type Args = Map<String, Value>;

pub const MAX_CONTENT: usize = 128 * 1024;

pub fn fail<T>(message: impl Into<String>) -> Result<T, ShelfError> {
    Err(ShelfError(message.into()))
}

/// Shelf keys: 1..128 chars of letters, digits and `. _ : / -`, starting with a letter or digit.
pub fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

pub fn require_key(value: Option<&Value>) -> Result<String, ShelfError> {
    match value {
        Some(Value::String(key)) if valid_key(key) => Ok(key.clone()),
        _ => fail("Key must be 1..128 letters, digits or . _ : / -"),
    }
}

/// An optional string field. Missing, null and "" all mean "not given".
pub fn text(
    args: &Args,
    name: &str,
    required: bool,
    max_len: usize,
) -> Result<Option<String>, ShelfError> {
    match args.get(name) {
        None | Some(Value::Null) => missing(name, required),
        Some(Value::String(s)) if s.is_empty() => missing(name, required),
        Some(Value::String(s)) if s.chars().count() <= max_len => Ok(Some(s.clone())),
        _ => fail(format!("{name} must be a string up to {max_len} chars")),
    }
}

fn missing(name: &str, required: bool) -> Result<Option<String>, ShelfError> {
    if required {
        fail(format!("{name} is required"))
    } else {
        Ok(None)
    }
}

/// A required string field.
pub fn text_req(args: &Args, name: &str, max_len: usize) -> Result<String, ShelfError> {
    text(args, name, true, max_len)?.map_or_else(|| fail(format!("{name} is required")), Ok)
}

pub fn string_list(args: &Args, name: &str, max_items: usize) -> Result<Vec<String>, ShelfError> {
    let items = match args.get(name) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) if items.len() <= max_items => items,
        _ => return fail(format!("{name} must be an array of at most {max_items} strings")),
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(s) if !s.is_empty() && s.chars().count() <= 512 => Ok(s.clone()),
            _ => fail(format!("{name} entries must be non-empty strings")),
        })
        .collect()
}

/// Lists such as ports that may mix numbers and short strings.
pub fn mixed_list(args: &Args, name: &str, max_items: usize) -> Result<Vec<Value>, ShelfError> {
    let items = match args.get(name) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) if items.len() <= max_items => items,
        _ => return fail(format!("{name} must be an array of at most {max_items} values")),
    };
    items
        .iter()
        .map(|item| match item {
            Value::Number(_) => Ok(item.clone()),
            Value::String(s) if !s.trim().is_empty() && s.chars().count() <= 64 => {
                Ok(Value::String(s.trim().to_string()))
            }
            _ => fail(format!("{name} entries must be numbers or short strings")),
        })
        .collect()
}

pub fn optional_unix(value: Option<&Value>) -> Result<Option<i64>, ShelfError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() && f >= 0.0 => Ok(Some(f.floor() as i64)),
            _ => fail("last_seen must be a unix timestamp"),
        },
        Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s
            .parse::<i64>()
            .map(Some)
            .or_else(|_| fail("last_seen must be a unix timestamp")),
        _ => fail("last_seen must be a unix timestamp"),
    }
}

pub fn optional_cvss(value: Option<&Value>) -> Result<Option<f64>, ShelfError> {
    let score = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s.is_empty() => return Ok(None),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match score {
        Some(s) if s.is_finite() && (0.0..=10.0).contains(&s) => Ok(Some((s * 10.0).round() / 10.0)),
        _ => fail("cvss_score must be a number 0..10"),
    }
}

pub fn require_positive_int(value: Option<&Value>, name: &str) -> Result<u64, ShelfError> {
    match value.and_then(Value::as_u64) {
        Some(n) if n >= 1 => Ok(n),
        _ => fail(format!("{name} must be a positive integer")),
    }
}

pub fn expect_revision(value: Option<&Value>) -> Result<i64, ShelfError> {
    match value.and_then(Value::as_u64) {
        Some(n) => Ok(n as i64),
        None => fail("expected_revision is required; use 0 to create, or the revision returned by get"),
    }
}

pub fn assert_size(content: &str) -> Result<(), ShelfError> {
    if content.len() > MAX_CONTENT {
        return fail("Content must be text of at most 128 KiB");
    }
    Ok(())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse stored content as a JSON object, or `None` if it is anything else.
pub fn parse_object(content: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}
