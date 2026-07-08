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

fn default_palette() -> String {
    "dark".to_string()
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
    /// The OpenAI Responses API ("Codex") transport used by ChatGPT-subscription
    /// OAuth connections. Speaks a DIFFERENT wire protocol than
    /// `OpenAiCompatible` (`/responses`, typed SSE events, encrypted reasoning
    /// continuity) — dispatched by the dedicated `service::openrouter::codex`
    /// submodule. Set only via OAuth resolution, never user-selectable in the
    /// providers modal (serde `"codex"`).
    Codex,
    /// The koma-free keyless free tier: an OpenAI chat-completions endpoint
    /// (`service::koma_free::KOMA_FREE_ENDPOINT`) reached with two custom headers
    /// (`X-Koma` install id + `X-Session`) and NO `Authorization`, pinning the
    /// `koma/apple` model. Speaks the same chat-completions wire as
    /// `OpenAiCompatible`; only auth + the forced endpoint/model differ. Set only
    /// via the first-run chooser / `/free` toggle, never user-selectable in the
    /// providers modal (serde `"koma_free"`).
    KomaFree,
}

impl ApiType {
    /// Whether the runtime can actually dispatch a request against this wire type.
    /// `OpenAiCompatible` and `KomaFree` speak the OpenAI chat-completions contract
    /// (`KomaFree` is that wire with keyless dual-header auth); `Codex`
    /// speaks the OpenAI Responses API (all have real transports). Only
    /// `AnthropicCompatible` stays DEFERRED — native Anthropic Messages is a
    /// distinct protocol (its own adapter, not a rider on this pass), so it is
    /// treated as unroutable. The single source of truth shared by the
    /// resolution-boundary gate (`Resolved::is_routable`) and the UI affordance.
    pub fn is_routable(self) -> bool {
        matches!(self, ApiType::OpenAiCompatible | ApiType::Codex | ApiType::KomaFree)
    }
}

/// Which OAuth-backed provider an [`OAuthConn`] authenticates against. Distinct
/// from [`ApiType`]: OAuth connections carry their own token lifecycle (access/
/// refresh/id tokens, expiry) rather than a static `api_key`, so they are kept
/// in a separate catalogue (`AppConfig::oauth_conns`) instead of `providers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    #[default]
    Codex,
    Kilocode,
}

impl OAuthProvider {
    /// Human-facing label for the `/settings` OAuth submenu.
    pub fn label(&self) -> &'static str {
        match self {
            OAuthProvider::Codex => "Codex",
            OAuthProvider::Kilocode => "Kilo Code",
        }
    }
}

/// One OAuth-authenticated connection, keyed by `uuid`. Populated by the login
/// flow in `crate::service::oauth` and persisted alongside `providers`/`models`.
///
/// Every field carries `#[serde(default)]` so a partially-written or
/// older-schema config loads cleanly; `uuid` defaults to a freshly minted v4.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConn {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub provider: OAuthProvider,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    /// Unix seconds the access token expires at; `0` = never expires.
    #[serde(default)]
    pub expires_at: u64,
    /// Unix seconds of the last successful refresh; `0` = never refreshed.
    #[serde(default)]
    pub last_refresh: u64,
    /// Codex ChatGPT account id.
    #[serde(default)]
    pub account_id: String,
    /// Kilo Code organization id.
    #[serde(default)]
    pub org_id: String,
    #[serde(default)]
    pub email: String,
    /// Codex plan type (e.g. "plus", "pro").
    #[serde(default)]
    pub plan: String,
}

/// Runtime role slot a model can be assigned to. Each role is GLOBALLY exclusive
/// (a given role is held by at most ONE model), but a single model may carry
/// SEVERAL roles (e.g. Main + Awareness + Compactor). Persisted in lowercase
/// (`"main"`, `"awareness"`, …).
///
/// `Planner` drives the MAIN turn instead of `Main` while the session is in
/// `AgentMode::Plan` (see `app::resolve::resolve_turn_model`). It has no config
/// slot of its own beyond the assignment, no legacy fallback, and does NOT
/// inherit Main's route the way Compactor/Awareness do — an unassigned or
/// unresolved Planner simply means the turn stays on Main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Main,
    Awareness,
    Safeguard,
    Compactor,
    Planner,
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

    /// Normalize a raw `route` string before persisting it onto a `ModelEntry`:
    /// trims whitespace, then maps both an empty string and the literal `"auto"`
    /// sentinel (case-insensitive) to `None` (automatic OpenRouter routing). The
    /// GUI's Auto row round-trips the literal string `"auto"` through this same
    /// path that empty-route already went through — collapse both to `None` so
    /// nothing ever persists an `only: ["auto"]` provider pin (which upstream
    /// 404s: "No allowed providers are available for the selected model").
    ///
    /// Also self-heals a route poisoned with an OpenRouter endpoint's display
    /// LABEL instead of its provider name: OpenRouter's `/endpoints` `name`
    /// field is formatted `"Provider | model-variant"` (e.g.
    /// `"Xiaomi | xiaomi/mimo-v2.5-20260422"`), and a provider name itself never
    /// contains `" | "` — so when the separator is present, only the trimmed
    /// prefix is kept as the canonical pin. `helpers::provider_routing_for`
    /// (openrouter service) calls back into this same function so the
    /// live-request pin and the persisted config can never drift apart.
    pub fn normalize_route(route: Option<String>) -> Option<String> {
        let trimmed = route?;
        let trimmed = trimmed.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
            return None;
        }
        let stripped = match trimmed.split_once(" | ") {
            Some((prefix, _)) => prefix.trim(),
            None => trimmed,
        };
        if stripped.is_empty() || stripped.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(stripped.to_string())
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Active theme palette name (see view::theme::PALETTES). Replaces `theme`+`accent`.
    #[serde(default = "default_palette")]
    pub palette: String,
    /// DEPRECATED (kept for back-compat / old config.json round-trip; no longer read
    /// by the renderer — palette selection now lives in `palette`). Do not remove yet.
    #[serde(default)]
    pub theme: ThemeMode,
    /// DEPRECATED — see `palette`. Kept for back-compat; no longer drives rendering.
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
    /// OAuth-authenticated connections (Codex / Kilo Code). Empty by default;
    /// old config files (no such key) load with an empty vec.
    #[serde(default)]
    pub oauth_conns: Vec<OAuthConn>,
    /// Stable per-install identity sent as the `X-Koma` header on koma-free
    /// requests (the keyless free tier's rate-limit bucket). Minted once (serde
    /// default on an older config lacking the key; the manual `Default` for a
    /// missing/corrupt file) and persisted on the next `save()` — never cleared.
    #[serde(default = "new_uuid")]
    pub install_id: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            palette: default_palette(),
            theme: ThemeMode::default(),
            accent: default_accent(),
            providers: Vec::new(),
            models: Vec::new(),
            mcp_servers: Vec::new(),
            oauth_conns: Vec::new(),
            // Mint a stable install id even on the missing/corrupt-config fallback
            // path, so the koma-free `X-Koma` header is never empty; it is
            // persisted on the first `save()` and read back stably thereafter.
            install_id: new_uuid(),
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

    /// Index of the OAuth connection whose `uuid` matches, if any. Mirrors
    /// [`Self::provider_index_by_uuid`] for the OAuth catalogue.
    pub fn oauth_index_by_uuid(&self, uuid: &str) -> Option<usize> {
        self.oauth_conns.iter().position(|c| c.uuid == uuid)
    }

    /// Upsert a model into the GLOBAL catalogue by uuid, with per-role steal (see
    /// [`upsert_model_entry`]). The caller persists via [`Self::save`] afterwards.
    pub fn upsert_model(&mut self, entry: ModelEntry) {
        upsert_model_entry(&mut self.models, entry);
    }

    /// Remove the global model with `uuid` (no-op if none matches).
    pub fn remove_model_by_uuid(&mut self, uuid: &str) {
        self.models.retain(|m| m.uuid != uuid);
    }

    /// Upsert an API provider by uuid (the GUI Connector ProviderForm). A `Some(uuid)`
    /// matching an existing provider EDITS it in place — updating `name`/`endpoint`/
    /// `api_key` while PRESERVING its `api_type` (the form doesn't expose the wire type,
    /// so an OAuth/Codex/koma-free provider keeps its transport). A `None`/empty uuid, or
    /// a uuid with no match, CREATES a new [`ApiType::OpenAiCompatible`] provider (the
    /// default the TUI providers modal also starts from). The caller persists via
    /// [`Self::save`] afterwards.
    ///
    /// The plaintext key is never round-tripped to the webview (see [`PushProvider`] in
    /// `client::render`), so an EMPTY incoming `api_key` on edit means "unchanged" — the
    /// existing stored key is preserved. Only a non-empty incoming key overwrites it. On
    /// create, an empty key just stores empty (nothing to preserve).
    pub fn upsert_provider(
        &mut self,
        uuid: Option<String>,
        name: String,
        endpoint: String,
        api_key: String,
    ) {
        let uuid = uuid.filter(|u| !u.is_empty());
        if let Some(u) = uuid.as_deref() {
            if let Some(slot) = self.providers.iter_mut().find(|p| p.uuid == u) {
                slot.name = name;
                slot.endpoint = endpoint;
                if !api_key.is_empty() {
                    slot.api_key = api_key;
                }
                // `api_type` intentionally preserved (not exposed by the GUI form).
                return;
            }
        }
        self.providers.push(ProviderConn {
            uuid: uuid.unwrap_or_else(new_uuid),
            name,
            api_type: ApiType::OpenAiCompatible,
            endpoint,
            api_key,
        });
    }

    /// Remove the provider with `uuid` (no-op if none matches). Models referencing the
    /// removed provider keep their now-dangling `provider_uuid` (surfaces empty in the
    /// UI for re-pick), matching the TUI Settings-save behaviour — no cascade.
    pub fn remove_provider_by_uuid(&mut self, uuid: &str) {
        self.providers.retain(|p| p.uuid != uuid);
    }

    /// Upsert an MCP server by uuid: replace the entry whose uuid matches, else append.
    /// An entry arriving with an EMPTY uuid is treated as brand-new (a fresh uuid is
    /// minted). Config-layer setter shared by the GUI `SetMcpServer` handler; the caller
    /// persists via [`Self::save`] afterwards.
    pub fn upsert_mcp_server(&mut self, mut entry: McpServerEntry) {
        if entry.uuid.is_empty() {
            entry.uuid = new_uuid();
        }
        match self.mcp_servers.iter_mut().find(|s| s.uuid == entry.uuid) {
            Some(slot) => *slot = entry,
            None => self.mcp_servers.push(entry),
        }
    }

    /// Remove the MCP server with `uuid` (no-op if none matches).
    pub fn remove_mcp_server_by_uuid(&mut self, uuid: &str) {
        self.mcp_servers.retain(|s| s.uuid != uuid);
    }

    /// Set the `enabled` flag on the MCP server with `uuid`; returns whether one matched.
    pub fn set_mcp_enabled_by_uuid(&mut self, uuid: &str, enabled: bool) -> bool {
        match self.mcp_servers.iter_mut().find(|s| s.uuid == uuid) {
            Some(s) => {
                s.enabled = enabled;
                true
            }
            None => false,
        }
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

/// Upsert `entry` into a model `list` by uuid with per-role STEAL (the invariant that
/// each role is held by at most ONE model within a given scope): every role `entry` now
/// holds is first removed from every OTHER model in `list`, then `entry` replaces its
/// uuid-match (or is appended). An `entry` arriving with an EMPTY uuid is treated as
/// brand-new (a fresh uuid is minted).
///
/// Scope-agnostic on purpose: the GLOBAL catalogue (`AppConfig::models`, via
/// [`AppConfig::upsert_model`]) and each session's LOCAL override layer
/// (`settings.session_models`, called directly by the GUI `SetModel` handler) each keep
/// the invariant INDEPENDENTLY, so a global Main and a session Main can coexist (session
/// wins at resolve). Mirrors the TUI settings modal's per-scope role-steal, lifted to the
/// config layer so the daemon's gui handler and the mode share one implementation.
pub(crate) fn upsert_model_entry(list: &mut Vec<ModelEntry>, mut entry: ModelEntry) {
    if entry.uuid.is_empty() {
        entry.uuid = new_uuid();
    }
    for other in list.iter_mut() {
        if other.uuid != entry.uuid {
            other.roles.retain(|r| !entry.roles.contains(r));
        }
    }
    match list.iter_mut().find(|m| m.uuid == entry.uuid) {
        Some(slot) => *slot = entry,
        None => list.push(entry),
    }
}
