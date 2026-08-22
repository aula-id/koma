#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn validate_url_safe_rejects_loopback_v4() {
    assert!(validate_url_safe("http://127.0.0.1/secret").is_err());
}

#[test]
fn validate_url_safe_rejects_localhost() {
    assert!(validate_url_safe("http://localhost/admin").is_err());
}

#[test]
fn validate_url_safe_rejects_private_10() {
    assert!(validate_url_safe("http://10.0.0.1/").is_err());
}

#[test]
fn validate_url_safe_rejects_private_172() {
    assert!(validate_url_safe("http://172.16.0.1/").is_err());
}

#[test]
fn validate_url_safe_rejects_private_192() {
    assert!(validate_url_safe("http://192.168.1.1/").is_err());
}

#[test]
fn validate_url_safe_rejects_link_local() {
    assert!(validate_url_safe("http://169.254.1.1/").is_err());
}

#[test]
fn validate_url_safe_rejects_cloud_metadata() {
    assert!(validate_url_safe("http://169.254.169.254/latest/meta-data/").is_err());
}

#[test]
fn validate_url_safe_rejects_ipv6_loopback() {
    assert!(validate_url_safe("http://[::1]/").is_err());
}

#[test]
fn validate_url_safe_rejects_ipv6_unique_local() {
    assert!(validate_url_safe("http://[fc00::1]/").is_err());
}

#[test]
fn validate_url_safe_rejects_ipv6_link_local() {
    assert!(validate_url_safe("http://[fe80::1]/").is_err());
}

#[test]
fn validate_url_safe_rejects_non_http() {
    assert!(validate_url_safe("ftp://example.com/").is_err());
    assert!(validate_url_safe("file:///etc/passwd").is_err());
}

#[test]
fn validate_url_safe_allows_public() {
    assert!(validate_url_safe("https://example.com/").is_ok());
    assert!(validate_url_safe("https://docs.rust-lang.org/").is_ok());
    assert!(validate_url_safe("http://8.8.8.8/").is_ok());
}

#[test]
fn generate_token_is_correct_length() {
    let token = generate_token().unwrap();
    assert_eq!(token.len(), TOKEN_BYTES * 2);
    // All hex chars.
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
}
