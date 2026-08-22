//! Token cache + single-flight refresh: the send-time hook every OAuth-backed
//! request goes through to get a (possibly just-refreshed) bearer token.
//!
//! Codex and xAI tokens expire and need refreshing (each against its own token
//! endpoint — Codex's fixed URL, xAI's discovered-per-call one); Kilo Code
//! tokens in this flow carry no expiry (`expires_at == 0`) and no refresh token,
//! so the staleness check is a no-op for them and [`fresh_key`] just returns the
//! cached token. Staleness windows and the refresh call both dispatch per
//! provider (see [`refresh_window`] / [`fresh_key`]).
//!
//! Refreshing is single-flighted per uuid (a `tokio::sync::Mutex` stashed in
//! `FLIGHTS`) so concurrent requests against the same connection don't race
//! to refresh the same refresh_token — the second caller re-checks staleness
//! after acquiring the lock and finds the first caller already refreshed it.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Mutex, RwLock};

use super::registry::{
    CLAUDE_MAX_REFRESH_AGE_SECS, CLAUDE_REFRESH_LEAD_SECS, CODEX_MAX_REFRESH_AGE_SECS,
    CODEX_REFRESH_LEAD_SECS, EXT_REFRESH_LEAD_SECS, KOMA_MAX_REFRESH_AGE_SECS,
    KOMA_REFRESH_LEAD_SECS, XAI_MAX_REFRESH_AGE_SECS, XAI_REFRESH_LEAD_SECS,
};
use super::{claude, codex, komarun, xai};
use crate::model::app_config::{AppConfig, OAuthConn, OAuthProvider};

#[derive(Clone)]
struct TokenSnap {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    last_refresh: u64,
    provider: OAuthProvider,
    /// Codex ChatGPT account id, or Kilo organization id.
    account: String,
    /// Set once a refresh attempt comes back with an unrecoverable error
    /// (invalid_grant / refresh_token_reused): stop retrying, keep serving
    /// the last-known token until the user re-logs in.
    unrecoverable: bool,
    /// W12: for an EXTENSION-backed conn, the manifest-declared generic OAuth2 token
    /// endpoint koma POSTs a `refresh_token` grant to (and the optional `client_id`). `None`
    /// for every native conn (each dispatches its own provider-specific refresh) and for an
    /// ext conn whose manifest declared no refresh descriptor (koma then never refreshes it).
    refresh_token_url: Option<String>,
    refresh_client_id: Option<String>,
}

impl TokenSnap {
    fn from_conn(conn: &OAuthConn) -> Self {
        let account = match conn.provider {
            OAuthProvider::Codex => conn.account_id.clone(),
            OAuthProvider::Kilocode => conn.org_id.clone(),
            // xAI has no org/account identity — the send-time account string stays
            // empty (so the Kilo org header never fires on an xAI request).
            OAuthProvider::Xai => String::new(),
            // Anthropic doesn't use a chatgpt-account-id-style header; keep it empty
            // like xAI.
            OAuthProvider::ClaudeAI => String::new(),
            // koma.run account login has no org/account header concept either.
            OAuthProvider::KomaRun => String::new(),
            // Command Code: login-only; no org/account header.
            OAuthProvider::CommandCode => String::new(),
            // W11: an ext-backed conn is not a model provider in v1, so it has no
            // send-time account/org header. (It never reaches send-time either — no
            // ModelEntry resolves to it — but stay exhaustive + inert.)
            OAuthProvider::Extension => String::new(),
        };
        TokenSnap {
            access_token: conn.access_token.clone(),
            refresh_token: conn.refresh_token.clone(),
            expires_at: conn.expires_at,
            last_refresh: conn.last_refresh,
            provider: conn.provider,
            account,
            unrecoverable: false,
            // Only ext-backed conns carry these (native conns leave them None).
            refresh_token_url: conn.refresh_token_url.clone(),
            refresh_client_id: conn.refresh_client_id.clone(),
        }
    }
}

fn cache() -> &'static RwLock<HashMap<String, TokenSnap>> {
    static CACHE: OnceLock<RwLock<HashMap<String, TokenSnap>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn flights() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    FLIGHTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Seed or replace the cache entry for `conn`. Called right after a fresh
/// login and once for every entry when `AppConfig` loads at startup.
pub async fn seed(conn: &OAuthConn) {
    cache()
        .write()
        .await
        .insert(conn.uuid.clone(), TokenSnap::from_conn(conn));
    // Command Code: also seed the remembered chat-transport preference so the
    // next resolve/stream path skips a re-probe after restart.
    crate::service::oauth::commandcode::seed_chat_pref(conn);
}

/// Drop the cache entry for `uuid`. Called when the `/settings` OAuth
/// submenu deletes a connection.
pub async fn evict(uuid: &str) {
    cache().write().await.remove(uuid);
    flights().lock().await.remove(uuid);
}

/// Get the per-uuid single-flight lock, creating it on first use.
async fn flight_for(uuid: &str) -> Arc<Mutex<()>> {
    let mut flights = flights().lock().await;
    flights
        .entry(uuid.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// The `(refresh_lead, max_refresh_age)` staleness window for a provider whose
/// tokens expire + refresh, or `None` for one whose tokens never expire (Kilo
/// Code — no refresh token, `expires_at == 0`). A `max_refresh_age` of `0`
/// DISABLES the "too old since last refresh" cap (xAI's long-lived
/// `offline_access` refresh token).
fn refresh_window(provider: OAuthProvider) -> Option<(u64, u64)> {
    match provider {
        OAuthProvider::Codex => Some((CODEX_REFRESH_LEAD_SECS, CODEX_MAX_REFRESH_AGE_SECS)),
        OAuthProvider::Xai => Some((XAI_REFRESH_LEAD_SECS, XAI_MAX_REFRESH_AGE_SECS)),
        OAuthProvider::ClaudeAI => Some((CLAUDE_REFRESH_LEAD_SECS, CLAUDE_MAX_REFRESH_AGE_SECS)),
        OAuthProvider::KomaRun => Some((KOMA_REFRESH_LEAD_SECS, KOMA_MAX_REFRESH_AGE_SECS)),
        // Command Code keys never expire.
        OAuthProvider::CommandCode => None,
        OAuthProvider::Kilocode => None,
        // W12: an ext-backed token MAY be refreshable (when its manifest declared a refresh
        // descriptor). Use a generic short lead + no age cap; a stale token only actually
        // triggers a dispatch if the conn also carries a `refresh_token_url` (gated in
        // `fresh_key`'s Extension arm) — otherwise that arm serves the cached token verbatim,
        // exactly the W11 lifecycle-owned-by-extension behavior. A token with no `expires_at`
        // (no lifecycle hint) never goes stale regardless (`is_stale`'s `near_expiry` gate).
        OAuthProvider::Extension => Some((EXT_REFRESH_LEAD_SECS, 0)),
    }
}

fn is_stale(snap: &TokenSnap) -> bool {
    if snap.unrecoverable {
        return false;
    }
    // A provider with no refresh window (Kilo Code) never goes stale.
    let Some((lead, max_age)) = refresh_window(snap.provider) else {
        return false;
    };
    let now = now_secs();
    let near_expiry = snap.expires_at != 0 && now + lead > snap.expires_at;
    let too_old_since_refresh =
        max_age != 0 && snap.last_refresh != 0 && now.saturating_sub(snap.last_refresh) > max_age;
    near_expiry || too_old_since_refresh
}

/// Persist a refreshed token set back into `config.json`'s matching
/// `oauth_conns` entry.
fn persist_refresh(uuid: &str, tokens: &codex::TokenResponse, refreshed_at: u64) {
    let mut config = AppConfig::load();
    if let Some(idx) = config.oauth_index_by_uuid(uuid) {
        let conn = &mut config.oauth_conns[idx];
        conn.access_token = tokens.access_token.clone();
        if !tokens.refresh_token.is_empty() {
            conn.refresh_token = tokens.refresh_token.clone();
        }
        if !tokens.id_token.is_empty() {
            conn.id_token = tokens.id_token.clone();
        }
        conn.expires_at = tokens
            .expires_in
            .map(|secs| refreshed_at + secs)
            .unwrap_or(conn.expires_at);
        conn.last_refresh = refreshed_at;
        if let Err(e) = config.save() {
            crate::model::store::append_global_error_log(
                "oauth",
                &format!("failed to persist refreshed token for {uuid}: {e}"),
            );
        }
    }
}

/// W12: build the form body for a generic OAuth2 `refresh_token` grant against an
/// extension-declared token endpoint. PURE (no I/O) so the request shape is unit-testable.
/// `client_id` is included ONLY when the manifest declared a non-empty one (some token
/// endpoints require it; others identify the client by the refresh token alone).
fn ext_refresh_form(refresh_token: &str, client_id: Option<&str>) -> Vec<(&'static str, String)> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];
    if let Some(cid) = client_id.map(str::trim).filter(|c| !c.is_empty()) {
        form.push(("client_id", cid.to_string()));
    }
    form
}

/// W12: refresh an EXTENSION-backed token via a generic OAuth2 `refresh_token` grant,
/// form-encoded and POSTed to the manifest-declared `token_url`. Returns the shared
/// [`codex::TokenResponse`] shape (`{ access_token, refresh_token?, id_token?, expires_in? }`)
/// so the caller's persist/update path stays provider-agnostic. Any transport / non-2xx /
/// parse failure is an `Err(String)` (never a panic, never a logged token); `fresh_key`
/// degrades to serving the cached token on `Err`, exactly like a native refresh failure.
async fn ext_refresh(
    client: &reqwest::Client,
    token_url: &str,
    refresh_token: &str,
    client_id: Option<&str>,
) -> Result<codex::TokenResponse, String> {
    let form = ext_refresh_form(refresh_token, client_id);
    let resp = client
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("ext token refresh request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "ext token refresh returned HTTP {}",
            resp.status().as_u16()
        ));
    }
    resp.json::<codex::TokenResponse>()
        .await
        .map_err(|e| format!("ext token refresh parse failed: {e}"))
}

/// The send-time hook: resolve `oauth_uuid` to a bearer token (refreshing it
/// first if it's Codex and near/over its staleness window), plus the
/// provider-specific `account` string (Codex account id / Kilo org id).
///
/// An empty `oauth_uuid` means "this connection isn't OAuth-backed" — it
/// passes `fallback_key` straight through (the caller's existing `api_key`
/// path) with no account id.
pub async fn fresh_key(oauth_uuid: &str, fallback_key: &str) -> (String, String) {
    if oauth_uuid.is_empty() {
        return (fallback_key.to_string(), String::new());
    }

    // Lazy-seed the cache from disk on a miss (e.g. first send after startup,
    // before anything explicitly called `seed`).
    if !cache().read().await.contains_key(oauth_uuid) {
        let config = AppConfig::load();
        if let Some(idx) = config.oauth_index_by_uuid(oauth_uuid) {
            seed(&config.oauth_conns[idx]).await;
        }
    }

    let snap = match cache().read().await.get(oauth_uuid).cloned() {
        Some(s) => s,
        // Unknown uuid (deleted / never persisted) — nothing to serve.
        None => return (String::new(), String::new()),
    };

    if !is_stale(&snap) {
        return (snap.access_token, snap.account);
    }

    // Single-flight the refresh per uuid: only one task actually calls the
    // token endpoint; the rest wait for it and then re-read the cache.
    let flight = flight_for(oauth_uuid).await;
    let _guard = flight.lock().await;

    // Re-check after acquiring the lock — another in-flight refresh may have
    // already handled it while we were waiting.
    let snap = match cache().read().await.get(oauth_uuid).cloned() {
        Some(s) => s,
        None => return (String::new(), String::new()),
    };
    if !is_stale(&snap) {
        return (snap.access_token, snap.account);
    }

    // Disk re-seed: the OAuth daemon may have refreshed this token while we were
    // waiting. Reload from config.json and check if it's now fresher.
    {
        let disk_config = AppConfig::load();
        if let Some(disk_idx) = disk_config.oauth_index_by_uuid(oauth_uuid) {
            let disk_conn = &disk_config.oauth_conns[disk_idx];
            if disk_conn.last_refresh > snap.last_refresh
                || (disk_conn.expires_at != 0 && disk_conn.expires_at > snap.expires_at)
            {
                seed(disk_conn).await;
                if let Some(s) = cache().read().await.get(oauth_uuid).cloned() {
                    if !is_stale(&s) {
                        return (s.access_token, s.account);
                    }
                }
            }
        }
    }

    // Dispatch the refresh call per provider (both return the shared
    // `codex::TokenResponse` shape, so the persist/update path below is
    // provider-agnostic). Kilo never reaches here — `is_stale` returns false for
    // it — so its arm just serves the cached token defensively.
    let refreshed = match snap.provider {
        OAuthProvider::Xai => xai::refresh(http_client(), &snap.refresh_token).await,
        OAuthProvider::Codex => codex::refresh(http_client(), &snap.refresh_token).await,
        OAuthProvider::ClaudeAI => claude::refresh(http_client(), &snap.refresh_token).await,
        OAuthProvider::KomaRun => komarun::refresh(http_client(), &snap.refresh_token).await,
        // Command Code keys never expire — always return cached.
        OAuthProvider::CommandCode => return (snap.access_token.clone(), snap.account.clone()),
        OAuthProvider::Kilocode => return (snap.access_token.clone(), snap.account.clone()),
        // W12: refresh via the manifest-declared generic OAuth2 `refresh_token` endpoint,
        // gated on the conn carrying BOTH a non-empty `refresh_token` AND a
        // `refresh_token_url`. Without both, koma cannot refresh (login-only, or the
        // extension owns the lifecycle) → serve the cached token verbatim, like Kilo Code.
        OAuthProvider::Extension => {
            match snap
                .refresh_token_url
                .as_deref()
                .filter(|u| !u.trim().is_empty())
            {
                Some(url) if !snap.refresh_token.is_empty() => {
                    ext_refresh(
                        http_client(),
                        url,
                        &snap.refresh_token,
                        snap.refresh_client_id.as_deref(),
                    )
                    .await
                }
                _ => return (snap.access_token.clone(), snap.account.clone()),
            }
        }
    };
    match refreshed {
        Ok(tokens) => {
            let refreshed_at = now_secs();
            persist_refresh(oauth_uuid, &tokens, refreshed_at);

            let mut updated = snap.clone();
            updated.access_token = tokens.access_token.clone();
            if !tokens.refresh_token.is_empty() {
                updated.refresh_token = tokens.refresh_token.clone();
            }
            updated.expires_at = tokens
                .expires_in
                .map(|secs| refreshed_at + secs)
                .unwrap_or(updated.expires_at);
            updated.last_refresh = refreshed_at;

            let bearer = updated.access_token.clone();
            let account = updated.account.clone();
            cache()
                .write()
                .await
                .insert(oauth_uuid.to_string(), updated);
            (bearer, account)
        }
        Err(e) => {
            // Never log token values; the error string here is already
            // scrubbed (status + trimmed body, or the unrecoverable marker).
            if e.starts_with("unrecoverable") {
                let mut unrecoverable = snap.clone();
                unrecoverable.unrecoverable = true;
                cache()
                    .write()
                    .await
                    .insert(oauth_uuid.to_string(), unrecoverable);
            }
            // Recoverable (or now-marked-unrecoverable) failure: fall back to
            // the cached, possibly stale, token rather than failing the send.
            (snap.access_token, snap.account)
        }
    }
}

/// Force a re-check of the token for `uuid` by evicting + re-seeding from disk.
/// Called after a 401 to pick up a token the OAuth daemon just refreshed.
pub async fn force_refresh(uuid: &str) {
    cache().write().await.remove(uuid);
    let config = AppConfig::load();
    if let Some(idx) = config.oauth_index_by_uuid(uuid) {
        seed(&config.oauth_conns[idx]).await;
    }
}

#[cfg(test)]
#[path = "manager_ext_refresh_tests.rs"]
mod ext_refresh_tests;
