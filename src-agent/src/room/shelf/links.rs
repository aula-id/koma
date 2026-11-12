//! Referential rules between gate records: a claim's evidence and assets must
//! exist, evidence needs a parent claim, a verdict needs a claim, and a finding
//! needs a confirmed verdict for the same claim.

use rusqlite::Connection;
use serde_json::{Map, Value};

use super::args::{fail, parse_object};
use super::store::get_row;
use super::ShelfError;

fn str_list(record: &Map<String, Value>, name: &str) -> Vec<String> {
    record
        .get(name)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn str_field<'a>(record: &'a Map<String, Value>, name: &str) -> &'a str {
    record.get(name).and_then(Value::as_str).unwrap_or("")
}

/// The stored object under `key`, if it exists and is a record of `kind`.
fn typed_row(
    conn: &Connection,
    key: &str,
    kind: &str,
    missing: impl FnOnce() -> String,
) -> Result<Map<String, Value>, ShelfError> {
    let Some(row) = get_row(conn, key)? else {
        return fail(missing());
    };
    match parse_object(&row.content) {
        Some(body) if body.get("kind").and_then(Value::as_str) == Some(kind) => Ok(body),
        _ => fail(format!("{key} is not a {kind}")),
    }
}

pub fn assert_claim_links(
    conn: &Connection,
    kind: &str,
    key: &str,
    record: &Map<String, Value>,
) -> Result<(), ShelfError> {
    match kind {
        "claim" => {
            for evidence_key in str_list(record, "evidence_keys") {
                let body = typed_row(conn, &evidence_key, "evidence", || {
                    format!(
                        "evidence_keys entry missing: {evidence_key}. put_evidence first, or omit evidence_keys and attach later."
                    )
                })?;
                if str_field(&body, "claim_key") != key {
                    return fail(format!(
                        "{evidence_key} claim_key is {} — must equal this claim key {key}",
                        str_field(&body, "claim_key")
                    ));
                }
            }
            for asset_key in str_list(record, "asset_keys") {
                if get_row(conn, &asset_key)?.is_none() {
                    return fail(format!("asset_keys entry missing: {asset_key}. put_asset first."));
                }
            }
        }
        "evidence" => {
            let claim_key = str_field(record, "claim_key");
            typed_row(conn, claim_key, "claim", || {
                format!("claim_key {claim_key} does not exist. put_claim first, then put_evidence with that claim_key.")
            })?;
        }
        "verdict" => {
            let claim_key = str_field(record, "claim_key");
            typed_row(conn, claim_key, "claim", || {
                format!("claim_key {claim_key} does not exist. Cannot verdict a missing claim.")
            })?;
        }
        "finding" => {
            let claim_key = str_field(record, "claim_key");
            let verdict_key = str_field(record, "verdict_key");
            if !verdict_key.starts_with("verdicts/") && !verdict_key.starts_with("verdict/") {
                return fail("verdict_key must start with verdicts/");
            }
            typed_row(conn, claim_key, "claim", || format!("claim_key {claim_key} does not exist"))?;
            let verdict = typed_row(conn, verdict_key, "verdict", || {
                format!("verdict_key {verdict_key} does not exist. put_verdict confirmed first, then put_finding.")
            })?;
            if str_field(&verdict, "claim_key") != claim_key {
                return fail(format!(
                    "{verdict_key} claim_key is {} — must equal finding claim_key {claim_key}",
                    str_field(&verdict, "claim_key")
                ));
            }
            if str_field(&verdict, "verdict") != "confirmed" {
                return fail(format!(
                    "put_finding only after verdict=confirmed (got {}). Use returned|dismissed|discarded without a finding.",
                    str_field(&verdict, "verdict")
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
