//! oauth-demo-daemon: the W11 test vehicle for DELEGATED extension OAuth.
//!
//! It contributes one OAuth login provider (`demo`) and requires the
//! `oauth:contribute` grant. koma surfaces it as a picker row and delegates the
//! whole login to this daemon over the `oauth.*` invoke contract (see the SDK's
//! `Extension` trait docs). This sample runs a FAKE device-code flow with no real
//! network: `oauth.begin` hands back a user code + verification URL, the first two
//! `oauth.poll`s report `pending`, and the third reports `success` with a fake
//! token. Run `cargo run -p oauth-demo-daemon` to see the demo handshake.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest};
use serde_json::Value;

/// Number of `oauth.poll`s (per provider) before the fake flow "completes".
const POLLS_UNTIL_SUCCESS: u32 = 3;

#[derive(Default)]
struct OAuthDemo {
    /// How many times `oauth.poll` has been called since the last `oauth.begin`.
    /// A real extension would track per-provider pending state (a device code +
    /// its expiry); this fake flow only needs a counter.
    poll_count: u32,
}

impl OAuthDemo {
    /// The provider id from an `oauth.*` invoke's `{ "providerId": ... }`.
    fn provider_id(params: &Value) -> &str {
        params.get("providerId").and_then(|v| v.as_str()).unwrap_or("")
    }
}

impl Extension for OAuthDemo {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }

    fn on_invoke(&mut self, method: &str, params: Value) -> Value {
        match method {
            // Start a login: reset the poll counter and hand koma a device code +
            // verification URL. koma renders these in its `waiting_code` phase.
            "oauth.begin" => {
                let provider = Self::provider_id(&params);
                if provider != "demo" {
                    return serde_json::json!({ "error": format!("unknown provider: {provider}") });
                }
                self.poll_count = 0;
                serde_json::json!({
                    "userCode": "ABCD-1234",
                    "verificationUrl": "https://example.com/activate"
                })
            }
            // koma polls this ~every 3s. Report `pending` until the fake flow
            // "completes", then `success` with a fake token (only `access_token`
            // is required; the rest are optional identity/lifecycle hints).
            "oauth.poll" => {
                let provider = Self::provider_id(&params);
                if provider != "demo" {
                    return serde_json::json!({ "status": "failed", "error": format!("unknown provider: {provider}") });
                }
                self.poll_count += 1;
                if self.poll_count < POLLS_UNTIL_SUCCESS {
                    serde_json::json!({ "status": "pending" })
                } else {
                    serde_json::json!({
                        "status": "success",
                        "token": {
                            "access_token": "demo-access-token-abc123",
                            "refresh_token": "demo-refresh-token-xyz789",
                            "email": "demo@example.com",
                            "label": "Demo account"
                        }
                    })
                }
            }
            // Best-effort teardown — koma ignores the result. Reset our state.
            "oauth.cancel" => {
                self.poll_count = 0;
                serde_json::json!({ "ok": true })
            }
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }
}

fn main() {
    run_daemon(
        OAuthDemo::default(),
        DaemonDemo {
            // In demo mode, show the begin step (a real login is driven by koma over
            // the socket; the poll loop needs a live host, so demo mode just shows
            // the opening handshake).
            invoke: Some((
                "oauth.begin".to_string(),
                serde_json::json!({ "providerId": "demo" }),
            )),
            driver: None,
        },
    );
}
