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
fn decodes_percent_encoded_values() {
    let line = "GET /auth/callback?code=a%20b%2Bc&state=hello%2Fworld HTTP/1.1";
    let params = parse_query(line);
    assert_eq!(
        params,
        vec![
            ("code".to_string(), "a b+c".to_string()),
            ("state".to_string(), "hello/world".to_string()),
        ]
    );
}
