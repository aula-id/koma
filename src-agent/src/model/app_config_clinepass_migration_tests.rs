#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::AppConfig;

/// `strip_clinepass` removes a clinepass conn + orphaned model, leaves
/// a codex conn + its model untouched.
#[test]
fn strips_clinepass_and_orphan_models() {
    let json = serde_json::json!({
        "palette": "tokyo-night",
        "oauth_conns": [
            {
                "uuid": "c1",
                "provider": "clinepass",
                "access_token": "wp:dead",
                "name": "ClinePass account",
                "email": "u@cline.bot"
            },
            {
                "uuid": "c2",
                "provider": "codex",
                "access_token": "good",
                "name": "Codex account",
                "email": "u@codex.io"
            }
        ],
        "models": [
            {
                "uuid": "m1",
                "model_id": "gpt-4",
                "provider_uuid": "c1",
                "roles": ["Main"]
            },
            {
                "uuid": "m2",
                "model_id": "codex-default",
                "provider_uuid": "c2",
                "roles": ["Main"]
            }
        ]
    });

    let mut val = serde_json::to_value(&json).unwrap();
    let stripped = AppConfig::strip_clinepass(&mut val);
    assert!(stripped, "should report a strip");

    let conns = val["oauth_conns"].as_array().unwrap();
    assert_eq!(conns.len(), 1, "only codex conn survives");
    assert_eq!(conns[0]["provider"], "codex");

    let models = val["models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "only the codex model survives");
    assert_eq!(models[0]["provider_uuid"], "c2");
}

/// When there are no clinepass conns, `strip_clinepass` returns false
/// and the JSON is unchanged (no model sweep runs).
#[test]
fn no_clinepass_no_strip() {
    let json = serde_json::json!({
        "oauth_conns": [
            {
                "uuid": "c1",
                "provider": "codex",
                "access_token": "good"
            }
        ],
        "models": [
            {
                "uuid": "m1",
                "provider_uuid": "c1"
            }
        ]
    });

    let mut val = serde_json::to_value(&json).unwrap();
    let stripped = AppConfig::strip_clinepass(&mut val);
    assert!(!stripped, "no clinepass → no strip");
    assert_eq!(val["oauth_conns"].as_array().unwrap().len(), 1);
    assert_eq!(val["models"].as_array().unwrap().len(), 1);
}

/// A JSON document with no `oauth_conns` key at all is a no-op.
#[test]
fn no_oauth_conns_key() {
    let json = serde_json::json!({
        "palette": "tokyo-night",
        "models": []
    });
    let mut val = serde_json::to_value(&json).unwrap();
    let stripped = AppConfig::strip_clinepass(&mut val);
    assert!(!stripped);
}
