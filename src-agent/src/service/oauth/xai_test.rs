#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::is_valid_xai_endpoint;

#[test]
fn accepts_x_ai_hosts() {
    assert!(is_valid_xai_endpoint("https://auth.x.ai/oauth2/token"));
    assert!(is_valid_xai_endpoint("https://x.ai/token"));
    assert!(is_valid_xai_endpoint("https://api.x.ai/v1/models"));
    assert!(is_valid_xai_endpoint("https://api.x.ai:443/oauth2/token"));
    assert!(is_valid_xai_endpoint("https://AUTH.X.AI/oauth2/token"));
}

#[test]
fn rejects_non_https_and_foreign_hosts() {
    assert!(!is_valid_xai_endpoint("http://auth.x.ai/token")); // not https
    assert!(!is_valid_xai_endpoint("https://evil.com/token"));
    assert!(!is_valid_xai_endpoint("https://x.ai.evil.com/token"));
    assert!(!is_valid_xai_endpoint("https://notx.ai/token")); // not a .x.ai suffix
    assert!(!is_valid_xai_endpoint("https://evilx.ai/token"));
    assert!(!is_valid_xai_endpoint("https://x.ai@evil.com/token")); // userinfo → host evil.com
    assert!(!is_valid_xai_endpoint("https://user:pass@x.ai/token")); // credentials
    assert!(!is_valid_xai_endpoint("//x.ai/token")); // no scheme
    assert!(!is_valid_xai_endpoint("not-a-url"));
}

/// Regression: WHATWG treats `\` as a path separator for special (https)
/// schemes, so `https://evil.com\.x.ai/…` actually DIALS `evil.com`. The old
/// string-split extractor read that as host `evil.com\.x.ai` and wrongly
/// passed it (a token-exfiltration bypass). Every vector here dials a NON-x.ai
/// host and MUST be rejected.
#[test]
fn rejects_backslash_authority_confusables() {
    for bad in [
        "https://evil.com\\.x.ai/token",
        "https://evil.com\\.x.ai:443/token",
        "https://evil.com\\.x.ai?a=1",
        "https://evil.com\\.x.ai#frag",
        "https://evil.com\\\\.x.ai/token", // multiple backslashes
    ] {
        assert!(
            !is_valid_xai_endpoint(bad),
            "must reject (dials non-x.ai): {bad}"
        );
    }
}

/// The hard invariant, checked against the SAME parser the HTTP client dials
/// with: whenever the validator accepts a URL, the host `url`/reqwest would
/// actually connect to must be an x.ai host. This holds by construction (the
/// validator parses via `url::Url`) and guards against a future regression to
/// hand-rolled host extraction — which would accept a `\`-confusable the
/// client dials elsewhere. Probes include `\`-forms that legitimately resolve
/// to x.ai (e.g. `x.ai\.evil.com` → host x.ai, path `/.evil.com`).
#[test]
fn accept_implies_dialed_host_is_xai() {
    for probe in [
        "https://x.ai/token",
        "https://api.x.ai/v1/models",
        "https://auth.x.ai/oauth2/token",
        "https://evil.com\\.x.ai/token",
        "https://evil.com\\.x.ai:443/token",
        "https://x.ai\\.evil.com/token", // dials x.ai (path /.evil.com) — safe to accept
        "https://x.ai\\@evil.com/token", // dials x.ai (path /@evil.com) — safe to accept
        "https://x.ai@evil.com/token",   // dials evil.com — must be rejected
        "https://evil.com/token",
        "https://x.ai.evil.com/token",
        "http://x.ai/token",
        "//x.ai/token",
    ] {
        if is_valid_xai_endpoint(probe) {
            let host = url::Url::parse(probe)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
                .unwrap_or_default();
            assert!(
                host == "x.ai" || host.ends_with(".x.ai"),
                "validator accepted {probe:?} but the dialed host is {host:?}"
            );
        }
    }
}
