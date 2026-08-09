//! oauth-demo-daemon: the reference sample for DELEGATED extension OAuth
//! (W11) and the model-provider-gateway wiring built on top of it (W12 /
//! W12b). koma never sees your provider's client secret or runs any of the
//! network flow itself — it only surfaces a picker row, relays progress
//! phases, and stores whatever token you eventually hand back.
//!
//! # The begin/poll/cancel contract
//!
//! This sample contributes one OAuth login provider (`demo`) and requires
//! the `oauth:contribute` grant. That combination gets you three `on_invoke`
//! methods, each carrying `{ "providerId": "demo" }` (your provider's `id`
//! from `manifest.json`, so a daemon backing more than one provider knows
//! which flow a call is for):
//!
//! - `oauth.begin` — start a login. We reply with a device code +
//!   verification URL (`{"userCode": ..., "verificationUrl": ...}`), which
//!   koma renders as its `waiting_code` phase. The other shape a real
//!   extension can reply with is `{"url": "https://..."}` for a browser
//!   flow (koma's `waiting_url` phase — it does NOT auto-open the URL) or
//!   `{"error": "..."}` for an immediate, terminal `failed`.
//! - `oauth.poll` — koma calls this roughly every 3 seconds after `begin`.
//!   We reply `{"status": "pending"}` for the first `POLLS_UNTIL_SUCCESS -
//!   1` calls, then `{"status": "success", "token": {...}}`. A real
//!   extension backed by an actual network flow would do the real waiting
//!   on its own thread and have `poll` report whatever that thread's state
//!   is — replying promptly here matters: koma bounds EACH begin/poll
//!   invoke at 25 seconds, and the WHOLE begin-to-success loop at 5 minutes
//!   before giving up with `failed: "extension OAuth timed out"`.
//! - `oauth.cancel` — best-effort teardown when the user backs out or a new
//!   flow supersedes this one. We reply `{"ok": true}` but koma ignores
//!   whatever we send back; only that we got the call and can drop our
//!   pending state matters.
//!
//! Your extension MUST be `kind: "daemon"` for delegated OAuth — the
//! begin/poll handshake needs state (a pending device code, an in-flight
//! poll) held across invokes, and a `oneshot` is respawned fresh per invoke
//! with nothing remembered.
//!
//! # W12/W12b: from a login to a resolvable model provider
//!
//! A provider that ONLY logs a user in (like this sample's `demo` provider)
//! is account-login-only: koma stores the token as a connection, and that's
//! the end of the story in v1. To make that connection a resolvable MODEL
//! PROVIDER — so an extension-contributed sub-agent's `model:` slug (or a
//! user's own model picker) can actually route requests through it — your
//! manifest's `OAuthProviderDef` needs two more fields:
//!
//! ```jsonc
//! {
//!   "id": "acme",
//!   "name": "Acme",
//!   "method": "browser",
//!   // W12 — makes this provider a resolvable model-provider gateway:
//!   "chat_endpoint": "https://api.acme.test/v1",
//!   "api_type": "openai",              // must be "openai" or "anthropic"
//!   // W12, ignored in v1 (the extension owns the whole token lifecycle;
//!   // koma never refreshes on your behalf yet):
//!   "refresh": { "token_url": "https://acme.test/token", "client_id": "..." }
//! }
//! ```
//!
//! With that declared and the `models:contribute` grant also requested (an
//! extension that registers models almost always needs BOTH
//! `oauth:contribute` and `models:contribute`), once a user connects and
//! your daemon has a live account, drive these two ext->koma calls from
//! your DRIVER THREAD — never from `on_invoke`/`on_event`, per the SDK's
//! DEADLOCK RULE — right after a successful `oauth.poll`:
//!
//! ```ignore
//! // Register the models this connected account can serve. Re-registering
//! // the same id later UPDATES its display name in place and keeps its
//! // stable uuid, so anything already bound to it (e.g. a sub-agent) keeps
//! // resolving. At most 100 models per call; `default: true` on at most
//! // one entry.
//! let reply = koma.call("models.register", serde_json::json!({
//!     "models": [
//!         { "id": "acme-large", "name": "Acme Large", "default": true },
//!         { "id": "acme-fast",  "name": "Acme Fast" }
//!     ]
//! }));
//! // -> { "registered": 2, "uuids": ["...", "..."], "defaultUuid": "..." }
//! //
//! // The "default": true entry is a HINT, not a command: koma only
//! // "vacuum-fills" it onto the user's Main role if Main is currently
//! // unset or still pointing at the keyless koma-free placeholder in BOTH
//! // the global catalogue and the active session's overrides. First
//! // vacuum-fill wins — once a real model holds Main anywhere, a later
//! // extension's default only ever surfaces as a `recommendedBy` picker
//! // hint, it never displaces what's already there.
//!
//! // Separately, a KEY-BACKED gateway (a provider you serve with your own
//! // static API key rather than riding the user's OAuth token) registers
//! // through providers.register instead — also gated by
//! // `models:contribute`, also driver-thread-only:
//! let reply = koma.call("providers.register", serde_json::json!({
//!     "name": "acme-gateway",
//!     "endpoint": "https://api.acme.test/v1",
//!     "api_type": "openai",
//!     "key": "sk-..."
//! }));
//! // -> { "uuid": "..." } — re-registering the same `name` ROTATES the key
//! // in place and keeps the same uuid, so models already bound to this
//! // provider keep resolving through a rotated key with no re-registration
//! // needed. Uninstalling this extension purges everything it registered:
//! // providers, the models served by them, its oauth conns, and its
//! // preferred-model record — see docs/EXTENSIONS.md's lifecycle section.
//! ```
//!
//! This sample's own fake flow stays account-login-only (no
//! `chat_endpoint`/`api_type` in `manifest.json`, no `models:contribute`
//! grant) so it keeps working with zero setup — the calls above are shown
//! as comments, not exercised, precisely so you can see the shape without
//! this sample needing a real backing model provider to demo against. Run
//! `cargo run -p oauth-demo-daemon` to see the demo handshake and the
//! `oauth.begin` step.

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
        params
            .get("providerId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
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
