#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn parses_code_and_state() {
    let line = "GET /auth/callback?code=abc123&state=xyz789 HTTP/1.1";
    let params = parse_query(line);
    assert_eq!(
        params,
        vec![
            ("code".to_string(), "abc123".to_string()),
            ("state".to_string(), "xyz789".to_string()),
        ]
    );
}

#[test]
fn parses_error_param() {
    let line = "GET /auth/callback?error=access_denied&state=xyz789 HTTP/1.1";
    let params = parse_query(line);
    assert_eq!(
        params,
        vec![
            ("error".to_string(), "access_denied".to_string()),
            ("state".to_string(), "xyz789".to_string()),
        ]
    );
}

#[test]
fn favicon_request_has_no_params() {
    let line = "GET /favicon.ico HTTP/1.1";
    assert!(parse_query(line).is_empty());
}

#[test]
fn callback_page_has_logo_and_return_button() {
    let ok = callback_page(true);
    assert!(ok.contains("data:image/png;base64,"));
    assert!(ok.contains("You're signed in"));
    assert!(ok.contains("Back to koma"));
    assert!(ok.contains("goBack()"));
    let bad = callback_page(false);
    assert!(bad.contains("Sign-in didn't finish"));
    assert!(bad.contains("data:image/png;base64,"));
}
