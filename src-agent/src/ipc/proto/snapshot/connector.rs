// ─── provider/model connector drafts + the add/edit-model modal ──────────────

use serde::{Deserialize, Serialize};

/// A serde-safe mirror of ModelEndpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelEndpointWire {
    pub name: Option<String>,
    pub provider_name: Option<String>,
    pub price_prompt: Option<String>,
    pub price_completion: Option<String>,
    pub uptime_last_30m: Option<f64>,
}

/// A TOKENLESS serde-safe projection of one persisted OAuth connection
/// ([`crate::model::app_config::OAuthConn`]) for the streaming GUI OAuth surface
/// ([`crate::ipc::proto::DaemonEvent::OAuthState`]).
///
/// CRITICAL: this carries ONLY display/identity fields — `uuid`/`name`/`provider`/`email`/
/// `plan`/`account_id`. The `access_token`/`refresh_token`/`id_token` are DELIBERATELY
/// absent from the wire type so a secret can never be serialized to the webview even by
/// mistake. `provider` is the wire token (`"codex"` / `"kilocode"`, from
/// [`crate::model::app_config::OAuthProvider::wire_id`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OAuthConnWire {
    pub uuid: String,
    pub name: String,
    /// Wire provider token: `"codex"` | `"kilocode"`.
    pub provider: String,
    pub email: String,
    pub plan: String,
    pub account_id: String,
}

/// A serde-safe projection of one AVAILABLE OAuth login provider for the GUI's
/// `GetOAuthState` reply, built from the data-driven
/// [`crate::service::oauth::registry::oauth_providers`] source of truth. `id` is the
/// `StartOAuth` wire token, `label` the human name, `kind` the flow shape
/// (`"pkce"` / `"device"` / `"paste"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OAuthProviderWire {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// A serde-safe projection of one API-provider draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProviderDraftSnapshot {
    pub uuid: String,
    pub name: String,
    pub endpoint: String,
    pub api_type: String,
    pub api_key: String,
}

/// A serde-safe projection of one OAuth connection draft (provider-cycle merge
/// in the Models Select modal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OAuthDraftSnapshot {
    pub uuid: String,
    pub label: String,
    /// Wire token: `"codex"` | `"kilocode"`.
    pub provider: String,
    pub key: String,
    /// Display status computed at build time: `"active"` / `"renews in Nd"` /
    /// `"expired"` / `"no expiry"`. `#[serde(default)]` keeps an older peer's
    /// snapshot (pre-OAuth-submenu) decoding cleanly (empty string).
    #[serde(default)]
    pub status: String,
}

/// A serde-safe projection of the `/settings` OAuth submenu's connect-flow state
/// ([`crate::app::mode::settings::OAuthFlowState`]). Flat rather than a wire enum
/// (simplest shape): `kind` is the active variant's tag and every variant's data
/// rides in whichever of the other fields it needs; unused fields sit at their
/// default. `url` is dual-purpose — the Codex authorization URL for `codex_wait`,
/// the Kilo Code verification URL for `kilo_wait`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct OAuthFlowSnapshot {
    /// `"idle"` | `"starting"` | `"pick"` | `"codex_wait"` | `"codex_paste"` |
    /// `"kilo_wait"` | `"failed"`.
    pub kind: String,
    /// `Pick`'s cursor (0=Codex, 1=Kilo Code, 2=Codex paste-token).
    pub cursor: usize,
    /// `codex_wait`'s authorization URL / `kilo_wait`'s verification URL.
    pub url: String,
    /// `kilo_wait`'s device code the user approves.
    pub user_code: String,
    /// `codex_paste`'s in-progress token draft.
    pub input: String,
    /// `failed`'s human-readable reason.
    pub error: String,
    /// `codex_wait`/`kilo_wait`'s braille-spinner frame counter, advanced
    /// daemon-side every tick while the flow is in flight.
    pub frame: u8,
    /// `codex_wait`/`kilo_wait`'s "url copied to clipboard" confirmation flag,
    /// set after a successful `c` (copy-url) key press.
    pub copied: bool,
}

/// A serde-safe projection of one model draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelDraftSnapshot {
    pub uuid: String,
    pub name: String,
    pub model_id: String,
    pub provider_idx: usize,
    pub roles: Vec<String>,
    pub route: Option<String>,
    pub session_only: bool,
}

/// A serde-safe projection of the add-provider modal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProviderModalSnapshot {
    pub name: String,
    pub endpoint: String,
    pub api_type: String,
    pub api_key: String,
    pub field: usize,
}

/// A serde-safe projection of the role multi-select picker overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RolePickerSnapshot {
    pub checked: Vec<bool>,
    pub cursor: usize,
}

/// A serde-safe projection of the add/edit-model modal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelModalSnapshot {
    pub editing_idx: Option<usize>,
    pub uuid: String,
    pub name: String,
    pub provider_idx: usize,
    pub model_id: String,
    pub field: usize,
    pub roles: Vec<String>,
    pub role_picker: Option<RolePickerSnapshot>,
    pub query: String,
    pub result_sel: usize,
    pub route: Option<String>,
    pub route_sel: usize,
    pub endpoints: Option<Vec<ModelEndpointWire>>,
    pub endpoints_loading: bool,
    pub endpoints_for: Option<String>,
    /// Scope chosen at open time: `false` = global, `true` = session-local.
    #[serde(default)]
    pub session_only: bool,
}
