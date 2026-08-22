//! Command Code OAuth: browser-assisted API key retrieval via localhost POST
//! callback (NOT PKCE). The Studio website POSTs the API key back to a local
//! loopback server. Also supports static API key paste. Flow kind: "callback".
//!
//! Chat transport is discovered per-connection: try OpenAI-compat
//! `provider/v1/chat/completions` first (Provider+ plans); on plan rejection
//! fall back to NDJSON `POST /alpha/generate` (Go plan). The winner is stored
//! on [`OAuthConn::commandcode_chat`] so subsequent requests skip the probe.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use super::registry::{COMMANDCODE_AUTH_PATH, COMMANDCODE_STUDIO_BASE};
use crate::model::app_config::{new_uuid, AppConfig, OAuthConn, OAuthProvider};

/// Remembered chat transport: OpenAI-compat `/provider/v1/chat/completions`.
pub const CHAT_PROVIDER_V1: &str = "provider_v1";
/// Remembered chat transport: NDJSON `POST /alpha/generate`.
pub const CHAT_NDJSON: &str = "ndjson";

fn chat_pref_cache() -> &'static RwLock<HashMap<String, String>> {
    static CACHE: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Seed the process-wide chat-transport preference from a just-loaded/login conn.
pub fn seed_chat_pref(conn: &OAuthConn) {
    if conn.provider != OAuthProvider::CommandCode {
        return;
    }
    if let Some(pref) = conn.commandcode_chat.as_deref() {
        if let Ok(mut g) = chat_pref_cache().write() {
            g.insert(conn.uuid.clone(), pref.to_string());
        }
    }
}

/// Look up the remembered chat transport for `oauth_uuid` (cache, then disk).
/// Returns [`CHAT_PROVIDER_V1`], [`CHAT_NDJSON`], or `None` (not yet probed).
pub fn chat_pref(oauth_uuid: &str) -> Option<String> {
    if oauth_uuid.is_empty() {
        return None;
    }
    if let Ok(g) = chat_pref_cache().read() {
        if let Some(p) = g.get(oauth_uuid) {
            return Some(p.clone());
        }
    }
    let config = AppConfig::load();
    let pref = config
        .oauth_conns
        .iter()
        .find(|c| c.uuid == oauth_uuid)
        .and_then(|c| c.commandcode_chat.clone());
    if let Some(ref p) = pref {
        if let Ok(mut g) = chat_pref_cache().write() {
            g.insert(oauth_uuid.to_string(), p.clone());
        }
    }
    pref
}

/// Remember the working chat transport for this Command Code conn (cache + disk).
/// No-op if uuid empty, conn missing, or the value is already set to `mode`.
pub fn remember_chat_pref(oauth_uuid: &str, mode: &str) {
    if oauth_uuid.is_empty() {
        return;
    }
    if mode != CHAT_PROVIDER_V1 && mode != CHAT_NDJSON {
        return;
    }
    if let Ok(mut g) = chat_pref_cache().write() {
        g.insert(oauth_uuid.to_string(), mode.to_string());
    }
    let mut config = AppConfig::load();
    let Some(idx) = config.oauth_index_by_uuid(oauth_uuid) else {
        return;
    };
    let conn = &mut config.oauth_conns[idx];
    if conn.provider != OAuthProvider::CommandCode {
        return;
    }
    if conn.commandcode_chat.as_deref() == Some(mode) {
        return;
    }
    conn.commandcode_chat = Some(mode.to_string());
    let _ = config.save();
}

/// Whether an HTTP error body indicates the key lacks Provider API access
/// (Go plan 403 on `/provider/v1/chat/completions`).
pub fn is_provider_api_denied(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::FORBIDDEN && status.as_u16() != 402 {
        return false;
    }
    let b = body.to_ascii_lowercase();
    b.contains("doesn't include api access")
        || b.contains("does not include api access")
        || b.contains("upgrade to provider")
        || b.contains("go plan")
        || (b.contains("api access") && b.contains("upgrade"))
}

/// Build the Command Code Studio authorization URL. The browser opens this;
/// after authentication, the Studio website POSTs the API key to
/// `http://localhost:{port}/callback` with a CSRF `state` token.
pub fn build_auth_url(port: u16, state: &str) -> String {
    format!(
        "{COMMANDCODE_STUDIO_BASE}{COMMANDCODE_AUTH_PATH}?callback=http://localhost:{port}/callback&state={state}"
    )
}

/// Generate a random 32-byte state token, base64url-encoded. Uses two UUIDs
/// concatenated (256 bits total) — good enough for CSRF.
pub fn generate_state() -> String {
    use base64::Engine;
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(a.as_bytes());
    bytes[16..].copy_from_slice(b.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Build an [`OAuthConn`] from a completed callback or pasted API key. Keys
/// never expire (`expires_at = 0`).
pub fn to_conn(api_key: &str, user_name: &str, user_id: &str) -> OAuthConn {
    let label = if !user_name.is_empty() {
        user_name.to_string()
    } else if !user_id.is_empty() {
        user_id.to_string()
    } else {
        "unknown".to_string()
    };

    OAuthConn {
        uuid: new_uuid(),
        name: format!("commandcode ({label})"),
        provider: OAuthProvider::CommandCode,
        access_token: api_key.to_string(),
        refresh_token: api_key.to_string(),
        id_token: String::new(),
        expires_at: 0,
        last_refresh: 0,
        account_id: user_id.to_string(),
        org_id: String::new(),
        email: user_name.to_string(),
        plan: String::new(),
        ext_id: None,
        provider_id: None,
        chat_endpoint: None,
        api_type: None,
        refresh_token_url: None,
        refresh_client_id: None,
        commandcode_chat: None,
    }
}

#[cfg(test)]
#[path = "commandcode_test.rs"]
mod tests;
