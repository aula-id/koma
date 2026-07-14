//! W13 additional regression suite for `broker.rs` — PURE ADDITION alongside the existing
//! inline `mod tests` in that file (never touched here, and none of its helpers are reachable
//! from this sibling file — small local duplicates are built below instead).
//!
//! Gaps targeted (the inline suite is already exceptionally thorough — see the skip notes):
//! - a dropped reply-receiver (send on a closed oneshot) must never panic;
//! - param type-fuzz: a wrong JSON type where a string/array/object was expected degrades to
//!   the SAME "missing" validation error, never a panic or a silent misinterpretation;
//! - EXACT ±1 size-cap boundaries this suite doesn't already pin.
//!
//! Explicitly SKIPPED as already fully covered inline (see `broker.rs::tests`):
//! - the full verb×grant cross product (`grant_gate_truth_table` already tests every family,
//!   every unrelated-grant cross-check, and the orchestrate⇒read-only lattice edge);
//! - unknown verbs inside every family prefix → `UnknownMethod` (`grant_gate_truth_table` +
//!   `is_broker_method_covers_all_families` already probe `agents.bogus`, `sessions.bogus`,
//!   `chat.bogus`, `models.bogus`, `context.bogus`, `providers.bogus`);
//! - the `context.set` 8192/8193 exact boundary (`context_set_clear_isolation_and_size_boundary`
//!   already pins both sides).

use super::*;
use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::model::app_config::{OAuthConn, OAuthProvider};

/// Minimal single-session fixture, mirroring `broker::tests::fixture_state`.
fn fixture_state() -> AppState {
    AppState::new(Mode::Chat)
}

/// Drive [`handle_ext_call`] for one method and return the JSON reply, mirroring
/// `broker::tests::call_broker` (duplicated here — that helper lives inside the sibling
/// `mod tests` block and is not reachable from this file).
fn call_broker(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    client: &Option<Arc<OpenRouterClient>>,
    ext_id: &str,
    granted: &[Grant],
    method: &str,
    params: Value,
) -> Value {
    let (reply, mut reply_rx) = tokio::sync::oneshot::channel::<Value>();
    let req = ExtCallRequest {
        ext_id: ext_id.to_string(),
        granted: granted.to_vec(),
        method: method.to_string(),
        params,
        reply,
    };
    let mut client_local = client.clone();
    handle_ext_call(state, handle, &mut client_local, req);
    reply_rx.try_recv().expect("broker must reply inline on the oneshot in this wave")
}

/// A caller-owned OAuth conn (model-provider capable), mirroring `broker::tests::ext_conn`.
fn ext_conn(uuid: &str, ext_id: &str) -> OAuthConn {
    OAuthConn {
        uuid: uuid.to_string(),
        provider: OAuthProvider::Extension,
        access_token: "at".to_string(),
        ext_id: Some(ext_id.to_string()),
        provider_id: Some("prov".to_string()),
        chat_endpoint: Some("https://api.ext.test/v1".to_string()),
        api_type: Some("openai".to_string()),
        ..Default::default()
    }
}

// ─── reply-receiver-dropped: no panic ──────────────────────────────────────────────────────

/// Dropping the `reply` oneshot's RECEIVER before `handle_ext_call` sends on it (mirroring a
/// reader task that already timed out and stopped awaiting) must never panic — every reply
/// path uses `let _ = reply.send(..)`, discarding the `Err` a closed channel produces. Exercised
/// against a SYNC verb (`agents.list`, denied for lack of a grant) so the whole dispatch — gate
/// decision through to the final send — runs with nobody listening.
#[test]
fn reply_send_on_dropped_receiver_does_not_panic() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;

    let (reply, reply_rx) = tokio::sync::oneshot::channel::<Value>();
    drop(reply_rx); // the "reader already gave up" scenario
    let req = ExtCallRequest {
        ext_id: "test.ext".to_string(),
        granted: vec![],
        method: "agents.list".to_string(),
        params: json!({}),
        reply,
    };
    let mut client_local = client.clone();
    // Must return normally (not panic) even though nothing will ever receive this reply.
    handle_ext_call(&mut state, rt.handle(), &mut client_local, req);
}

/// The same dropped-receiver scenario for a verb that reaches its REAL handler (not just the
/// gate) — `context.clear`, granted — proving the discard-on-send-failure holds past the gate
/// too, not only on the early-return denial paths.
#[test]
fn reply_send_on_dropped_receiver_does_not_panic_past_the_gate() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;

    let (reply, reply_rx) = tokio::sync::oneshot::channel::<Value>();
    drop(reply_rx);
    let req = ExtCallRequest {
        ext_id: "test.ext".to_string(),
        granted: vec![Grant::ContextPublish],
        method: "context.clear".to_string(),
        params: json!({}),
        reply,
    };
    let mut client_local = client.clone();
    handle_ext_call(&mut state, rt.handle(), &mut client_local, req);
}

// ─── param type-fuzz: wrong JSON types degrade to the "missing" error, never a panic ──────

/// `agents.spawn { task: <number> }` — a non-string `task` is read via `.as_str()`, which
/// yields `None` for a JSON number, falling through to the SAME empty-task validation error a
/// missing/blank `task` produces (never a panic, never coerced to a string).
#[test]
fn agents_spawn_task_wrong_type_falls_to_missing_task_error() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.spawn",
        json!({ "task": 12345 }),
    );
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("non-empty 'task'")),
        "a numeric task must degrade to the empty-task error, got {out}"
    );
}

/// `agents.spawn { task, agent: <array> }` — a non-string `agent` degrades to `None`, so the
/// DEFAULT agent (`"general"`) is used rather than a panic or a stringified array.
#[test]
fn agents_spawn_agent_wrong_type_falls_to_default_agent() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsOrchestrate],
        "agents.spawn",
        json!({ "task": "do a thing", "agent": ["not", "a", "string"] }),
    );
    // Reaches the real spawn path (fixture has no client/session) — proving `agent` was
    // silently treated as absent (default agent), not a hard type error.
    let err = out.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(
        err.contains("failed to spawn agent 'general'"),
        "a non-string agent must fall back to the default agent 'general', got {out}"
    );
}

/// `agents.status { agentId: [1,2] }` (array instead of a number/numeric-string) — unparseable,
/// so it is treated as ABSENT: the "requires an 'agentId'" error, never a panic.
#[test]
fn agents_status_agent_id_wrong_type_is_treated_as_absent() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::AgentsRead],
        "agents.status",
        json!({ "agentId": [1, 2] }),
    );
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("requires an 'agentId'")),
        "an array agentId must be treated as absent, got {out}"
    );
}

/// `chat.prompt { text: <object> }` — a non-string `text` degrades to empty, hitting the same
/// "non-empty 'text'" error a missing key would.
#[test]
fn chat_prompt_text_wrong_type_falls_to_missing_text_error() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::ChatPrompt],
        "chat.prompt",
        json!({ "text": { "nested": true } }),
    );
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("non-empty 'text'")),
        "an object text must degrade to the empty-text error, got {out}"
    );
    assert!(state.rest.sessions[0].pending_ext_prompts.is_empty());
}

/// `models.invoke { prompt: <number> }` — a non-string `prompt` degrades to empty, hitting the
/// same validation error a missing/blank prompt would (never a stringified number).
#[test]
fn models_invoke_prompt_wrong_type_falls_to_missing_prompt_error() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::ModelsInvoke],
        "models.invoke",
        json!({ "prompt": 42 }),
    );
    assert!(out.get("error").is_some(), "a numeric prompt must be rejected as empty, got {out}");
}

/// `models.register { models: <object, not array> }` — the top-level type gate rejects it
/// outright with the "requires a 'models' array" error, never a panic on a non-array.
#[test]
fn models_register_models_wrong_type_is_rejected() {
    let mut config = AppConfig::default();
    let out = apply_models_register(&mut config, "my.ext", &json!({ "models": { "id": "m1" } }));
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("'models' array")),
        "a non-array 'models' must be rejected, got {out}"
    );
    assert!(config.models.is_empty());
}

/// `models.register { models: [ "not-an-object" ] }` — an array element that isn't a JSON
/// object has no `.get("id")`/`.get("name")` to read, so both fall to empty strings and the
/// per-entry "requires a non-empty 'id' and 'name'" error fires (never a panic).
#[test]
fn models_register_non_object_model_entry_is_rejected() {
    let mut config = AppConfig::default();
    config.oauth_conns.push(ext_conn("conn-a", "my.ext"));
    let out = apply_models_register(&mut config, "my.ext", &json!({ "models": ["not-an-object", 7, null] }));
    assert!(out.get("error").is_some(), "non-object array entries must be rejected, got {out}");
    assert!(config.models.is_empty(), "an atomically-rejected batch registers nothing");
}

/// `providers.register { name: <number>, key: <array> }` — both wrong-typed fields degrade to
/// empty, hitting the SAME "requires a non-empty 'name'" error a missing key would (name is
/// validated before key, so that's the first error surfaced).
#[test]
fn providers_register_wrong_types_fall_to_missing_field_errors() {
    let mut config = AppConfig::default();
    let out = apply_providers_register(
        &mut config,
        "my.ext",
        &json!({ "name": 123, "endpoint": "https://x.test", "api_type": "openai", "key": ["a"] }),
    );
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("non-empty 'name'")),
        "a numeric name must degrade to the missing-name error, got {out}"
    );
    assert!(config.providers.is_empty());
}

/// `sessions.switch { session: <number> }` — a non-string `session` degrades to the SAME
/// "requires a 'session'" error a missing key produces.
#[test]
fn sessions_switch_session_wrong_type_falls_to_missing_session_error() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::SessionsManage],
        "sessions.switch",
        json!({ "session": 7 }),
    );
    assert!(
        out.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("requires a 'session'")),
        "a numeric session must degrade to the missing-session error, got {out}"
    );
}

// ─── exact ±1 size-cap boundaries not already pinned inline ────────────────────────────────

/// `chat.prompt`: exactly 16384 bytes (16KB) is the LAST accepted size — must NOT hit the
/// "exceeds 16KB" error (the existing inline test only proves the 16385 REJECTED side).
#[test]
fn chat_prompt_16384_bytes_exactly_is_accepted() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::ChatPrompt],
        "chat.prompt",
        json!({ "text": "x".repeat(16_384) }),
    );
    assert_eq!(out, json!({ "queued": 1 }), "exactly 16384 bytes must be accepted, got {out}");
}

/// `models.invoke`: exactly 32768 bytes (32KB) must NOT hit the "exceeds 32KB" error — it
/// reaches the next validation step (role default "main" resolves; no client in the fixture ⇒
/// "no llm client"), proving the size gate itself passed at the boundary.
#[test]
fn models_invoke_32768_bytes_exactly_is_accepted_by_size_gate() {
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let mut state = fixture_state();
    let client: Option<Arc<OpenRouterClient>> = None;
    let out = call_broker(
        &mut state,
        rt.handle(),
        &client,
        "test.ext",
        &[Grant::ModelsInvoke],
        "models.invoke",
        json!({ "prompt": "x".repeat(32_768) }),
    );
    let err = out.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(
        !err.contains("32KB"),
        "exactly 32768 bytes must NOT trip the size cap, got {out}"
    );
    assert!(err.contains("no llm client"), "must reach the client check, got {out}");
}

/// `providers.register`: `name` exactly 200 chars is accepted; 201 chars is rejected
/// ("provider name too long"). `key` exactly 4096 chars is accepted; 4097 is rejected
/// ("key too long"). Each boundary probed independently against a fresh config.
#[test]
fn providers_register_name_and_key_exact_boundaries() {
    let mut config = AppConfig::default();
    let ok_name = apply_providers_register(
        &mut config,
        "my.ext",
        &json!({ "name": "n".repeat(200), "endpoint": "https://x.test", "api_type": "openai", "key": "k" }),
    );
    assert!(ok_name.get("uuid").is_some(), "a 200-char name must be accepted, got {ok_name}");

    let mut config2 = AppConfig::default();
    let bad_name = apply_providers_register(
        &mut config2,
        "my.ext",
        &json!({ "name": "n".repeat(201), "endpoint": "https://x.test", "api_type": "openai", "key": "k" }),
    );
    assert!(
        bad_name.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("name too long")),
        "a 201-char name must be rejected, got {bad_name}"
    );
    assert!(config2.providers.is_empty());

    let mut config3 = AppConfig::default();
    let ok_key = apply_providers_register(
        &mut config3,
        "my.ext",
        &json!({ "name": "N", "endpoint": "https://x.test", "api_type": "openai", "key": "k".repeat(4096) }),
    );
    assert!(ok_key.get("uuid").is_some(), "a 4096-char key must be accepted, got {ok_key}");

    let mut config4 = AppConfig::default();
    let bad_key = apply_providers_register(
        &mut config4,
        "my.ext",
        &json!({ "name": "N", "endpoint": "https://x.test", "api_type": "openai", "key": "k".repeat(4097) }),
    );
    assert!(
        bad_key.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("key too long")),
        "a 4097-char key must be rejected, got {bad_key}"
    );
    assert!(config4.providers.is_empty());
}

/// `models.register`: each model's `id`/`name` exactly 200 chars is accepted; 201 chars is
/// rejected ("model id/name too long"), and rejects the WHOLE batch atomically.
#[test]
fn models_register_field_length_exact_boundary() {
    let mut config = AppConfig::default();
    config.oauth_conns.push(ext_conn("conn-a", "my.ext"));
    let ok = apply_models_register(
        &mut config,
        "my.ext",
        &json!({ "models": [{ "id": "m".repeat(200), "name": "n".repeat(200) }] }),
    );
    assert_eq!(ok["registered"], json!(1), "exactly-200-char id/name must be accepted, got {ok}");

    let mut config2 = AppConfig::default();
    config2.oauth_conns.push(ext_conn("conn-a", "my.ext"));
    let bad = apply_models_register(
        &mut config2,
        "my.ext",
        &json!({ "models": [{ "id": "m".repeat(201), "name": "n" }] }),
    );
    assert!(
        bad.get("error").and_then(|e| e.as_str()).is_some_and(|e| e.contains("too long")),
        "a 201-char id must be rejected, got {bad}"
    );
    assert!(config2.models.is_empty(), "a rejected entry registers nothing");
}

/// `models.register`: a batch of EXACTLY [`MAX_REGISTER_MODELS`] (100) entries is accepted —
/// the existing inline test only proves the 101 REJECTED side.
#[test]
fn models_register_batch_of_100_exactly_is_accepted() {
    let mut config = AppConfig::default();
    config.oauth_conns.push(ext_conn("conn-a", "my.ext"));
    let models: Vec<Value> = (0..100).map(|i| json!({ "id": format!("m{i}"), "name": "n" })).collect();
    let out = apply_models_register(&mut config, "my.ext", &json!({ "models": models }));
    assert_eq!(out["registered"], json!(100), "exactly 100 models must be accepted, got {out}");
    assert_eq!(config.models.len(), 100);
}
