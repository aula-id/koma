//! PKCE (RFC 7636) code-verifier/challenge generation for the Codex
//! authorization-code flow.

use base64::Engine;
use sha2::{Digest, Sha256};

/// One PKCE pair plus the CSRF `state` value for a single login attempt.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

/// Generate a fresh PKCE verifier/challenge/state triple.
///
/// The verifier is 64 hex characters (two concatenated UUIDv4 `simple` forms)
/// — hex is a valid RFC 7636 `code_verifier` charset (`[A-Za-z0-9\-._~]`) and
/// 64 chars sits comfortably inside the 43-128 length bound. The challenge is
/// `BASE64URL-ENCODE(SHA256(verifier))` with no padding, per S256.
pub fn generate() -> Pkce {
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    let state = uuid::Uuid::new_v4().simple().to_string();
    Pkce {
        verifier,
        challenge,
        state,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
}
