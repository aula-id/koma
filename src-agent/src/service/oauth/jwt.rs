//! Unverified JWT payload decoding. koma never validates the signature here —
//! the tokens are only read locally to pull display metadata (account id,
//! plan, email, expiry) out of a token that was already obtained from the
//! provider's own token endpoint over TLS; signature verification is the
//! provider's job at request time, not ours at display time.

use base64::Engine;

/// Decode the middle (payload) segment of `token` as JSON, without verifying
/// the signature. Returns `None` if the token isn't a 3-segment JWT or the
/// payload isn't valid base64url / JSON.
pub fn decode_payload(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Codex/ChatGPT identity fields pulled out of the id_token (and, for email,
/// falling back to the access_token).
pub struct CodexIdentity {
    pub account_id: String,
    pub plan: String,
    pub email: String,
}

/// Extract the ChatGPT account id / plan / email a Codex login belongs to.
///
/// The canonical location is the `https://api.openai.com/auth` claim
/// (`chatgpt_account_id` / `chatgpt_plan_type`) on the id_token; if that claim
/// is absent, fall back to top-level `account_id` / `plan_type` claims. Email
/// comes from the id_token's `email` claim, falling back to the
/// access_token's `email` claim (some tokens carry it there instead).
pub fn codex_identity(id_token: &str, access_token: &str) -> CodexIdentity {
    let id_payload = decode_payload(id_token);

    let auth_claim = id_payload
        .as_ref()
        .and_then(|v| v.get("https://api.openai.com/auth"));

    let account_id = auth_claim
        .and_then(|c| c.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            id_payload
                .as_ref()
                .and_then(|v| v.get("account_id"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or_default()
        .to_string();

    let plan = auth_claim
        .and_then(|c| c.get("chatgpt_plan_type"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            id_payload
                .as_ref()
                .and_then(|v| v.get("plan_type"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or_default()
        .to_string();

    let email = id_payload
        .as_ref()
        .and_then(|v| v.get("email"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            decode_payload(access_token)
                .and_then(|v| v.get("email").and_then(|e| e.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_default();

    CodexIdentity {
        account_id,
        plan,
        email,
    }
}

/// The `exp` claim (unix seconds), or `0` if the token doesn't decode or has
/// no `exp` claim.
pub fn expiry(token: &str) -> u64 {
    decode_payload(token)
        .and_then(|v| v.get("exp").and_then(|e| e.as_u64()))
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_jwt(payload_json: &serde_json::Value) -> String {
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(payload_json).unwrap());
        format!("x.{payload_b64}.x")
    }

    #[test]
    fn decode_payload_roundtrips() {
        let payload = serde_json::json!({"foo": "bar"});
        let token = make_jwt(&payload);
        assert_eq!(decode_payload(&token), Some(payload));
    }

    #[test]
    fn decode_payload_rejects_malformed_token() {
        assert!(decode_payload("not-a-jwt").is_none());
        assert!(decode_payload("a.b").is_none());
    }

    #[test]
    fn codex_identity_reads_auth_claim() {
        let id_token = make_jwt(&serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_123",
                "chatgpt_plan_type": "plus",
            },
            "email": "user@example.com",
        }));
        let access_token = make_jwt(&serde_json::json!({}));
        let identity = codex_identity(&id_token, &access_token);
        assert_eq!(identity.account_id, "acct_123");
        assert_eq!(identity.plan, "plus");
        assert_eq!(identity.email, "user@example.com");
    }

    #[test]
    fn codex_identity_falls_back_to_top_level_and_access_token_email() {
        let id_token = make_jwt(&serde_json::json!({
            "account_id": "acct_456",
            "plan_type": "pro",
        }));
        let access_token = make_jwt(&serde_json::json!({"email": "fallback@example.com"}));
        let identity = codex_identity(&id_token, &access_token);
        assert_eq!(identity.account_id, "acct_456");
        assert_eq!(identity.plan, "pro");
        assert_eq!(identity.email, "fallback@example.com");
    }

    #[test]
    fn expiry_reads_exp_claim() {
        let token = make_jwt(&serde_json::json!({"exp": 1_700_000_000u64}));
        assert_eq!(expiry(&token), 1_700_000_000);
    }

    #[test]
    fn expiry_defaults_to_zero() {
        let token = make_jwt(&serde_json::json!({}));
        assert_eq!(expiry(&token), 0);
        assert_eq!(expiry("garbage"), 0);
    }
}
