//! Global application configuration persisted to `~/.koma/config.json`.
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

/// serde default for boolean fields that should be `true` when absent from an
/// older config (e.g. [`InstalledExtension::enabled`]): a freshly-installed
/// extension is enabled unless explicitly disabled.
fn default_true() -> bool {
    true
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
    /// (`KomaFree` is that wire with keyless dual-header auth); `Codex` speaks the
    /// OpenAI Responses API; `AnthropicCompatible` speaks the native Anthropic
    /// Messages API — all four have real transports (see the `codex` / `anthropic`
    /// submodules). The single source of truth shared by the resolution-boundary
    /// gate (`Resolved::is_routable`) and the UI affordance.
    pub fn is_routable(self) -> bool {
        matches!(
            self,
            ApiType::OpenAiCompatible
                | ApiType::AnthropicCompatible
                | ApiType::Codex
                | ApiType::KomaFree
        )
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
    /// xAI (Grok): an RFC 8628 device-code grant (like Kilo Code) but with a
    /// discovered token endpoint + refresh tokens (see `service::oauth::xai`).
    /// Resolves to an `OpenAiCompatible` chat route with an EMPTY `account_id`
    /// (no org concept), so the Kilo org header never fires for it.
    Xai,
    /// Claude (Anthropic) via claude.ai OAuth: a PKCE authorization-code grant with a
    /// local loopback callback (see `service::oauth::claude`), mirroring Codex's flow
    /// shape but against Anthropic's own endpoints.
    ClaudeAI,
    /// Koma (koma.run) account login: a PKCE authorization-code grant with a local
    /// loopback callback (see `service::oauth::komarun`), cloned from the Claude flow
    /// shape but against koma.run's native-client OAuth endpoints (form-encoded token
    /// exchange, no client_id/scope). Account login only — not a model provider yet.
    KomaRun,
    /// W11: a token stored by an EXTENSION-delegated OAuth flow. The actual provider
    /// identity lives in the connection's `ext_id`/`provider_id` fields, not this enum
    /// (which stays `Copy` + closed) — this variant is just the "backed by an extension"
    /// marker. Account login / token storage ONLY in v1: it is NOT a model provider yet,
    /// so every native code path treats it as an inert placeholder (W12 wires ext tokens
    /// as resolvable model providers). Serde tag `"extension"`; deliberately NOT mapped
    /// back by [`Self::from_wire_id`] (ext flows route through the `ext:<id>:<provider>`
    /// picker id, never a bare wire token).
    Extension,
}

impl OAuthProvider {
    /// Human-facing label for the `/settings` OAuth submenu.
    pub fn label(&self) -> &'static str {
        match self {
            OAuthProvider::Codex => "Codex",
            OAuthProvider::Kilocode => "Kilo Code",
            OAuthProvider::Xai => "xAI",
            OAuthProvider::ClaudeAI => "Claude",
            OAuthProvider::KomaRun => "Koma",
            // W11: generic marker label; a real ext-backed conn's picker row uses the
            // extension manifest's provider `name`, never this (see
            // `requests_oauth::ext_oauth_rows_for`).
            OAuthProvider::Extension => "Extension",
        }
    }

    /// The stable wire id token (`"codex"` / `"kilocode"`) — identical to the serde
    /// `snake_case` tag, but available as a `&str` WITHOUT serializing. The `StartOAuth`
    /// GUI request keys on this, and the tokenless [`OAuthConn`]→wire projection stamps
    /// it as the connection's `provider`, so the webview never needs to see the raw enum.
    pub fn wire_id(&self) -> &'static str {
        match self {
            OAuthProvider::Codex => "codex",
            OAuthProvider::Kilocode => "kilocode",
            OAuthProvider::Xai => "xai",
            OAuthProvider::ClaudeAI => "claudeai",
            OAuthProvider::KomaRun => "komarun",
            // W11: stamped as the `provider` on an ext-backed conn's tokenless wire
            // projection (so the webview sees a stable marker); the connection's real
            // identity is its `ext_id`/`provider_id`. NOT a `from_wire_id` input.
            OAuthProvider::Extension => "extension",
        }
    }

    /// The exact inverse of [`Self::wire_id`] for the NATIVE flow-driving variants:
    /// resolve a `StartOAuth` wire string
    /// (`"codex"` / `"kilocode"` / `"xai"` / `"claudeai"` / `"komarun"`) back to its
    /// [`OAuthProvider`]. `None` for anything else, INCLUDING `"codex_paste"` — that
    /// token selects the paste-token input screen, not a real flow-driving provider,
    /// so it is deliberately not an `OAuthProvider` variant (see
    /// [`Self::flow_kind`]'s doc) — AND `"extension"` (the W11 storage marker), whose
    /// flows route through the `ext:<extension_id>:<provider_id>` picker id in
    /// `start_oauth`, never a bare wire token. Shared by every `StartOAuth` caller that needs the
    /// mapping (the daemon's `hub::requests_oauth::start_oauth` and the GUI host-
    /// relay's detached `HostCtl::StartOAuth` handler) so the wire contract has one
    /// source of truth instead of two hand-written `match`es drifting apart.
    pub fn from_wire_id(id: &str) -> Option<Self> {
        match id {
            "codex" => Some(OAuthProvider::Codex),
            "kilocode" => Some(OAuthProvider::Kilocode),
            "xai" => Some(OAuthProvider::Xai),
            "claudeai" => Some(OAuthProvider::ClaudeAI),
            "komarun" => Some(OAuthProvider::KomaRun),
            _ => None,
        }
    }

    /// The login flow SHAPE, for the data-driven GUI provider list: `"pkce"` (Codex's
    /// browser loopback authorization-code grant) or `"device"` (Kilo Code's device-code
    /// grant). The Codex paste-token option is NOT an `OAuthProvider` variant (it reuses
    /// Codex's connection shape), so its `"paste"` kind is carried separately by
    /// [`crate::service::oauth::registry::oauth_providers`].
    pub fn flow_kind(&self) -> &'static str {
        match self {
            OAuthProvider::Codex => "pkce",
            OAuthProvider::Kilocode => "device",
            OAuthProvider::Xai => "device",
            OAuthProvider::ClaudeAI => "pkce",
            OAuthProvider::KomaRun => "pkce",
            // W11: never surfaced through the enum-driven `oauth_providers()` list (ext
            // rows carry their own kind, mapped from the manifest `method` — see
            // `requests_oauth::method_to_kind`), so this value is exhaustiveness-only and
            // never reaches the picker. An inert placeholder.
            OAuthProvider::Extension => "paste",
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
    /// W11: for an EXTENSION-delegated connection (`provider == Extension`), the id of
    /// the extension that owns this token's login flow. `None` for every native conn.
    /// `skip_serializing_if` keeps a native conn's on-disk JSON BYTE-IDENTICAL to the
    /// pre-W11 shape (the field is simply absent), and `default` lets an older config
    /// without the key load cleanly. W12 adds `chat_endpoint`/`api_type` alongside these
    /// to make an ext token a resolvable model provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext_id: Option<String>,
    /// W11: the extension-local provider id (its manifest `oauth_providers[].id`) this
    /// token was minted for. `None` for every native conn (see [`Self::ext_id`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
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
    /// For a session-local override CLONED from a global entry, the `uuid` of that
    /// global — the EXACT identity the GUI model quick-picker matches against to
    /// light the active row (rather than a fuzzy name/model_id compare). `None` for
    /// a directly-authored entry (every global catalogue entry, a TUI/settings-
    /// authored model, a koma-free mint). `#[serde(default)]` so an older config
    /// lacking the key deserializes to `None` (backward-compatible); omitted from the
    /// JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
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

/// One installed extension's persisted registry entry.
///
/// Written by the install path ([`crate::app::ext::install`]) after a signed
/// package is verified and unpacked into `~/.koma/extensions/<id>/`, and read at
/// boot to auto-start enabled daemon-kind extensions. Deliberately a FLAT, string-
/// typed projection of the extension's manifest (not the manifest itself): `tier`,
/// `kind`, and `granted` store the serde WIRE strings (`"free"`/`"paid"`,
/// `"daemon"`/`"oneshot"`, `"agents:read"`, …) rather than the
/// [`koma_extension`](koma_extension::protocol) enums, so persistence never couples
/// to the wire crate's serde shape (no cross-crate serde cycle) and an unknown
/// future variant degrades to an opaque string instead of failing the whole config
/// load.
///
/// Every field carries `#[serde(default)]` (with `enabled` defaulting to `true`) so
/// an older `config.json` — or a partially-written entry — loads cleanly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledExtension {
    /// Reverse-DNS manifest id, e.g. `"run.koma.example.echo-tool-daemon"`. The key
    /// for every registry op and the on-disk `extensions/<id>/` directory name.
    pub id: String,
    /// Manifest version string.
    pub version: String,
    /// Manifest tier as a wire string: `"free"` | `"paid"`.
    pub tier: String,
    /// Grants koma has extended to this extension, as wire strings (e.g.
    /// `"agents:read"`). Echoed from the manifest `requires` for now; real
    /// grant enforcement is a later wave.
    #[serde(default)]
    pub granted: Vec<String>,
    /// When false, the extension is skipped at boot (not auto-started). Defaults to
    /// `true` so a freshly-installed extension is live.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Manifest kind as a wire string: `"daemon"` | `"oneshot"`.
    #[serde(default)]
    pub kind: String,
    /// Manifest `runtime.exec`, relative to the package root (e.g.
    /// `"bin/echo-tool-daemon"`). Resolved against `extensions/<id>/` at spawn.
    #[serde(default)]
    pub exec: String,
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
    /// Installed extensions (the on-disk registry). Empty by default; old config
    /// files (no such key) load with an empty vec, so behaviour is unchanged until
    /// an extension is installed. Read at boot to auto-start enabled daemons.
    #[serde(default)]
    pub installed_extensions: Vec<InstalledExtension>,
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
            installed_extensions: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Load from `~/.koma/config.json`.
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

    /// Upsert an installed extension by `id`: replace the entry whose id matches,
    /// else append. Mirrors [`Self::upsert_mcp_server`] but keyed by the extension's
    /// reverse-DNS id (not a uuid). The caller persists via [`Self::save`] afterwards.
    // Registry building block for the install/uninstall command wiring (a later wave);
    // dead until then, like `seed_from_settings`.
    #[allow(dead_code)]
    pub fn upsert_extension(&mut self, ext: InstalledExtension) {
        match self.installed_extensions.iter_mut().find(|e| e.id == ext.id) {
            Some(slot) => *slot = ext,
            None => self.installed_extensions.push(ext),
        }
    }

    /// Remove the installed extension with `id` (no-op if none matches). Used by the
    /// uninstall path to purge the registry entry (the on-disk `extensions/<id>/`
    /// dir is removed separately, mirroring the internet/security uninstall shape).
    #[allow(dead_code)]
    pub fn remove_extension_by_id(&mut self, id: &str) {
        self.installed_extensions.retain(|e| e.id != id);
    }

    /// The installed extension with `id`, if any. Mirrors the `*_by_uuid` lookups.
    #[allow(dead_code)]
    pub fn extension_by_id(&self, id: &str) -> Option<&InstalledExtension> {
        self.installed_extensions.iter().find(|e| e.id == id)
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
            source_uuid: None,
        });
        true
    }

    /// Serialise (pretty-printed) to `~/.koma/config.json`.
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

/// Strip `role` from `entry`'s EFFECTIVE role set so [`ModelEntry::effective_roles`]
/// can never surface it again — regardless of which field carried it. The role is
/// removed from the `roles` vec, AND when the LEGACY single-`role` field equals `role`
/// it is cleared to `None`.
///
/// The dual clear is load-bearing: `effective_roles` folds the legacy field in ONLY
/// when `roles` is empty, so demoting via the vec alone would leave a role carried
/// solely by an OLD config's legacy field intact — and [`resolve_role`]'s first-wins
/// scan would then still find that stale second holder and shadow the intended one.
///
/// [`resolve_role`]: crate::app::resolve::resolve_role
pub(crate) fn strip_role(entry: &mut ModelEntry, role: ModelRole) {
    entry.roles.retain(|r| *r != role);
    if entry.role == Some(role) {
        entry.role = None;
    }
}

/// Upsert `entry` into a model `list` by uuid with per-role STEAL (the invariant that
/// each role is held by at most ONE model within a given scope): every role `entry` now
/// holds is first removed from every OTHER model in `list` — from BOTH its `roles` vec
/// and its legacy `role` field (via [`strip_role`]) — then `entry` replaces its
/// uuid-match (or is appended). An `entry` arriving with an EMPTY uuid is treated as
/// brand-new (a fresh uuid is minted).
///
/// The legacy-field clear is what makes the FIRST-WINS resolver correct: without it, an
/// OTHER model that held the stolen role only through its legacy single-`role` field
/// (an old config) would stay an effective holder, and `resolve_role` could pick it over
/// the just-saved entry.
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
    // The roles the incoming entry claims — folded through `effective_roles` so a
    // (hypothetical) legacy-field entry still steals the right role from others.
    let claimed = entry.effective_roles();
    for other in list.iter_mut() {
        if other.uuid != entry.uuid {
            for role in &claimed {
                strip_role(other, *role);
            }
        }
    }
    match list.iter_mut().find(|m| m.uuid == entry.uuid) {
        Some(slot) => *slot = entry,
        None => list.push(entry),
    }
}

#[cfg(test)]
mod oauth_provider_wire_tests {
    use super::OAuthProvider;

    /// [`OAuthProvider::from_wire_id`] must be the exact inverse of [`OAuthProvider::wire_id`]
    /// for every real variant — this is the one mapping BOTH the daemon's attached
    /// `StartOAuth` handler and the GUI host-relay's detached path resolve a wire string
    /// through, so a drift here silently breaks one side or the other.
    #[test]
    fn from_wire_id_round_trips_every_variant() {
        for p in [
            OAuthProvider::Codex,
            OAuthProvider::Kilocode,
            OAuthProvider::Xai,
            OAuthProvider::ClaudeAI,
            OAuthProvider::KomaRun,
        ] {
            assert_eq!(OAuthProvider::from_wire_id(p.wire_id()), Some(p));
        }
    }

    /// `"codex_paste"` selects the paste-token input screen, not a real flow-driving
    /// provider — it must resolve to `None` so callers route it to the paste path
    /// instead of mistaking it for (or falling back to) a real provider.
    #[test]
    fn from_wire_id_rejects_codex_paste_and_unknown() {
        assert_eq!(OAuthProvider::from_wire_id("codex_paste"), None);
        assert_eq!(OAuthProvider::from_wire_id("not_a_provider"), None);
        assert_eq!(OAuthProvider::from_wire_id(""), None);
        // W11: the `extension` storage marker is NOT a from_wire_id input — ext flows
        // route through the `ext:<id>:<provider>` picker id, never a bare token.
        assert_eq!(OAuthProvider::from_wire_id("extension"), None);
    }
}

#[cfg(test)]
mod oauth_conn_serde_tests {
    use super::{OAuthConn, OAuthProvider};

    /// A NATIVE OAuthConn exactly as pre-W11 `config.json` wrote it — no
    /// `ext_id`/`provider_id` keys, field order matching the struct declaration
    /// (which is the order `serde_json` emits). The W11 change MUST NOT alter this.
    const NATIVE_CONN_JSON: &str = concat!(
        r#"{"uuid":"11111111-1111-1111-1111-111111111111","name":"codex (me@example.com)","#,
        r#""provider":"codex","access_token":"at","refresh_token":"rt","id_token":"it","#,
        r#""expires_at":1750000000,"last_refresh":1749000000,"account_id":"acc","org_id":"","#,
        r#""email":"me@example.com","plan":"pro"}"#
    );

    /// SERDE-COMPAT PROOF: a pre-W11 native conn deserializes cleanly (the two new
    /// `Option` fields default to `None`) AND re-serializes BYTE-IDENTICALLY. The new
    /// fields carry `skip_serializing_if = "Option::is_none"`, so a native conn never
    /// emits them — existing `config.json` files round-trip unchanged.
    #[test]
    fn native_conn_roundtrips_byte_stable() {
        let conn: OAuthConn = serde_json::from_str(NATIVE_CONN_JSON).expect("pre-W11 conn parses");
        assert_eq!(conn.provider, OAuthProvider::Codex);
        assert!(conn.ext_id.is_none());
        assert!(conn.provider_id.is_none());
        let reser = serde_json::to_string(&conn).expect("serializes");
        assert_eq!(
            reser, NATIVE_CONN_JSON,
            "a native OAuthConn must round-trip byte-identically after W11"
        );
    }

    /// An EXT-backed conn serializes with the `"extension"` provider tag plus the two
    /// ext fields, and round-trips back to an equal value.
    #[test]
    fn ext_conn_roundtrips() {
        let conn = OAuthConn {
            uuid: "22222222-2222-2222-2222-222222222222".to_string(),
            name: "Demo account".to_string(),
            provider: OAuthProvider::Extension,
            access_token: "demo-at".to_string(),
            email: "demo@example.com".to_string(),
            ext_id: Some("run.koma.example.oauth-demo-daemon".to_string()),
            provider_id: Some("demo".to_string()),
            ..Default::default()
        };
        let v = serde_json::to_value(&conn).expect("serializes");
        assert_eq!(v["provider"], "extension");
        assert_eq!(v["ext_id"], "run.koma.example.oauth-demo-daemon");
        assert_eq!(v["provider_id"], "demo");

        let back: OAuthConn = serde_json::from_value(v).expect("ext conn roundtrips");
        assert_eq!(back.provider, OAuthProvider::Extension);
        assert_eq!(back.ext_id.as_deref(), Some("run.koma.example.oauth-demo-daemon"));
        assert_eq!(back.provider_id.as_deref(), Some("demo"));
        assert_eq!(back.access_token, "demo-at");
    }
}
