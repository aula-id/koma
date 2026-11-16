//! Public config and session DTOs. They are built from private structs only by
//! the projections in `host::redact` and `configure::describe_all`, and none of
//! them has a field that can hold a key, a token or endpoint credentials.
use serde::Serialize;

/// `ProviderConn`s, `OAuthConn`s and `ModelEntry`s past these are left out of a
/// `ConfigView`, with the matching `*_truncated` flag set.
pub const MAX_PROVIDERS: usize = 256;
pub const MAX_OAUTH: usize = 64;
pub const MAX_MODELS: usize = 2000;
/// `SessionInfo.daemon.capabilities` and `config.effort.get` `options`.
pub const MAX_CAPABILITIES: usize = 64;
pub const MAX_EFFORT_OPTIONS: usize = 64;

/// One consistent read of `config.json`, plus the role routes of its view.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfigView {
    pub revision: String,
    pub file: &'static str,
    pub view: &'static str,
    pub providers: Vec<Provider>,
    pub providers_truncated: bool,
    pub oauth: Vec<OAuthConn>,
    pub oauth_truncated: bool,
    pub models: Vec<Model>,
    pub models_truncated: bool,
    pub roles: Vec<RoleRoute>,
    pub main_fallback: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Provider {
    pub provider_uuid: String,
    pub name: String,
    pub api_type: &'static str,
    pub endpoint: String,
    pub endpoint_redacted: bool,
    pub managed: bool,
    pub has_key: bool,
    pub key_state: &'static str,
    pub model_count: usize,
}

/// Read-only: OAuth connections are listed, never edited.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OAuthConn {
    pub oauth_uuid: String,
    pub name: String,
    pub provider: &'static str,
    pub has_token: bool,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Model {
    pub model_uuid: String,
    pub name: String,
    pub model_id: String,
    pub provider_uuid: String,
    pub provider_kind: &'static str,
    pub provider_name: Option<String>,
    pub route: Option<String>,
    pub roles: Vec<&'static str>,
    /// The entry holds a role outside the five public ones. Those roles are
    /// never named.
    pub other_roles: bool,
    pub source_uuid: Option<String>,
}

/// Where one public role resolves from. Mirrors `RoleResolution`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoleRoute {
    pub role: &'static str,
    pub origin: String,
    pub model_uuid: Option<String>,
    pub source_uuid: Option<String>,
    pub configured_model: Option<String>,
    pub provider_uuid: Option<String>,
    pub provider_name: Option<String>,
    pub effective_model: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Limits {
    pub max_output_tokens: u32,
    pub context_window_limit: u64,
    pub context_model_alias: String,
    pub effective_window: Option<u64>,
}

/// Per-session facts that travel beside the v1 `state`, never inside it. They
/// change without a revision bump; clients apply the newest `observed_at_ms`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionInfo {
    pub name: String,
    /// `"default"` when the session uses the model default.
    pub effort: String,
    pub routes: Vec<RoleRoute>,
    pub limits: Limits,
    /// Not reported yet; always null.
    pub usage: Option<serde_json::Value>,
    pub daemon: DaemonInfo,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct DaemonInfo {
    pub version: String,
    pub ipc_protocol: u32,
    pub capabilities: Vec<String>,
}

/// What happened after a committed write. Absent keys did not apply.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Propagation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rebind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rooms: Option<&'static str>,
}

impl Propagation {
    /// The daemon did its own save, broadcast and rebind.
    pub fn by_daemon() -> Self {
        Self {
            bound: Some("applied_by_daemon"),
            broadcast: Some("sent_by_daemon"),
            rebind: Some("done_by_daemon"),
            rooms: None,
        }
    }
}
