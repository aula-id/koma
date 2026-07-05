//! `OpenRouterClient` struct definition plus construction and accessor methods.

/// A keyless, per-session HTTP holder. Owns ONLY the shared `reqwest::Client`
/// (header-less; internally Arc'd, safe to share across all roles) and the
/// per-session `plan_word`. Connection, model, provider-route, and effort are
/// resolved per-role at each call site and threaded in as parameters — nothing
/// credential- or model-specific is baked onto the client, so it never needs
/// rebuilding when those change (only at session boundaries, for a fresh
/// `plan_word`).
pub struct OpenRouterClient {
    pub(super) http: reqwest::Client,
    /// Whimsical plan lead-in word, chosen ONCE per client (= per session) in
    /// the constructor. [`Self::stream_complete`] injects this SAME word into the
    /// system message every request instead of rolling a fresh one each time —
    /// keeping the system prefix byte-stable across the session so OpenRouter
    /// prompt caching can hit. (A per-request random word busted the cache.)
    plan_word: String,
    /// Stable per-session id for the Codex Responses transport, minted once per
    /// client (same lifetime as `plan_word`). Sent as BOTH the `session_id`
    /// header and the request `prompt_cache_key` so the backend keys its prompt
    /// cache to this session across the turn's many `/responses` calls.
    codex_session_id: String,
}

impl Default for OpenRouterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenRouterClient {
    /// Build a fresh, keyless client. Takes no creds/model/provider/effort — those
    /// are resolved per-role and passed into each request method. Re-rolls the
    /// session's `plan_word`, so call this once per session activation (build /
    /// `/new` / picker-select / creds-confirm / cancel paths) and NOT on a mid-
    /// session cred/effort change (which would needlessly bust the cache-stable
    /// plan word).
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            // Pick the plan lead-in ONCE here so every request in this session
            // injects the same word → the cached system prefix stays byte-stable.
            plan_word: crate::resources::wanderer_word(),
            // Mint the Codex session id ONCE per client so every `/responses`
            // call in this session shares one prompt_cache_key + session_id.
            codex_session_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// The whimsical plan lead-in word chosen once per client (= per session) in
    /// the constructor. Exposed so the runtime can inject the SAME steer into the
    /// system message every request (inside the cached head), keeping the cached
    /// prefix byte-stable so prompt caching can hit.
    pub fn plan_word(&self) -> &str {
        &self.plan_word
    }

    /// The stable per-session Codex Responses id (see field docs). Used by the
    /// `codex` transport as the `session_id` header + `prompt_cache_key`.
    pub(super) fn codex_session_id(&self) -> &str {
        &self.codex_session_id
    }
}
