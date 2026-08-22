#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::ext_refresh_form;

/// W12: the generic OAuth2 refresh body always carries `grant_type=refresh_token` + the
/// refresh token, and appends `client_id` ONLY when a non-empty one is supplied (a
/// blank/whitespace client_id is treated as absent — some endpoints reject an empty one).
#[test]
fn ext_refresh_form_shapes_grant_body() {
    // With a client_id.
    let with = ext_refresh_form("rt-123", Some("cid-abc"));
    assert_eq!(
        with,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", "rt-123".to_string()),
            ("client_id", "cid-abc".to_string()),
        ]
    );
    // Without a client_id → only the two required fields.
    let without = ext_refresh_form("rt-123", None);
    assert_eq!(
        without,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", "rt-123".to_string()),
        ]
    );
    // A blank client_id is treated as absent.
    assert_eq!(ext_refresh_form("rt-123", Some("   ")), without);
}
