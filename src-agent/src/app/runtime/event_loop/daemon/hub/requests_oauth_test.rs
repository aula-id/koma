#![allow(clippy::unwrap_used, clippy::expect_used)]
//! W13 additional regression suite for `requests_oauth.rs` — PURE ADDITION alongside the
//! existing inline `mod ext_oauth_tests` in that file (never touched here).
//!
//! Explicitly SKIPPED as already fully covered inline (see
//! `requests_oauth.rs::ext_oauth_tests`):
//! - `ext_oauth_rows_for`'s full matrix (grant missing / disabled / undeclared providers / id
//!   format / kind mapping for every `method` value, including the unknown-method fallback);
//! - `decide_poll`'s full matrix (pending / unknown-status / minimal + full success token /
//!   failed-with-reason / bare-error / success-without-access-token → Failed);
//! - `parse_ext_provider_id`'s core malformed cases (empty, prefix-only, no separator, empty
//!   ext id, empty provider id).
//!
//! Gaps targeted here:
//! - `parse_ext_provider_id("ext:a:b:c")` — an id with EXTRA colons past the first separator
//!   is not malformed by the code's own contract (`split_once` takes only the FIRST `:`), so
//!   this pins the actual (perhaps surprising) accepted semantics rather than assuming `None`;
//! - `parse_begin` when BOTH a `url` AND a complete device code are present in the same reply —
//!   the existing tests only ever supply one or the other.

use super::*;
use serde_json::json;

/// `parse_ext_provider_id`'s intended semantics, verified straight from the implementation
/// (`id.strip_prefix("ext:")` then `rest.split_once(':')` — the FIRST remaining colon only):
/// an id with extra colons past that first separator is NOT malformed — the whole remainder
/// after the first colon becomes the `provider_id` verbatim, colons and all.
#[test]
fn parse_ext_id_extra_colons_retained_in_provider_id_verbatim() {
    assert_eq!(
        parse_ext_provider_id("ext:a:b:c"),
        Some(("a".to_string(), "b:c".to_string())),
        "only the FIRST colon after the prefix splits; everything after it is the provider id"
    );
    // A more realistic shape: a provider id that itself happens to look like a UUID-ish
    // string with colons is still carried through unmangled.
    assert_eq!(
        parse_ext_provider_id("ext:run.koma.example.demo:oauth:v2"),
        Some(("run.koma.example.demo".to_string(), "oauth:v2".to_string()))
    );
}

/// `parse_begin` when a reply carries BOTH a `url` AND a complete device code
/// (`userCode`+`verificationUrl`): the implementation checks the device-code pair FIRST, so
/// `Device` wins — pinning that precedence (an extension author relying on the opposite
/// order would be surprised, so this is worth locking down explicitly).
#[test]
fn begin_device_wins_when_both_url_and_device_code_present() {
    let reply = json!({
        "url": "https://example.com/auth",
        "userCode": "ABCD-1234",
        "verificationUrl": "https://example.com/activate",
    });
    assert_eq!(
        parse_begin(&reply),
        BeginOutcome::Device {
            user_code: "ABCD-1234".to_string(),
            verification_url: "https://example.com/activate".to_string(),
        },
        "a reply carrying both a url and a complete device code must resolve to Device"
    );
}

/// A malformed device code (only `userCode`, no `verificationUrl`) alongside a valid `url`
/// falls through to `Browser` — the device-code pair must be COMPLETE to win, a partial pair
/// never wins by itself nor blocks the url fallback.
#[test]
fn begin_falls_back_to_url_when_device_code_is_incomplete() {
    let reply = json!({
        "url": "https://example.com/auth",
        "userCode": "ABCD-1234",
        // verificationUrl deliberately absent.
    });
    assert_eq!(
        parse_begin(&reply),
        BeginOutcome::Browser { url: "https://example.com/auth".to_string() },
        "an incomplete device code must not shadow a valid url"
    );
}
