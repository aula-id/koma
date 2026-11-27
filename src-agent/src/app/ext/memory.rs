//! Validation helpers for the memory extension contract.

use anyhow::{bail, Context, Result};
use koma_extension::memory::{RetrieveResponse, StoreResponse};

/// The host bounds provider context before it is added to a request.
pub(crate) const MAX_RECALL_BYTES: usize = 8 * 1024;

/// Parse a response and reject provider errors and responses above the budget.
pub(crate) fn parse_retrieve(
    value: serde_json::Value,
    max_bytes: usize,
) -> Result<RetrieveResponse> {
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        bail!("memory provider returned an error: {error}");
    }
    let result: RetrieveResponse =
        serde_json::from_value(value).context("invalid memory response")?;
    let mut used = 0usize;
    for entry in &result.entries {
        if entry.id.is_empty() {
            bail!("memory entry has an empty id");
        }
        used = used
            .saturating_add(entry.id.len())
            .saturating_add(entry.content.len());
        if used > max_bytes.min(MAX_RECALL_BYTES) {
            bail!("memory response exceeds the allowed size");
        }
    }
    Ok(result)
}

/// Accept an acknowledgment only when it matches the submitted operation.
pub(crate) fn parse_store(value: serde_json::Value, key: &str) -> Result<()> {
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        bail!("memory provider returned an error: {error}");
    }
    let result: StoreResponse =
        serde_json::from_value(value).context("invalid memory acknowledgment")?;
    if key.is_empty() || result.idempotency_key != key {
        bail!("memory acknowledgment does not match the request");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retrieval_rejects_bad_or_oversized_results() {
        assert!(parse_retrieve(json!({"entries":[{"id":"x","content":"hello"}]}), 6).is_ok());
        assert!(parse_retrieve(json!({"entries":[{"id":"x","content":"hello"}]}), 5).is_err());
        assert!(parse_retrieve(json!({"entries":[{"id":"","content":"hello"}]}), 100).is_err());
        assert!(parse_retrieve(json!({"error":"failed"}), 100).is_err());
    }

    #[test]
    fn storage_rejects_missing_or_mismatched_acknowledgments() {
        assert!(parse_store(json!({"idempotency_key":"one"}), "one").is_ok());
        assert!(parse_store(json!({"idempotency_key":"two"}), "one").is_err());
        assert!(parse_store(json!({"error":"failed"}), "one").is_err());
    }
}
