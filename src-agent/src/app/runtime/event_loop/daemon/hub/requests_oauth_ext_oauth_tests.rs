#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

fn def(id: &str, name: &str, method: &str) -> OAuthProviderDef {
    OAuthProviderDef {
        id: id.to_string(),
        name: name.to_string(),
        method: method.to_string(),
        chat_endpoint: None,
        api_type: None,
        refresh: None,
    }
}

// ── row surfacing (pure builder) ─────────────────────────────────────────────────

/// Granted + enabled + a declared provider → exactly one row, with the exact
/// `ext:<ext_id>:<provider_id>` id, the provider `name` as the label, and the badge kind
/// mapped from `method`.
#[test]
fn rows_for_granted_declared_provider() {
    let providers = [def("demo", "Demo Login", "device_code")];
    let rows = ext_oauth_rows_for(
        "run.koma.example.oauth-demo-daemon",
        true,
        &["oauth:contribute".to_string()],
        &providers,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "ext:run.koma.example.oauth-demo-daemon:demo");
    assert_eq!(rows[0].label, "Demo Login");
    assert_eq!(rows[0].kind, "device");
}

/// A declared provider but NO `oauth:contribute` grant → no rows (the authorization gate).
#[test]
fn rows_none_without_grant() {
    let providers = [def("demo", "Demo Login", "device_code")];
    assert!(ext_oauth_rows_for("ext.a", true, &[], &providers).is_empty());
    // An unrelated grant does not unlock it either.
    assert!(
        ext_oauth_rows_for("ext.a", true, &["agents:read".to_string()], &providers).is_empty()
    );
}

/// A disabled extension contributes no rows even when granted + declaring providers.
#[test]
fn rows_none_when_disabled() {
    let providers = [def("demo", "Demo Login", "browser")];
    assert!(ext_oauth_rows_for(
        "ext.a",
        false,
        &["oauth:contribute".to_string()],
        &providers
    )
    .is_empty());
}

/// Granted + enabled but declaring NO providers → no rows.
#[test]
fn rows_none_without_declared_providers() {
    assert!(
        ext_oauth_rows_for("ext.a", true, &["oauth:contribute".to_string()], &[]).is_empty()
    );
}

/// Multiple declared providers → one row each, ids kept distinct.
#[test]
fn rows_one_per_declared_provider() {
    let providers = [
        def("gh", "GitHub", "browser"),
        def("gl", "GitLab", "device_code"),
    ];
    let rows = ext_oauth_rows_for(
        "acme.ext",
        true,
        &["oauth:contribute".to_string()],
        &providers,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "ext:acme.ext:gh");
    assert_eq!(rows[0].kind, "pkce");
    assert_eq!(rows[1].id, "ext:acme.ext:gl");
    assert_eq!(rows[1].kind, "device");
}

#[test]
fn method_maps_to_badge_kind() {
    assert_eq!(method_to_kind("browser"), "pkce");
    assert_eq!(method_to_kind("device_code"), "device");
    assert_eq!(method_to_kind("paste"), "paste");
    // Unknown method → browser badge.
    assert_eq!(method_to_kind("carrier_pigeon"), "pkce");
}

// ── picker-id parsing (start_oauth routing) ─────────────────────────────────────

#[test]
fn parse_ext_id_valid() {
    assert_eq!(
        parse_ext_provider_id("ext:run.koma.example.oauth-demo-daemon:demo"),
        Some((
            "run.koma.example.oauth-demo-daemon".to_string(),
            "demo".to_string()
        ))
    );
}

#[test]
fn parse_ext_id_malformed_is_none() {
    // Not an ext id (a native provider).
    assert_eq!(parse_ext_provider_id("codex"), None);
    // Prefix but no separator.
    assert_eq!(parse_ext_provider_id("ext:justanid"), None);
    // Empty ext id or empty provider id.
    assert_eq!(parse_ext_provider_id("ext::demo"), None);
    assert_eq!(parse_ext_provider_id("ext:some.ext:"), None);
    // Empty / prefix-only.
    assert_eq!(parse_ext_provider_id(""), None);
    assert_eq!(parse_ext_provider_id("ext:"), None);
}

// ── oauth.begin classification ──────────────────────────────────────────────────

#[test]
fn begin_browser() {
    assert_eq!(
        parse_begin(&json!({ "url": "https://example.com/auth" })),
        BeginOutcome::Browser {
            url: "https://example.com/auth".to_string()
        }
    );
}

#[test]
fn begin_device() {
    assert_eq!(
        parse_begin(
            &json!({ "userCode": "ABCD-1234", "verificationUrl": "https://example.com/activate" })
        ),
        BeginOutcome::Device {
            user_code: "ABCD-1234".to_string(),
            verification_url: "https://example.com/activate".to_string(),
        }
    );
}

#[test]
fn begin_error_and_empty_are_failed() {
    assert!(
        matches!(parse_begin(&json!({ "error": "nope" })), BeginOutcome::Failed(e) if e == "nope")
    );
    // Neither a url nor a (complete) device code → failed, never a stuck spinner.
    assert!(matches!(parse_begin(&json!({})), BeginOutcome::Failed(_)));
    assert!(matches!(
        parse_begin(&json!({ "userCode": "X" })),
        BeginOutcome::Failed(_)
    ));
    assert!(matches!(
        parse_begin(&json!({ "url": "" })),
        BeginOutcome::Failed(_)
    ));
}

// ── oauth.poll decision ─────────────────────────────────────────────────────────

#[test]
fn poll_pending_continues() {
    assert_eq!(
        decide_poll(&json!({ "status": "pending" })),
        PollDecision::Continue
    );
    // Unknown status and empty reply are both non-terminal (keep polling until budget).
    assert_eq!(
        decide_poll(&json!({ "status": "warming_up" })),
        PollDecision::Continue
    );
    assert_eq!(decide_poll(&json!({})), PollDecision::Continue);
}

#[test]
fn poll_success_maps_token() {
    let d = decide_poll(&json!({
        "status": "success",
        "token": {
            "access_token": "at-123",
            "refresh_token": "rt-456",
            "expires_at": 1_800_000_000u64,
            "email": "me@example.com",
            "label": "My Account"
        }
    }));
    assert_eq!(
        d,
        PollDecision::Success(ExtToken {
            access_token: "at-123".to_string(),
            refresh_token: Some("rt-456".to_string()),
            expires_at: Some(1_800_000_000),
            email: Some("me@example.com".to_string()),
            label: Some("My Account".to_string()),
        })
    );
}

#[test]
fn poll_success_minimal_token() {
    // Only access_token → the rest default to None.
    let d =
        decide_poll(&json!({ "status": "success", "token": { "access_token": "at-only" } }));
    assert_eq!(
        d,
        PollDecision::Success(ExtToken {
            access_token: "at-only".to_string(),
            refresh_token: None,
            expires_at: None,
            email: None,
            label: None,
        })
    );
}

#[test]
fn poll_success_without_access_token_is_failed() {
    // A "success" with an empty/missing access_token is a protocol violation → failed.
    assert!(matches!(
        decide_poll(&json!({ "status": "success", "token": { "access_token": "" } })),
        PollDecision::Failed(_)
    ));
    assert!(matches!(
        decide_poll(&json!({ "status": "success", "token": {} })),
        PollDecision::Failed(_)
    ));
    assert!(matches!(
        decide_poll(&json!({ "status": "success" })),
        PollDecision::Failed(_)
    ));
}

#[test]
fn poll_failed_and_bare_error() {
    assert!(matches!(
        decide_poll(&json!({ "status": "failed", "error": "user denied" })),
        PollDecision::Failed(e) if e == "user denied"
    ));
    // A bare error object (no status) is terminal too — malformed replies never hang.
    assert!(matches!(
        decide_poll(&json!({ "error": "extension crashed" })),
        PollDecision::Failed(e) if e == "extension crashed"
    ));
    // A "failed" without an error message still fails with a default reason.
    assert!(matches!(
        decide_poll(&json!({ "status": "failed" })),
        PollDecision::Failed(_)
    ));
}

// ── conn construction ───────────────────────────────────────────────────────────

#[test]
fn build_conn_stamps_ext_identity() {
    let token = ExtToken {
        access_token: "at".to_string(),
        refresh_token: Some("rt".to_string()),
        expires_at: Some(42),
        email: Some("e@x.test".to_string()),
        label: Some("Nice Label".to_string()),
    };
    // An account-login-only def (no chat_endpoint/api_type) → the conn is NOT a model
    // provider (its W12 meta stays None).
    let conn = build_ext_conn(
        "run.koma.ext.demo",
        "demo",
        &def("demo", "Demo", "browser"),
        token,
    );
    assert_eq!(conn.provider, OAuthProvider::Extension);
    assert_eq!(conn.ext_id.as_deref(), Some("run.koma.ext.demo"));
    assert_eq!(conn.provider_id.as_deref(), Some("demo"));
    assert_eq!(conn.name, "Nice Label"); // label wins
    assert_eq!(conn.access_token, "at");
    assert_eq!(conn.refresh_token, "rt");
    assert_eq!(conn.expires_at, 42);
    assert_eq!(conn.email, "e@x.test");
    assert!(!conn.uuid.is_empty()); // minted host-side
    assert!(conn.chat_endpoint.is_none());
    assert!(conn.api_type.is_none());
    assert!(
        conn.ext_model_route().is_none(),
        "account-login-only conn is not a model provider"
    );
}

/// W12: a def declaring a chat endpoint + a recognised api_type + a refresh descriptor
/// stamps the conn's model-provider meta, so the ext token becomes a resolvable provider.
#[test]
fn build_conn_stamps_model_provider_meta() {
    let provider_def = OAuthProviderDef {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        method: "browser".to_string(),
        chat_endpoint: Some("https://api.demo.test/v1".to_string()),
        api_type: Some("openai".to_string()),
        refresh: Some(koma_extension::protocol::OAuthRefreshDef {
            token_url: "https://demo.test/token".to_string(),
            client_id: "cid".to_string(),
        }),
    };
    let token = ExtToken {
        access_token: "at".to_string(),
        refresh_token: Some("rt".to_string()),
        expires_at: Some(42),
        email: None,
        label: None,
    };
    let conn = build_ext_conn("e.ext", "demo", &provider_def, token);
    assert_eq!(
        conn.chat_endpoint.as_deref(),
        Some("https://api.demo.test/v1")
    );
    assert_eq!(conn.api_type.as_deref(), Some("openai"));
    assert_eq!(
        conn.refresh_token_url.as_deref(),
        Some("https://demo.test/token")
    );
    assert_eq!(conn.refresh_client_id.as_deref(), Some("cid"));
    assert!(
        conn.ext_model_route().is_some(),
        "a declared model provider resolves"
    );
}

/// W12: only `"openai"`/`"anthropic"` are accepted api_type wires; anything else (an
/// unknown/legacy value, or absent) normalizes to `None` (account-login-only).
#[test]
fn normalize_ext_api_type_accepts_only_known_wires() {
    assert_eq!(
        normalize_ext_api_type(Some("openai")).as_deref(),
        Some("openai")
    );
    assert_eq!(
        normalize_ext_api_type(Some("  anthropic  ")).as_deref(),
        Some("anthropic")
    );
    assert_eq!(normalize_ext_api_type(Some("openai_compatible")), None);
    assert_eq!(normalize_ext_api_type(Some("")), None);
    assert_eq!(normalize_ext_api_type(None), None);
}

#[test]
fn build_conn_falls_back_to_ext_provider_name() {
    let token = ExtToken {
        access_token: "at".to_string(),
        refresh_token: None,
        expires_at: None,
        email: None,
        label: None,
    };
    let conn = build_ext_conn(
        "run.koma.ext.demo",
        "demo",
        &def("demo", "Demo", "browser"),
        token,
    );
    assert_eq!(conn.name, "run.koma.ext.demo:demo"); // no label → id fallback
    assert_eq!(conn.refresh_token, "");
    assert_eq!(conn.expires_at, 0);
}
