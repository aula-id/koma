#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::oauth_status;
use crate::model::app_config::{OAuthConn, OAuthProvider};

fn conn_with_expiry(expires_at: u64) -> OAuthConn {
    OAuthConn {
        provider: OAuthProvider::Codex,
        expires_at,
        ..Default::default()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[test]
fn zero_expiry_is_no_expiry() {
    assert_eq!(oauth_status(&conn_with_expiry(0)), "no expiry");
}

#[test]
fn past_expiry_is_expired() {
    let past = now_secs().saturating_sub(3600);
    assert_eq!(oauth_status(&conn_with_expiry(past)), "expired");
}

#[test]
fn several_days_out_shows_days() {
    let later = now_secs() + 3 * 86_400 + 100;
    assert_eq!(oauth_status(&conn_with_expiry(later)), "renews in 3d");
}

#[test]
fn within_a_day_is_active() {
    let soon = now_secs() + 3600;
    assert_eq!(oauth_status(&conn_with_expiry(soon)), "active");
}
