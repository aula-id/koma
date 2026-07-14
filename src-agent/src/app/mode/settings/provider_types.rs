//! Provider-related types: [`ProviderDraft`], [`ProviderModal`], [`new_uuid`], and
//! UI-only `impl` blocks for [`ApiType`] and [`ModelRole`].

pub use crate::model::app_config::{ApiType, ModelRole, OAuthProvider};

impl ModelRole {
    pub fn label(&self) -> &'static str {
        match self {
            ModelRole::Main      => "main",
            ModelRole::Awareness => "awareness",
            ModelRole::Safeguard => "safeguard",
            ModelRole::Compactor => "compactor",
            ModelRole::Planner   => "planner",
        }
    }

    pub const ALL: [ModelRole; 5] = [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ];
}

impl ApiType {
    /// Short label used in the providers table column.
    pub fn short_label(self) -> &'static str {
        match self {
            ApiType::OpenAiCompatible   => "OpenAI",
            ApiType::AnthropicCompatible => "Anthropic",
            ApiType::Codex               => "Codex",
            ApiType::KomaFree            => "koma free",
        }
    }

    /// Full human-readable label for the api type. Kept for forward-compat; the UI
    /// Type field was removed (new providers are always `OpenAiCompatible`).
    #[allow(dead_code)]
    pub fn full_label(self) -> &'static str {
        match self {
            ApiType::OpenAiCompatible   => "OpenAI compatible",
            ApiType::AnthropicCompatible => "Anthropic (Claude)",
            ApiType::Codex               => "Codex (OAuth)",
            ApiType::KomaFree            => "koma free (keyless)",
        }
    }

    /// Flip between the two USER-SELECTABLE variants. `Codex` and `KomaFree` are
    /// set only via OAuth resolution / the koma-free chooser (never chosen in the
    /// providers modal), so both are EXCLUDED from the rotation: toggling off them
    /// lands back on the default and no user-selectable variant ever toggles INTO
    /// them. Kept for forward-compat; not called from the UI since the Type field
    /// was removed.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            ApiType::OpenAiCompatible   => ApiType::AnthropicCompatible,
            ApiType::AnthropicCompatible => ApiType::OpenAiCompatible,
            ApiType::Codex               => ApiType::OpenAiCompatible,
            ApiType::KomaFree            => ApiType::OpenAiCompatible,
        }
    }
}

/// Mint a fresh random UUID (v4) as a `String`. Used when CREATING a new
/// provider/model draft in the UI so its identity is stable across the edit
/// session (before the first config save) and matches the persisted
/// [`crate::model::app_config`] uuid scheme.
pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// One OAuth-authenticated connection (Codex / Kilo Code), drafted for display
/// in the model modal's provider cycle AND the `/settings` OAuth submenu. Built
/// from `config.oauth_conns` by [`OAuthDraft::from_config`] (called from
/// `SettingsState::from` and re-run after a login/delete); mutated only by the
/// OAuth submenu's connect/delete actions, never by the Models Select save path.
#[derive(Clone, Debug)]
pub struct OAuthDraft {
    pub uuid: String,
    /// `"codex (email@x.com)"` / `"kilocode (org-id)"` / `"codex (a1b2c3d4)"` —
    /// see [`OAuthDraft::from_config`] for exactly how this is built.
    pub label: String,
    pub provider: OAuthProvider,
    /// Snapshot of the access token, carried so `mm_provider_conn` can return it
    /// alongside the catalogue endpoint without a second config/oauth_conns
    /// lookup — same trust model as `ProviderDraft::api_key` (it already crosses
    /// the client wire in `ProviderDraftSnapshot`).
    pub key: String,
    /// Display status computed at build time from `expires_at`: `"active"` /
    /// `"renews in Nd"` / `"expired"` (Codex) or `"no expiry"` (Kilo Code, which
    /// carries no expiry in this flow). See [`oauth_status`].
    pub status: String,
}

impl OAuthDraft {
    /// Build the full OAuth-connection draft list from `config.oauth_conns`. The
    /// label prefers email, then org id, then a short uuid slug — so an entry
    /// always shows a recognisable identity. Re-run this (not a partial patch)
    /// after every login/delete so the list and every entry's `status` stay
    /// fresh.
    pub fn from_config(config: &crate::model::app_config::AppConfig) -> Vec<OAuthDraft> {
        config
            .oauth_conns
            .iter()
            .map(|c| {
                let short = match c.provider {
                    OAuthProvider::Codex => "codex",
                    OAuthProvider::Kilocode => "kilocode",
                    OAuthProvider::Xai => "xai",
                    OAuthProvider::ClaudeAI => "claude",
                    OAuthProvider::KomaRun => "koma",
                };
                let ident = if !c.email.is_empty() {
                    c.email.clone()
                } else if !c.org_id.is_empty() {
                    c.org_id.clone()
                } else {
                    c.uuid.chars().take(8).collect::<String>()
                };
                OAuthDraft {
                    uuid: c.uuid.clone(),
                    label: format!("{short} ({ident})"),
                    provider: c.provider,
                    key: c.access_token.clone(),
                    status: oauth_status(c),
                }
            })
            .collect()
    }
}

/// Compute the display status for one OAuth connection.
///
/// - `expires_at == 0` (Kilo Code always; a Codex entry with no known expiry) →
///   `"no expiry"`.
/// - Past `expires_at` → `"expired"`.
/// - More than a day out → `"renews in {N}d"`.
/// - Within the last day → `"active"`.
pub fn oauth_status(c: &crate::model::app_config::OAuthConn) -> String {
    if c.expires_at == 0 {
        return "no expiry".to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now >= c.expires_at {
        return "expired".to_string();
    }
    let days = (c.expires_at - now) / 86_400;
    if days >= 1 {
        format!("renews in {days}d")
    } else {
        "active".to_string()
    }
}

#[cfg(test)]
mod oauth_status_tests {
    use super::oauth_status;
    use crate::model::app_config::{OAuthConn, OAuthProvider};

    fn conn_with_expiry(expires_at: u64) -> OAuthConn {
        OAuthConn {
            provider: OAuthProvider::Codex,
            expires_at,
            ..Default::default()
        }
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    #[test]
    fn zero_expiry_is_no_expiry() {
        assert_eq!(oauth_status(&conn_with_expiry(0)), "no expiry");
    }

    #[test]
    fn past_expiry_is_expired() {
        let past = now_secs().saturating_sub(3600);
        assert_eq!(oauth_status(&conn_with_expiry(past)), "expired");
    }

    #[test]
    fn several_days_out_shows_days() {
        let later = now_secs() + 3 * 86_400 + 100;
        assert_eq!(oauth_status(&conn_with_expiry(later)), "renews in 3d");
    }

    #[test]
    fn within_a_day_is_active() {
        let soon = now_secs() + 3600;
        assert_eq!(oauth_status(&conn_with_expiry(soon)), "active");
    }
}

/// One API provider entry, mirrored to/from a persisted
/// [`crate::model::app_config::ProviderConn`]. `uuid` carries the persisted
/// identity so a reorder/delete/edit round-trips without losing the
/// model→provider linkage.
#[derive(Clone, Debug)]
pub struct ProviderDraft {
    /// Persisted identity (matches the `ProviderConn` uuid). Minted on create.
    pub uuid: String,
    pub name: String,
    pub endpoint: String,
    pub api_type: ApiType,
    pub api_key: String,
}

/// State for the "Add API provider" modal overlay.
#[derive(Clone, Debug)]
pub struct ProviderModal {
    pub name: String,
    pub endpoint: String,
    pub api_type: ApiType,
    pub api_key: String,
    /// Active field: 0=name, 1=endpoint, 2=api_key, 3=Save button, 4=Cancel button.
    pub field: usize,
}

impl ProviderModal {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            endpoint: String::new(),
            api_type: ApiType::OpenAiCompatible,
            api_key: String::new(),
            field: 0,
        }
    }
}
