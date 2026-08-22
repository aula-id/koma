#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// RFC 7636 Appendix B test vector.
#[test]
fn challenge_matches_rfc7636_vector() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn generate_produces_well_formed_triple() {
    let p = generate();
    assert_eq!(p.verifier.len(), 64);
    assert!(p.verifier.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!p.challenge.is_empty());
    assert!(!p.state.is_empty());
}
