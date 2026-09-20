//! Small, provider-independent search arguments with explicit runtime validation.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

use crate::model::msglog::history_search::{self, Order, Search};

pub(super) struct Options {
    pub search: Search,
    pub scope: super::SearchScope,
    pub skip: usize,
    pub limit: usize,
}

pub(super) fn string<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        _ => bail!("{name} must be a string"),
    }
}

impl Options {
    pub fn parse(args: &Value) -> Result<Self> {
        ensure!(args.is_object(), "arguments must be an object");
        let query = string(args, "query")?.map(str::trim).map(str::to_owned);
        if let Some(query) = &query {
            ensure!(
                query.len() <= 2000,
                "query must be at most 2000 bytes; use up to 5 precise words"
            );
            ensure!(
                history_search::fts_query(query).is_some(),
                "query needs a word of at least 2 characters"
            );
        }
        let role = string(args, "role")?.map(str::to_owned);
        ensure!(
            role.as_deref()
                .is_none_or(|r| matches!(r, "user" | "assistant" | "tool")),
            "role must be user, assistant or tool"
        );
        let after = string(args, "after")?.map(timestamp).transpose()?;
        let before = string(args, "before")?.map(timestamp).transpose()?;
        ensure!(
            query.is_some() || role.is_some() || after.is_some() || before.is_some(),
            "provide query, role, after or before to search or browse messages"
        );
        if let (Some(after), Some(before)) = (after, before) {
            ensure!(after < before, "after must be earlier than before");
        }
        let order = match string(args, "sort")?.unwrap_or("latest") {
            "latest" => Order::Latest,
            "oldest" => Order::Oldest,
            "relevance" => Order::Relevance,
            _ => bail!("sort must be latest, oldest or relevance"),
        };
        ensure!(
            order != Order::Relevance || query.is_some(),
            "relevance sorting requires query"
        );
        let integer = |name: &str, default: usize, min: usize, max: usize| -> Result<usize> {
            let n = match args.get(name) {
                None => default,
                Some(value) => value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .with_context(|| format!("{name} must be an integer"))?,
            };
            ensure!(
                (min..=max).contains(&n),
                "{name} must be between {min} and {max}"
            );
            Ok(n)
        };
        ensure!(
            args.get("offset").is_none(),
            "use skip for search results; offset belongs to message_load"
        );
        Ok(Self {
            search: Search {
                query,
                role,
                after,
                before,
                order,
            },
            scope: super::parse_scope(string(args, "scope")?)?,
            skip: integer("skip", 0, 0, 10_000)?,
            limit: integer("limit", 10, 1, 20)?,
        })
    }
}

/// Explicit seconds and timezone keep date/hour filters unambiguous across
/// models and machines. Validate the calendar before using SQLite's converter.
fn timestamp(value: &str) -> Result<i64> {
    let invalid = || {
        anyhow::anyhow!(
            "time must be YYYY-MM-DDTHH:MM:SSZ or YYYY-MM-DDTHH:MM:SS+HH:MM (explicit timezone)"
        )
    };
    let b = value.as_bytes();
    ensure!(
        value.is_ascii() && matches!(b.len(), 20 | 25),
        "{}",
        invalid()
    );
    ensure!(
        b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':',
        "{}",
        invalid()
    );
    let number = |start, end| -> Result<i64> {
        let part = &value[start..end];
        ensure!(part.bytes().all(|c| c.is_ascii_digit()), "{}", invalid());
        part.parse().map_err(|_| invalid())
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    ensure!(
        (1..=9999).contains(&year) && (1..=12).contains(&month),
        "invalid calendar date"
    );
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][month as usize - 1];
    ensure!(
        (1..=days).contains(&day)
            && number(11, 13)? < 24
            && number(14, 16)? < 60
            && number(17, 19)? < 60,
        "invalid calendar date or clock time"
    );
    if b.len() == 20 {
        ensure!(b[19] == b'Z', "{}", invalid());
    } else {
        ensure!(
            matches!(b[19], b'+' | b'-')
                && b[22] == b':'
                && number(20, 22)? <= 14
                && number(23, 25)? < 60,
            "{}",
            invalid()
        );
        ensure!(
            number(20, 22)? != 14 || number(23, 25)? == 0,
            "timezone offset exceeds 14 hours"
        );
    }
    let conn = rusqlite::Connection::open_in_memory()?;
    let seconds: Option<i64> = conn.query_row(
        "SELECT CAST(strftime('%s',?1) AS INTEGER)",
        [value],
        |row| row.get(0),
    )?;
    seconds.ok_or_else(invalid)
}
