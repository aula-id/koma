#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn validate_search_engine_rejects_empty() {
    assert!(validate_search_engine("").is_err());
}

#[test]
fn validate_search_engine_rejects_no_scheme() {
    assert!(validate_search_engine("html.duckduckgo.com/html/?q={query}").is_err());
}

#[test]
fn validate_search_engine_rejects_no_query_placeholder() {
    assert!(validate_search_engine("https://html.duckduckgo.com/html/").is_err());
}

#[test]
fn validate_search_engine_accepts_valid_ddg() {
    assert!(validate_search_engine(DEFAULT_SEARCH_ENGINE).is_ok());
}

#[test]
fn build_search_url_percent_encodes_query() {
    let url = build_search_url(DEFAULT_SEARCH_ENGINE, "hello world").unwrap();
    assert!(url.contains("hello+world"));
    assert!(!url.contains("{query}"));
}

#[test]
fn build_search_url_propagates_template_validation_error() {
    assert!(build_search_url("ftp://bad", "test").is_err());
}
