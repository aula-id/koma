//! Opt-in LLM request dump for debugging OAuth/quality regressions.
//!
//! Enable with `KOMA_DEBUG_LLM=1` (also accepts `true` / `yes`). When on, each
//! outbound LLM POST appends a redacted block to `~/.koma/error.log` via
//! [`crate::model::store::append_global_error_log`]. Off by default — zero I/O.

use serde::Serialize;

/// Body dump hard cap (512 KiB). Larger bodies are truncated with a marker.
const MAX_BODY_CHARS: usize = 512 * 1024;

/// True when `KOMA_DEBUG_LLM` is set to a truthy value (`1` / `true` / `yes`).
pub(super) fn llm_debug_enabled() -> bool {
    match std::env::var("KOMA_DEBUG_LLM") {
        Ok(v) => {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

/// Redact a single header value. `Authorization` / cookie / api-key names lose
/// their values; everything else passes through.
pub(super) fn redact_header_value(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower == "authorization"
        || lower.contains("cookie")
        || lower.contains("api-key")
        || lower.contains("apikey")
        || lower == "x-api-key"
    {
        "<redacted>".to_string()
    } else {
        value.to_string()
    }
}

/// Append one redacted request dump when the flag is on. Best-effort; never
/// panics. `headers` are `(name, value)` pairs as they would leave the client
/// (Authorization may still carry a real bearer — it is stripped here).
pub(super) fn dump_outbound(url: &str, headers: &[(&str, &str)], body: &impl Serialize) {
    if !llm_debug_enabled() {
        return;
    }
    let mut block = String::new();
    block.push_str("url: ");
    block.push_str(url);
    block.push('\n');
    block.push_str("headers:\n");
    for (name, value) in headers {
        block.push_str("  ");
        block.push_str(name);
        block.push_str(": ");
        block.push_str(&redact_header_value(name, value));
        block.push('\n');
    }
    block.push_str("body:\n");
    match serde_json::to_string_pretty(body) {
        Ok(json) => {
            if json.chars().count() > MAX_BODY_CHARS {
                let truncated: String = json.chars().take(MAX_BODY_CHARS).collect();
                block.push_str(&truncated);
                block.push_str("\n… [truncated: body exceeded 512 KiB]\n");
            } else {
                block.push_str(&json);
                block.push('\n');
            }
        }
        Err(e) => {
            block.push_str(&format!("<serialize error: {e}>\n"));
        }
    }
    crate::model::store::append_global_error_log("LLM debug request", &block);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn redaction_strips_bearer_and_cookies() {
        assert_eq!(
            redact_header_value("Authorization", "Bearer secret-token"),
            "<redacted>"
        );
        assert_eq!(
            redact_header_value("authorization", "Bearer x"),
            "<redacted>"
        );
        assert_eq!(redact_header_value("Cookie", "sid=abc"), "<redacted>");
        assert_eq!(redact_header_value("X-Api-Key", "k"), "<redacted>");
        assert_eq!(
            redact_header_value("originator", "codex_cli_rs"),
            "codex_cli_rs"
        );
        assert_eq!(
            redact_header_value("chatgpt-account-id", "acct_1"),
            "acct_1"
        );
    }

    #[test]
    fn disabled_by_default_no_panic() {
        // Ensure calling dump with the flag unset is a no-op (does not panic).
        // We cannot reliably unset env in parallel tests, so only assert the
        // redaction path + that dump_outbound is safe to call.
        let body = serde_json::json!({"model": "test"});
        // Always safe: when disabled returns immediately; when enabled appends.
        dump_outbound(
            "https://example.test/v1/chat",
            &[("Authorization", "Bearer secret"), ("X-Title", "koma")],
            &body,
        );
    }
}
