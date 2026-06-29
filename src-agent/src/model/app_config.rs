//! Global application configuration persisted to `~/.simple-coder/config.json`.
//!
//! Unlike per-session `settings.json`, this file stores user-wide preferences
//! that apply across all sessions: visual theme, accent colour, and any future
//! global knobs. It is loaded once at startup (after `ensure_dirs`) and never
//! written automatically — the user (or a future `/settings` command) calls
//! `save()` explicitly.
//!
//! On-disk format (pretty-printed JSON):
//! ```json
//! {
//!   "theme": "dark",
//!   "accent": "green"
//! }
//! ```
//!
//! Unknown keys are silently ignored (forward-compat); missing keys fall back
//! to defaults (back-compat). Any read error — file absent, parse failure,
//! permission denied — returns `AppConfig::default()` instead of propagating,
//! so a corrupt or missing config never prevents startup.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::model::store::base_dir;

/// Visual colour scheme.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

fn default_accent() -> String {
    "green".to_string()
}

/// Mint a fresh random UUID (v4) as a `String`. Used as the serde default for
/// the `uuid` field of [`ProviderConn`] / [`ModelEntry`] / [`McpServerEntry`] so
/// entries read from an old config file without a uuid get a stable identity on
/// load, and so new entries can be minted in Rust without a hand-rolled scheme.
///
/// `pub(crate)` so the `/mcp` dashboard can mint a uuid for a freshly-created
/// server entry through the same canonical config-layer helper (mirrors the
/// settings UI's own `new_uuid`).
pub(crate) fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Wire protocol an API provider connection speaks. Mirrors the UI-side
/// `ApiType`; this is the persisted form (serde snake_case).
///
/// `OpenAiCompatible` is the default — the OpenRouter/OpenAI chat-completions
/// wire is what the runtime currently speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    #[default]
    OpenAiCompatible,
    AnthropicCompatible,
}

impl ApiType {
    /// Whether the runtime can actually dispatch a request against this wire type.
    /// Only `OpenAiCompatible` routes today — the client speaks the OpenAI
    /// chat-completions contract exclusively. `AnthropicCompatible` is persisted +
    /// selectable in the UI but DEFERRED: native Anthropic Messages is a distinct
    /// protocol (its own adapter, not a rider on this pass), so it is treated as
    /// unroutable. The single source of truth shared by the resolution-boundary
    /// gate (`Resolved::is_routable`) and the UI affordance.
    pub fn is_routable(self) -> bool {
        matches!(self, ApiType::OpenAiCompatible)
    }
}

/// Runtime role slot a model can be assigned to. Each role is GLOBALLY exclusive
/// (a given role is held by at most ONE model), but a single model may carry
/// SEVERAL roles (e.g. Main + Awareness + Compactor). Persisted in lowercase
/// (`"main"`, `"awareness"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Main,
    Awareness,
    Safeguard,
    Compactor,
}

/// One API provider connection: a base URL + auth + wire type, keyed by `uuid`.
///
/// Every field carries `#[serde(default)]` so a partially-written or
/// older-schema config loads cleanly; `uuid` defaults to a freshly minted v4.
///
/// `Default` (all fields empty) backs the daemon thin client's KEYLESS
/// reconstruction of the `/agents` provider catalogue — it builds a `ProviderConn`
/// with just the `uuid`/`name`/`endpoint` the model label needs and an empty
/// `api_key` (no key ever crosses the wire). Note `Default::default()` yields an
/// EMPTY `uuid` (the `new_uuid` serde default is a deserialize-only fallback), so
/// callers that need a real id set it explicitly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderConn {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api_type: ApiType,
    /// Base URL, e.g. `https://openrouter.ai/api/v1`.
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
}

/// One model entry in the global catalogue. References its serving provider by
/// `provider_uuid`. `route` pins an OpenRouter upstream provider name; `roles`
/// lists the runtime slots this model holds (a model may hold several; each role
/// is globally unique, held by at most one model).
///
/// Back-compat: an older config wrote a single `role: Option<ModelRole>`. That
/// field is still READ (hidden, never re-serialized) so old entries migrate;
/// always go through [`Self::effective_roles`] to fold the legacy field into the
/// new list. On save we write `roles` and leave `role` `None`, so the legacy key
/// stops being emitted once a config is re-saved.
///
/// Every field carries `#[serde(default)]`; `uuid` defaults to a freshly minted
/// v4 and `route` is omitted from the JSON when `None`.
///
/// `Default` (all fields empty) backs the daemon thin client's KEYLESS
/// reconstruction of the `/agents` model catalogue (just `uuid`/`name`/`model_id`/
/// `provider_uuid`). As with [`ProviderConn`], `Default::default()` yields an empty
/// `uuid`; the reconstruction sets it explicitly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelEntry {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub provider_uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Runtime roles this model holds. Empty = unassigned. Serialized as-is.
    #[serde(default)]
    pub roles: Vec<ModelRole>,
    /// LEGACY single-role field: READ-ONLY back-compat. Deserialized from old
    /// configs but never written back (`skip_serializing`), so it silently
    /// migrates into `roles` via [`Self::effective_roles`].
    #[serde(default, skip_serializing)]
    pub role: Option<ModelRole>,
}

impl ModelEntry {
    /// The roles this entry effectively holds, folding in the legacy single-role
    /// field for back-compat: if `roles` is non-empty it wins; otherwise the
    /// legacy `role` (when `Some`) is promoted to a one-element list; otherwise
    /// empty. Every roles READ (resolver + load mapping) goes through this so a
    /// pre-multi-role config behaves identically until it's re-saved.
    pub fn effective_roles(&self) -> Vec<ModelRole> {
        if !self.roles.is_empty() {
            self.roles.clone()
        } else if let Some(r) = self.role {
            vec![r]
        } else {
            Vec::new()
        }
    }
}

/// Wire transport an MCP server speaks. `Stdio` (the default) launches a child
/// process and talks over its stdin/stdout; `Http` connects to a streamable-HTTP
/// MCP endpoint. Persisted serde snake_case (`"stdio"` / `"http"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
}

/// One configured MCP (Model Context Protocol) server, keyed by `uuid`.
///
/// koma connects to each ENABLED entry as an MCP client, discovers its tools, and
/// advertises them to the model (see [`crate::app::mcp`]). For now these entries
/// are managed by hand-editing `config.json`; there is no UI.
///
/// Every field carries `#[serde(default)]` so an older `config.json` (which had no
/// `mcp_servers` at all) — or a partially-written entry — loads cleanly. `uuid`
/// defaults to a freshly minted v4 so a hand-written entry without one still gets a
/// stable identity on load.
///
/// Transport-specific fields are unioned: a `Stdio` server uses `command` / `args`
/// / `env`; an `Http` server uses `url`. Unused fields for a given transport are
/// simply ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerEntry {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    /// Human-facing name; also the source of the `mcp__<name>__<tool>` namespace
    /// (sanitised at advertise time).
    #[serde(default)]
    pub name: String,
    /// When false, the server is skipped entirely (no connection, no tools).
    #[serde(default)]
    pub enabled: bool,
    /// Wire transport (`Stdio` default, or `Http`).
    #[serde(default)]
    pub transport: McpTransport,
    /// Stdio transport: the executable to launch (e.g. `npx`).
    #[serde(default)]
    pub command: String,
    /// Stdio transport: arguments passed to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Stdio transport: extra environment variables (`[["KEY","value"], …]`) set on
    /// the child process. A list of pairs (not a map) so the on-disk order is stable
    /// and round-trips predictably.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Http transport: the streamable-HTTP MCP endpoint URL.
    #[serde(default)]
    pub url: String,
}

/// Global user-facing configuration (theme + accent + provider/model catalogue).
///
/// All fields carry `#[serde(default)]` so the struct round-trips cleanly
/// when the on-disk file was written by an older version that lacked a field,
/// or when the file is absent entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default = "default_accent")]
    pub accent: String,
    /// Global catalogue of API provider connections, keyed by uuid.
    #[serde(default)]
    pub providers: Vec<ProviderConn>,
    /// Global catalogue of named models; each references a provider by uuid.
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Configured MCP servers. Empty by default; old config files (no such key)
    /// load with an empty vec, so behaviour is unchanged until a server is added.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerEntry>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            accent: default_accent(),
            providers: Vec::new(),
            models: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Load from `~/.simple-coder/config.json`.
    ///
    /// Returns `AppConfig::default()` on ANY error (file absent, parse failure,
    /// etc.) so startup is never blocked by a missing or corrupt config file.
    pub fn load() -> Self {
        let path = match base_dir() {
            Ok(d) => d.join("config.json"),
            Err(_) => return AppConfig::default(),
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return AppConfig::default(),
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Index of the provider whose `uuid` matches, if any. Used by the
    /// `/settings` load/save mapping to resolve a [`ModelEntry::provider_uuid`]
    /// back to the UI draft's positional `provider_idx`.
    pub fn provider_index_by_uuid(&self, uuid: &str) -> Option<usize> {
        self.providers.iter().position(|p| p.uuid == uuid)
    }

    /// Idempotent migration seed: synthesize the global provider/model catalogue
    /// from the legacy per-session `settings.*` fields the first time it's empty.
    ///
    /// Guard (returns `false`, no mutation): the catalogue is already configured
    /// (`providers` OR `models` non-empty), or there's nothing to seed from
    /// (`settings.api_key` empty — a fresh install with no key yet). Otherwise
    /// synthesizes ONE OpenRouter [`ProviderConn`] (endpoint [`DEFAULT_BASE_URL`],
    /// [`ApiType::OpenAiCompatible`], `api_key` from `settings.api_key`) plus a
    /// Main-role [`ModelEntry`] (`model_id` from `settings.model`, referencing the
    /// new provider's uuid, `route` from `settings.provider` when non-empty), and
    /// returns `true` so the caller persists `config.json`.
    ///
    /// The old `settings.*` fields are left untouched (downgrade-safe); the
    /// resolver's legacy fallback keeps working until this seed runs, after which
    /// the role-resolution path engages. Safe to call repeatedly — the guard makes
    /// every call after the first a no-op.
    ///
    /// Retained for the legacy/migration path: the first-run wizard now writes the
    /// catalogue directly from the entered endpoint, so the wizard no longer calls
    /// this — but it stays as the seed-from-`settings.*` migration entry point.
    #[allow(dead_code)] // legacy/migration seed; wizard writes config directly now
    pub fn seed_from_settings(&mut self, settings: &crate::model::settings::Settings) -> bool {
        if !self.providers.is_empty() || !self.models.is_empty() {
            return false; // already configured
        }
        if settings.api_key.is_empty() {
            return false; // nothing to seed from (fresh install, no key)
        }
        let provider_uuid = new_uuid();
        self.providers.push(ProviderConn {
            uuid: provider_uuid.clone(),
            name: "OpenRouter".to_string(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: crate::config::DEFAULT_BASE_URL.to_string(),
            api_key: settings.api_key.clone(),
        });
        self.models.push(ModelEntry {
            uuid: new_uuid(),
            name: "Main".to_string(),
            model_id: settings.model.clone(),
            provider_uuid,
            // Empty provider slug = OpenRouter default routing → no `route` pin.
            route: if settings.provider.is_empty() {
                None
            } else {
                Some(settings.provider.clone())
            },
            roles: vec![ModelRole::Main],
            role: None,
        });
        true
    }

    /// Serialise (pretty-printed) to `~/.simple-coder/config.json`.
    ///
    /// Called by the `/settings` dashboard when the user saves theme/accent
    /// changes.
    pub fn save(&self) -> Result<()> {
        let path = base_dir()?.join("config.json");
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
