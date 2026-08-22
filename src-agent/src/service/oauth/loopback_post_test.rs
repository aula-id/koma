#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn extract_header_finds_origin() {
    let headers = "POST /callback HTTP/1.1\r\nOrigin: https://commandcode.ai\r\nContent-Type: application/json\r\n\r\n";
    assert_eq!(extract_header(headers, "Origin"), "https://commandcode.ai");
}

#[test]
fn extract_header_missing() {
    assert_eq!(extract_header("GET / HTTP/1.1\r\n\r\n", "Origin"), "");
}
