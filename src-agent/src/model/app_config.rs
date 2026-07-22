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

use crate::model::store::base_dir;
use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    /// Command Code's `/alpha/generate` NDJSON transport. Used when a Command Code
    /// connection's remembered preference is `"ndjson"` (Go plan — provider/v1 chat
    /// returns 403). Provider-plan keys stay on `OpenAiCompatible` against
    /// `/provider/v1`. Set only via OAuth resolution, never user-selectable
    /// (serde `"command_code"`).
    CommandCode,
}

impl ApiType {
    /// Whether the runtime can actually dispatch a request against this wire type.
    pub fn is_routable(self) -> bool {
        matches!(
            self,
            ApiType::OpenAiCompatible
                | ApiType::AnthropicCompatible
                | ApiType::Codex
                | ApiType::KomaFree
                | ApiType::CommandCode
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
    /// Command Code: browser posts API key to localhost callback (NOT auth-code PKCE).
    /// Chat: try OpenAI-compat `provider/v1` first; on plan rejection fall back to
    /// NDJSON `/alpha/generate` and remember the working transport on the conn.
    /// Catalogue: https://api.commandcode.ai/provider/v1 (`/models`).
    /// Flow kind: "callback".
    CommandCode,
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
            OAuthProvider::CommandCode => "Command Code",
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
            OAuthProvider::CommandCode => "commandcode",
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
            "commandcode" => Some(OAuthProvider::CommandCode),
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
            OAuthProvider::CommandCode => "callback",
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
    /// W12: the chat-completions endpoint an ext-backed token resolves to (captured at
    /// login from the extension manifest's `OAuthProviderDef.chat_endpoint`). Present
    /// only when the extension declared this provider as a MODEL provider (not
    /// account-login-only). `None` for every native conn and every account-login-only
    /// ext conn. Flat `Option` with `skip_serializing_if` so a native conn's on-disk
    /// JSON stays BYTE-IDENTICAL (the field is simply absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_endpoint: Option<String>,
    /// W12: the wire protocol that endpoint speaks, NORMALIZED at storage time to one of
    /// `"openai"` / `"anthropic"` (an unrecognised or absent manifest `api_type` stores
    /// `None`, which makes the conn account-login-only — `models.register` refuses it and
    /// resolution treats a referencing entry as dangling). `None` for every native conn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_type: Option<String>,
    /// W12: the token endpoint koma POSTs a generic OAuth2 `refresh_token` grant to when an
    /// ext-backed token nears expiry (from the manifest's `OAuthRefreshDef.token_url`).
    /// `None` = koma never refreshes this conn itself (the extension owns the lifecycle, or
    /// the token never expires). `None` for every native conn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token_url: Option<String>,
    /// W12: the `client_id` sent with the ext token-refresh grant, when the manifest
    /// declared one (some token endpoints require it; others identify the client by the
    /// refresh token alone). `None` for every native conn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_client_id: Option<String>,
    /// Command Code only: remembered working chat transport after the first successful
    /// probe. `"provider_v1"` (OpenAI-compat `/provider/v1/chat/completions`, Provider+
    /// plans) or `"ndjson"` (`POST /alpha/generate`, Go plan). `None` = unknown — try
    /// provider/v1 first, fall back to NDJSON on plan/API rejection, and persist the
    /// winner. `None` for every non-CommandCode conn; `skip_serializing_if` keeps their
    /// on-disk JSON byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commandcode_chat: Option<String>,
}

impl OAuthConn {
    /// W12: this ext-backed conn's chat route parts IFF it is a usable MODEL provider — a
    /// non-empty stored `chat_endpoint` AND a recognised `api_type`
    /// (`"openai"` → [`ApiType::OpenAiCompatible`], `"anthropic"` →
    /// [`ApiType::AnthropicCompatible`], the two wire types the native OAuth arms produce).
    /// `None` for an account-login-only ext conn (no endpoint / unrecognised or absent
    /// api_type) and for every native conn (whose W12 meta fields are always `None`).
    ///
    /// The SINGLE source of truth for "is this ext conn a model provider": the resolution
    /// boundary ([`crate::app::resolve`]) builds the route from this, and the
    /// `models.register` broker verb gates on it, so the two can never disagree about which
    /// conns can serve a registered model.
    pub fn ext_model_route(&self) -> Option<(&str, ApiType)> {
        let endpoint = self
            .chat_endpoint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let api_type = match self.api_type.as_deref().map(str::trim) {
            Some("openai") => ApiType::OpenAiCompatible,
            Some("anthropic") => ApiType::AnthropicCompatible,
            _ => return None,
        };
        Some((endpoint, api_type))
    }
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
    /// W12b: for a KEY-BACKED provider injected by an extension via the `providers.register`
    /// broker verb (a first-party gateway the extension owns), the id of the owning extension.
    /// `None` for every user-authored / native provider (the settings modal, `/free`, first-run
    /// onboarding, the koma-free mint). The ownership tag the host-enforced delete guard checks
    /// (a user cannot delete an ext-managed provider — only uninstall removes it) and the
    /// uninstall purge sweeps by. `skip_serializing_if = "Option::is_none"` keeps a native
    /// provider's on-disk JSON BYTE-IDENTICAL to the pre-W12b shape (the field is simply
    /// absent), and `default` lets an older config without the key load cleanly. Never affects
    /// resolution: `app::resolve::from_entry` keys purely on `provider_uuid`, so an ext-owned
    /// key-backed provider flows through the identical native `config.providers` route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext_id: Option<String>,
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
    /// Provenance: the reverse-DNS id of the extension that registered this server, or
    /// `None` for a user-configured / hand-written one. Added for FUTURE provenance so
    /// [`AppConfig::remove_ext_mcp_servers`] can deregister exactly an extension's rows on
    /// uninstall; a row with no provenance (every row today) is still matched by its
    /// `command` path living under `extensions/<id>/`. `skip_serializing_if` keeps a
    /// user-configured row's on-disk JSON byte-identical to before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext_id: Option<String>,
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

/// W12b: the outcome of [`AppConfig::purge_extension`] — how many catalogue entries an
/// uninstall removed, and whether the removal reset the GLOBAL Main role (so the uninstall
/// handler can surface the "main model reset" toast). Purely a report; the mutation already
/// happened on the `config`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtPurge {
    /// Key-backed `providers` (ext-owned) removed.
    pub providers_removed: usize,
    /// `models` removed because their `provider_uuid` pointed at a dead ext anchor.
    pub models_removed: usize,
    /// UUIDs of the models removed (for consumer rebind → inherit).
    pub model_uuids: Vec<String>,
    /// `oauth_conns` (ext-owned) removed.
    pub conns_removed: usize,
    /// Dead provider/oauth anchor uuids (for scrubbing session_models by provider).
    pub dead_anchors: Vec<String>,
    /// A removed model held the GLOBAL Main role → Main is now unassigned (self-heals to
    /// koma-free at dispatch). The caller toasts the reset.
    pub main_reset: bool,
}

/// Result of cascading a provider / oauth-conn / model removal through the global catalogue.
/// Callers pass [`CascadePurge::models_removed`] into the consumer-rebind helper so agents
/// and session overrides fall back to inherit instead of dangling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CascadePurge {
    /// Model uuids that were dropped from `config.models`.
    pub models_removed: Vec<String>,
    /// A removed model held the GLOBAL Main role.
    pub main_reset: bool,
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
    /// W12b: each extension's PREFERRED (recommended-default) model, keyed by extension id →
    /// the `ModelEntry::uuid` it marked `default: true` in a `models.register` call. Drives two
    /// things: the one-shot VACUUM-FILL of the Main role (when Main is unset / only the keyless
    /// koma-free placeholder, the first extension's preferred model is auto-assigned Main), and
    /// the additive `recommendedBy` hint on the model wire projection so the GUI picker can flag
    /// an extension-recommended model even when Main is already a real user choice. A `BTreeMap`
    /// for deterministic on-disk ordering. `skip_serializing_if = "BTreeMap::is_empty"` keeps a
    /// zero-extension config's JSON BYTE-IDENTICAL (the key is simply absent), and `default`
    /// loads an older config without it cleanly. Cleared for an extension on uninstall.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ext_preferred_models: std::collections::BTreeMap<String, String>,
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
            ext_preferred_models: std::collections::BTreeMap::new(),
        }
    }
}

impl AppConfig {
    /// Load from `~/.koma/config.json`.
    ///
    /// Strips any `clinepass` OAuth conns + orphaned models (whose `provider_uuid`
    /// referenced one of those stripped conns) on load — ClinePass was only usable
    /// with an existing CLI login (no browser authorize/callback) and is now removed.
    /// The migration runs at the JSON level so the enum variant can still deserialize
    /// (it hasn't been deleted yet). If anything was stripped the cleaned config is
    /// persisted; otherwise the file is left untouched.
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
        // --- clinepass migration: strip before enum deserialization ---
        let mut val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        let stripped = Self::strip_clinepass(&mut val);
        let config: AppConfig = serde_json::from_value(val).unwrap_or_default();
        if stripped {
            // Persist the cleaned config so we don't re-strip on every boot.
            // Ignore save errors (best-effort migration; the stripped in-memory
            // config is still valid for the session).
            let _ = config.save();
        }
        config
    }

    /// Remove every OAuth conn whose `provider` field is `"clinepass"` and every
    /// model whose `provider_uuid` matches one of the stripped conn uuids. Operates
    /// on a `serde_json::Value` so it runs BEFORE enum deserialization (the
    /// `ClinePass` variant still exists in the type-level enum — this only strips
    /// it from the JSON document). Returns `true` if anything was removed.
    fn strip_clinepass(doc: &mut serde_json::Value) -> bool {
        let mut stripped_uuids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // 1) Strip clinepass oauth conns; collect their uuids.
        if let Some(conns) = doc.get_mut("oauth_conns").and_then(|c| c.as_array_mut()) {
            let before = conns.len();
            conns.retain(|c| {
                let is_clinepass = c.get("provider").and_then(|v| v.as_str()) == Some("clinepass");
                if is_clinepass {
                    if let Some(uuid) = c.get("uuid").and_then(|v| v.as_str()) {
                        stripped_uuids.insert(uuid.to_string());
                    }
                }
                !is_clinepass
            });
            if conns.len() == before {
                return false; // nothing stripped; no need to check models
            }
        } else {
            return false; // no oauth_conns key at all
        }

        // 2) Strip orphaned models whose provider_uuid referenced a stripped conn.
        if let Some(models) = doc.get_mut("models").and_then(|m| m.as_array_mut()) {
            models.retain(|m| {
                let uuid = m
                    .get("provider_uuid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                !stripped_uuids.contains(uuid)
            });
        }

        true
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

    /// Remove the global model with `uuid` (no-op if none matches). Catalogue-only —
    /// callers that need agents/sessions nudged back to inherit should use
    /// [`Self::cascade_remove_models`] + the app-level consumer rebind.
    #[allow(dead_code)] // public catalogue primitive; cascade path is preferred at call sites
    pub fn remove_model_by_uuid(&mut self, uuid: &str) {
        self.models.retain(|m| m.uuid != uuid);
    }

    /// Drop every model whose uuid is in `dead`. Returns the uuids that actually existed
    /// and whether any held global Main. Catalogue-only; pair with consumer rebind.
    pub fn cascade_remove_models(
        &mut self,
        dead: &std::collections::HashSet<String>,
    ) -> CascadePurge {
        if dead.is_empty() {
            return CascadePurge::default();
        }
        let main_reset = self
            .models
            .iter()
            .any(|m| dead.contains(&m.uuid) && m.effective_roles().contains(&ModelRole::Main));
        let mut models_removed = Vec::new();
        self.models.retain(|m| {
            if dead.contains(&m.uuid) {
                models_removed.push(m.uuid.clone());
                false
            } else {
                true
            }
        });
        CascadePurge {
            models_removed,
            main_reset,
        }
    }

    /// Remove provider `provider_uuid` and every model whose `provider_uuid` points at it.
    /// No-op if the provider is missing. Catalogue-only; pair with consumer rebind.
    pub fn cascade_remove_provider(&mut self, provider_uuid: &str) -> CascadePurge {
        if !self.providers.iter().any(|p| p.uuid == provider_uuid) {
            return CascadePurge::default();
        }
        let mut dead = std::collections::HashSet::new();
        dead.insert(provider_uuid.to_string());
        let purge = self.remove_models_by_providers(&dead);
        self.providers.retain(|p| p.uuid != provider_uuid);
        purge
    }

    /// Remove oauth connection `conn_uuid` and every model whose `provider_uuid` points
    /// at it (models can anchor on oauth uuids). No-op if the conn is missing.
    /// Catalogue-only; pair with consumer rebind.
    pub fn cascade_remove_oauth_conn(&mut self, conn_uuid: &str) -> CascadePurge {
        if !self.oauth_conns.iter().any(|c| c.uuid == conn_uuid) {
            return CascadePurge::default();
        }
        let mut dead = std::collections::HashSet::new();
        dead.insert(conn_uuid.to_string());
        let purge = self.remove_models_by_providers(&dead);
        self.oauth_conns.retain(|c| c.uuid != conn_uuid);
        purge
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
            // A user-authored provider (the GUI Connector form) is never ext-managed.
            ext_id: None,
        });
    }

    /// Remove the provider with `uuid` and cascade-drop every model that pointed at it.
    /// No-op if none matches. Catalogue-only; callers that also need agents/sessions
    /// rewritten to inherit should follow with the app-level consumer rebind helper.
    #[allow(dead_code)]
    pub fn remove_provider_by_uuid(&mut self, uuid: &str) {
        let _ = self.cascade_remove_provider(uuid);
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
        match self
            .installed_extensions
            .iter_mut()
            .find(|e| e.id == ext.id)
        {
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

    /// Deregister every configured MCP server that BELONGS to extension `ext_id` (uninstall
    /// step 5): a row whose `ext_id` provenance matches, OR whose stdio `command` path lives
    /// under `extensions/<ext_id>/` — an extension that bundled its own MCP-server binary,
    /// now a dead orphan once that binary is deleted. Returns the count removed; the caller
    /// persists via [`Self::save`] and triggers the live MCP reload afterwards.
    ///
    /// The path test is a LEXICAL, component-wise prefix ([`std::path::Path::starts_with`])
    /// on the CONFIGURED command string — deliberately NOT canonicalized (the binary may
    /// already be gone, so `canonicalize` would fail), and component-wise so
    /// `extensions/<id>` can never match `extensions/<id>-other`. A blank command, or an
    /// unresolved extensions dir, matches nothing (kept).
    pub fn remove_ext_mcp_servers(&mut self, ext_id: &str) -> usize {
        let ext_prefix = crate::model::store::extensions_dir()
            .ok()
            .map(|d| d.join(ext_id));
        let before = self.mcp_servers.len();
        self.mcp_servers.retain(|s| {
            let by_provenance = s.ext_id.as_deref() == Some(ext_id);
            let by_command_path = match &ext_prefix {
                Some(prefix) => {
                    let cmd = s.command.trim();
                    !cmd.is_empty() && std::path::Path::new(cmd).starts_with(prefix)
                }
                None => false,
            };
            // Keep the row UNLESS it belongs to this extension by either signal.
            !(by_provenance || by_command_path)
        });
        before - self.mcp_servers.len()
    }

    /// The installed extension with `id`, if any. Mirrors the `*_by_uuid` lookups.
    #[allow(dead_code)]
    pub fn extension_by_id(&self, id: &str) -> Option<&InstalledExtension> {
        self.installed_extensions.iter().find(|e| e.id == id)
    }

    /// Remove every model whose `provider_uuid` is in `dead` (orphan prevention when the
    /// serving provider/conn is removed). The SINGLE sweep shared by the
    /// `providers.unregister` broker verb, [`Self::cascade_remove_provider`],
    /// [`Self::cascade_remove_oauth_conn`], and [`Self::purge_extension`], so a removed
    /// anchor never leaves a model pointing at a vanished provider.
    pub(crate) fn remove_models_by_providers(
        &mut self,
        dead: &std::collections::HashSet<String>,
    ) -> CascadePurge {
        if dead.is_empty() {
            return CascadePurge::default();
        }
        let main_reset = self.models.iter().any(|m| {
            dead.contains(&m.provider_uuid) && m.effective_roles().contains(&ModelRole::Main)
        });
        let mut models_removed = Vec::new();
        self.models.retain(|m| {
            if dead.contains(&m.provider_uuid) {
                models_removed.push(m.uuid.clone());
                false
            } else {
                true
            }
        });
        CascadePurge {
            models_removed,
            main_reset,
        }
    }

    /// W12b: purge every trace of extension `ext_id` from the global catalogue, on-loop, as one
    /// PURE mutation (the caller persists via [`Self::save`] afterwards). Removes:
    /// - `providers` whose `ext_id == Some(ext_id)` (W12b key-backed gateways),
    /// - `models` whose `provider_uuid` pointed at ANY of those providers OR at one of this
    ///   extension's `oauth_conns` (orphan prevention — the same [`Self::remove_models_by_providers`]
    ///   sweep `providers.unregister` uses),
    /// - `oauth_conns` whose `ext_id == Some(ext_id)` (W11/W12 delegated tokens),
    /// - the extension's `ext_preferred_models` record.
    ///
    /// Order matters: the dead-anchor set (providers ∪ conns) is captured, and the Main-reset
    /// flag computed, BEFORE any removal — so [`ExtPurge::main_reset`] reports whether a model
    /// that held the GLOBAL Main role referenced a now-dead anchor (the caller surfaces the
    /// "main model reset" toast off it; resolution self-heals to koma-free exactly as it does
    /// for any other dangling Main provider). Returns the counts + that flag. Never touches
    /// per-session `session_models` (those live in `AppState`, not `config`).
    pub fn purge_extension(&mut self, ext_id: &str) -> ExtPurge {
        use std::collections::HashSet;
        // Dead anchors: this extension's key-backed providers ∪ its oauth conns.
        let mut dead: HashSet<String> = self
            .providers
            .iter()
            .filter(|p| p.ext_id.as_deref() == Some(ext_id))
            .map(|p| p.uuid.clone())
            .collect();
        dead.extend(
            self.oauth_conns
                .iter()
                .filter(|c| c.ext_id.as_deref() == Some(ext_id))
                .map(|c| c.uuid.clone()),
        );
        // Model sweep (also computes main_reset from dead anchors).
        let model_purge = self.remove_models_by_providers(&dead);
        let before_providers = self.providers.len();
        self.providers
            .retain(|p| p.ext_id.as_deref() != Some(ext_id));
        let providers_removed = before_providers - self.providers.len();
        let before_conns = self.oauth_conns.len();
        self.oauth_conns
            .retain(|c| c.ext_id.as_deref() != Some(ext_id));
        let conns_removed = before_conns - self.oauth_conns.len();
        self.ext_preferred_models.remove(ext_id);
        ExtPurge {
            providers_removed,
            models_removed: model_purge.models_removed.len(),
            model_uuids: model_purge.models_removed,
            conns_removed,
            dead_anchors: dead.into_iter().collect(),
            main_reset: model_purge.main_reset,
        }
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
            // A migration-seeded provider is native (user's own key).
            ext_id: None,
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
            OAuthProvider::CommandCode,
        ] {
            assert_eq!(OAuthProvider::from_wire_id(p.wire_id()), Some(p));
        }
    }

    /// `"codex_paste"` selects the paste-token input screen, not a real flow-driving
    /// provider — it must resolve to `None` so callers route it to the paste path
    /// instead of mistaking it for (or falling back to) a real provider.
    #[test]
    fn from_wire_id_rejects_paste_variants_and_unknown() {
        assert_eq!(OAuthProvider::from_wire_id("codex_paste"), None);
        assert_eq!(OAuthProvider::from_wire_id("clinepass_paste"), None);
        assert_eq!(OAuthProvider::from_wire_id("commandcode_paste"), None);
        assert_eq!(OAuthProvider::from_wire_id("not_a_provider"), None);
        assert_eq!(OAuthProvider::from_wire_id(""), None);
        // W11: the `extension` storage marker is NOT a from_wire_id input — ext flows
        // route through the `ext:<id>:<provider>` picker id, never a bare token.
        assert_eq!(OAuthProvider::from_wire_id("extension"), None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
        // W12: the four data-driven-resolution fields also default to None on a native
        // conn, so they too are omitted from the on-disk JSON.
        assert!(conn.chat_endpoint.is_none());
        assert!(conn.api_type.is_none());
        assert!(conn.refresh_token_url.is_none());
        assert!(conn.refresh_client_id.is_none());
        // A native conn is never a model provider (no chat_endpoint/api_type meta).
        assert!(conn.ext_model_route().is_none());
        let reser = serde_json::to_string(&conn).expect("serializes");
        assert_eq!(
            reser, NATIVE_CONN_JSON,
            "a native OAuthConn must round-trip byte-identically after W11/W12"
        );
    }

    /// An EXT-backed conn serializes with the `"extension"` provider tag plus the two
    /// ext fields and the W12 model-provider meta, and round-trips back to an equal value.
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
            // W12 model-provider meta.
            chat_endpoint: Some("https://api.demo.test/v1".to_string()),
            api_type: Some("openai".to_string()),
            refresh_token_url: Some("https://demo.test/token".to_string()),
            refresh_client_id: Some("cid".to_string()),
            ..Default::default()
        };
        let v = serde_json::to_value(&conn).expect("serializes");
        assert_eq!(v["provider"], "extension");
        assert_eq!(v["ext_id"], "run.koma.example.oauth-demo-daemon");
        assert_eq!(v["provider_id"], "demo");
        assert_eq!(v["chat_endpoint"], "https://api.demo.test/v1");
        assert_eq!(v["api_type"], "openai");
        assert_eq!(v["refresh_token_url"], "https://demo.test/token");
        assert_eq!(v["refresh_client_id"], "cid");

        let back: OAuthConn = serde_json::from_value(v).expect("ext conn roundtrips");
        assert_eq!(back.provider, OAuthProvider::Extension);
        assert_eq!(
            back.ext_id.as_deref(),
            Some("run.koma.example.oauth-demo-daemon")
        );
        assert_eq!(back.provider_id.as_deref(), Some("demo"));
        assert_eq!(back.access_token, "demo-at");
        assert_eq!(
            back.chat_endpoint.as_deref(),
            Some("https://api.demo.test/v1")
        );
        assert_eq!(back.api_type.as_deref(), Some("openai"));
        assert_eq!(
            back.refresh_token_url.as_deref(),
            Some("https://demo.test/token")
        );
    }

    /// [`OAuthConn::ext_model_route`] accepts a conn with both a chat endpoint and a
    /// recognised api_type, mapping the wire string to the dispatch [`ApiType`]; it rejects
    /// a conn missing either half (account-login-only) or carrying an unrecognised api_type.
    #[test]
    fn ext_model_route_gates_on_endpoint_and_api_type() {
        use super::ApiType;
        let with = |endpoint: Option<&str>, api_type: Option<&str>| OAuthConn {
            provider: OAuthProvider::Extension,
            chat_endpoint: endpoint.map(str::to_string),
            api_type: api_type.map(str::to_string),
            ..Default::default()
        };
        // openai → OpenAiCompatible.
        assert_eq!(
            with(Some("https://x.test/v1"), Some("openai")).ext_model_route(),
            Some(("https://x.test/v1", ApiType::OpenAiCompatible))
        );
        // anthropic → AnthropicCompatible.
        assert_eq!(
            with(Some("https://x.test"), Some("anthropic")).ext_model_route(),
            Some(("https://x.test", ApiType::AnthropicCompatible))
        );
        // Missing endpoint, missing api_type, or unrecognised api_type → account-login-only.
        assert!(with(None, Some("openai")).ext_model_route().is_none());
        assert!(with(Some("https://x.test"), None)
            .ext_model_route()
            .is_none());
        assert!(with(Some("   "), Some("openai"))
            .ext_model_route()
            .is_none());
        assert!(with(Some("https://x.test"), Some("openai_compatible"))
            .ext_model_route()
            .is_none());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod provider_conn_serde_tests {
    use super::{ApiType, ProviderConn};

    /// A NATIVE ProviderConn exactly as pre-W12b `config.json` wrote it — no `ext_id` key,
    /// field order matching the struct declaration (which is the order `serde_json` emits).
    const NATIVE_PROVIDER_JSON: &str = concat!(
        r#"{"uuid":"33333333-3333-3333-3333-333333333333","name":"OpenRouter","#,
        r#""api_type":"open_ai_compatible","endpoint":"https://openrouter.ai/api/v1","#,
        r#""api_key":"sk-abc"}"#
    );

    /// SERDE-COMPAT PROOF: a pre-W12b native provider deserializes cleanly (the new `ext_id`
    /// field defaults to `None`) AND re-serializes BYTE-IDENTICALLY — `skip_serializing_if =
    /// "Option::is_none"` means a native provider never emits the key. Existing `config.json`
    /// files round-trip unchanged (the W11/W12 `OAuthConn` discipline, applied to providers).
    #[test]
    fn native_provider_roundtrips_byte_stable() {
        let p: ProviderConn =
            serde_json::from_str(NATIVE_PROVIDER_JSON).expect("pre-W12b provider parses");
        assert_eq!(p.api_type, ApiType::OpenAiCompatible);
        assert!(p.ext_id.is_none(), "a native provider carries no ext_id");
        let reser = serde_json::to_string(&p).expect("serializes");
        assert_eq!(
            reser, NATIVE_PROVIDER_JSON,
            "a native ProviderConn must round-trip byte-identically after W12b"
        );
    }

    /// An EXT-owned (key-backed) provider serializes WITH the `ext_id` key and round-trips.
    #[test]
    fn ext_provider_roundtrips() {
        let p = ProviderConn {
            uuid: "44444444-4444-4444-4444-444444444444".to_string(),
            name: "Demo Gateway".to_string(),
            api_type: ApiType::AnthropicCompatible,
            endpoint: "https://api.demo.test/v1".to_string(),
            api_key: "demo-key".to_string(),
            ext_id: Some("run.koma.example.gateway".to_string()),
        };
        let v = serde_json::to_value(&p).expect("serializes");
        assert_eq!(v["ext_id"], "run.koma.example.gateway");
        let back: ProviderConn = serde_json::from_value(v).expect("ext provider roundtrips");
        assert_eq!(back.ext_id.as_deref(), Some("run.koma.example.gateway"));
        assert_eq!(back.api_type, ApiType::AnthropicCompatible);
        assert_eq!(back, p);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod ext_purge_tests {
    use super::*;
    use std::collections::HashSet;

    /// A key-backed ext provider owned by `ext_id`.
    fn ext_provider(uuid: &str, ext_id: &str) -> ProviderConn {
        ProviderConn {
            uuid: uuid.to_string(),
            name: "gw".to_string(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "https://gw.test/v1".to_string(),
            api_key: "k".to_string(),
            ext_id: Some(ext_id.to_string()),
        }
    }

    fn native_provider(uuid: &str) -> ProviderConn {
        ProviderConn {
            uuid: uuid.to_string(),
            name: uuid.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn cascade_remove_provider_drops_matching_models_and_flags_main() {
        let mut config = AppConfig::default();
        config.providers.push(native_provider("p1"));
        config.providers.push(native_provider("p2"));
        config.models.push(ModelEntry {
            uuid: "m-main".into(),
            provider_uuid: "p1".into(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-other".into(),
            provider_uuid: "p2".into(),
            ..Default::default()
        });
        let report = config.cascade_remove_provider("p1");
        assert_eq!(report.models_removed, vec!["m-main".to_string()]);
        assert!(report.main_reset);
        assert!(config.providers.iter().all(|p| p.uuid != "p1"));
        assert!(config.models.iter().all(|m| m.uuid != "m-main"));
        assert!(config.models.iter().any(|m| m.uuid == "m-other"));
        // Missing provider is a no-op.
        let empty = config.cascade_remove_provider("nope");
        assert!(empty.models_removed.is_empty());
        assert!(!empty.main_reset);
    }

    #[test]
    fn cascade_remove_oauth_conn_drops_matching_models() {
        let mut config = AppConfig::default();
        config.oauth_conns.push(OAuthConn {
            uuid: "oauth-1".into(),
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-oauth".into(),
            provider_uuid: "oauth-1".into(),
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-keep".into(),
            provider_uuid: "other".into(),
            ..Default::default()
        });
        let report = config.cascade_remove_oauth_conn("oauth-1");
        assert_eq!(report.models_removed, vec!["m-oauth".to_string()]);
        assert!(!report.main_reset);
        assert!(config.oauth_conns.is_empty());
        assert!(config.models.iter().all(|m| m.uuid != "m-oauth"));
        assert!(config.models.iter().any(|m| m.uuid == "m-keep"));
    }

    #[test]
    fn cascade_remove_models_drops_by_uuid_and_flags_main() {
        let mut config = AppConfig::default();
        config.models.push(ModelEntry {
            uuid: "m1".into(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m2".into(),
            ..Default::default()
        });
        let mut dead = HashSet::new();
        dead.insert("m1".into());
        dead.insert("missing".into());
        let report = config.cascade_remove_models(&dead);
        assert_eq!(report.models_removed, vec!["m1".to_string()]);
        assert!(report.main_reset);
        assert!(config.models.iter().all(|m| m.uuid != "m1"));
        assert!(config.models.iter().any(|m| m.uuid == "m2"));
    }

    #[test]
    fn remove_provider_by_uuid_cascades_models() {
        let mut config = AppConfig::default();
        config.providers.push(native_provider("p1"));
        config.models.push(ModelEntry {
            uuid: "m1".into(),
            provider_uuid: "p1".into(),
            ..Default::default()
        });
        config.remove_provider_by_uuid("p1");
        assert!(config.providers.is_empty());
        assert!(config.models.is_empty(), "thin wrapper must cascade");
    }

    /// `purge_extension` removes the extension's providers + oauth conns + orphaned models +
    /// preferred record, leaves EVERY other owner's entries untouched, and reports `main_reset`
    /// only when a removed model held the global Main role.
    #[test]
    fn purge_removes_ext_entries_and_reports_main_reset() {
        let mut config = AppConfig::default();
        // ext A: one key-backed provider + one oauth conn, two models (one holds Main).
        config.providers.push(ext_provider("prov-a", "ext.a"));
        config.oauth_conns.push(OAuthConn {
            uuid: "conn-a".to_string(),
            provider: OAuthProvider::Extension,
            ext_id: Some("ext.a".to_string()),
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-a-main".to_string(),
            model_id: "big".to_string(),
            provider_uuid: "prov-a".to_string(),
            roles: vec![ModelRole::Main],
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-a-conn".to_string(),
            model_id: "small".to_string(),
            provider_uuid: "conn-a".to_string(),
            ..Default::default()
        });
        config
            .ext_preferred_models
            .insert("ext.a".to_string(), "m-a-main".to_string());
        // ext B + a native provider/model that must SURVIVE the purge of A.
        config.providers.push(ext_provider("prov-b", "ext.b"));
        config.providers.push(ProviderConn {
            uuid: "prov-native".to_string(),
            name: "native".to_string(),
            ..Default::default()
        });
        config.models.push(ModelEntry {
            uuid: "m-native".to_string(),
            provider_uuid: "prov-native".to_string(),
            ..Default::default()
        });
        config
            .ext_preferred_models
            .insert("ext.b".to_string(), "m-b".to_string());

        let report = config.purge_extension("ext.a");
        assert_eq!(report.providers_removed, 1);
        assert_eq!(report.conns_removed, 1);
        assert_eq!(
            report.models_removed, 2,
            "both of A's models (provider + conn backed) are swept"
        );
        assert!(
            report.main_reset,
            "a removed model held the global Main role"
        );

        // A is gone; B + native survive.
        assert!(config.providers.iter().all(|p| p.uuid != "prov-a"));
        assert!(config.oauth_conns.is_empty());
        assert!(config
            .models
            .iter()
            .all(|m| m.provider_uuid != "prov-a" && m.provider_uuid != "conn-a"));
        assert!(
            config.providers.iter().any(|p| p.uuid == "prov-b"),
            "another extension is untouched"
        );
        assert!(
            config.providers.iter().any(|p| p.uuid == "prov-native"),
            "a native provider is untouched"
        );
        assert!(
            config.models.iter().any(|m| m.uuid == "m-native"),
            "a native model is untouched"
        );
        assert_eq!(
            config.ext_preferred_models.get("ext.a"),
            None,
            "A's preferred record is cleared"
        );
        assert_eq!(
            config.ext_preferred_models.get("ext.b").map(String::as_str),
            Some("m-b"),
            "another extension's preferred record is untouched"
        );
    }

    /// Purging an extension whose models hold NO runtime role leaves `main_reset` false.
    #[test]
    fn purge_without_main_holder_does_not_flag_reset() {
        let mut config = AppConfig::default();
        config.providers.push(ext_provider("prov-a", "ext.a"));
        config.models.push(ModelEntry {
            uuid: "m1".to_string(),
            provider_uuid: "prov-a".to_string(),
            ..Default::default()
        });
        let report = config.purge_extension("ext.a");
        assert_eq!(report.providers_removed, 1);
        assert_eq!(report.models_removed, 1);
        assert!(!report.main_reset, "no removed model held Main");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod clinepass_migration_tests {
    use super::AppConfig;

    /// `strip_clinepass` removes a clinepass conn + orphaned model, leaves
    /// a codex conn + its model untouched.
    #[test]
    fn strips_clinepass_and_orphan_models() {
        let json = serde_json::json!({
            "palette": "tokyo-night",
            "oauth_conns": [
                {
                    "uuid": "c1",
                    "provider": "clinepass",
                    "access_token": "wp:dead",
                    "name": "ClinePass account",
                    "email": "u@cline.bot"
                },
                {
                    "uuid": "c2",
                    "provider": "codex",
                    "access_token": "good",
                    "name": "Codex account",
                    "email": "u@codex.io"
                }
            ],
            "models": [
                {
                    "uuid": "m1",
                    "model_id": "gpt-4",
                    "provider_uuid": "c1",
                    "roles": ["Main"]
                },
                {
                    "uuid": "m2",
                    "model_id": "codex-default",
                    "provider_uuid": "c2",
                    "roles": ["Main"]
                }
            ]
        });

        let mut val = serde_json::to_value(&json).unwrap();
        let stripped = AppConfig::strip_clinepass(&mut val);
        assert!(stripped, "should report a strip");

        let conns = val["oauth_conns"].as_array().unwrap();
        assert_eq!(conns.len(), 1, "only codex conn survives");
        assert_eq!(conns[0]["provider"], "codex");

        let models = val["models"].as_array().unwrap();
        assert_eq!(models.len(), 1, "only the codex model survives");
        assert_eq!(models[0]["provider_uuid"], "c2");
    }

    /// When there are no clinepass conns, `strip_clinepass` returns false
    /// and the JSON is unchanged (no model sweep runs).
    #[test]
    fn no_clinepass_no_strip() {
        let json = serde_json::json!({
            "oauth_conns": [
                {
                    "uuid": "c1",
                    "provider": "codex",
                    "access_token": "good"
                }
            ],
            "models": [
                {
                    "uuid": "m1",
                    "provider_uuid": "c1"
                }
            ]
        });

        let mut val = serde_json::to_value(&json).unwrap();
        let stripped = AppConfig::strip_clinepass(&mut val);
        assert!(!stripped, "no clinepass → no strip");
        assert_eq!(val["oauth_conns"].as_array().unwrap().len(), 1);
        assert_eq!(val["models"].as_array().unwrap().len(), 1);
    }

    /// A JSON document with no `oauth_conns` key at all is a no-op.
    #[test]
    fn no_oauth_conns_key() {
        let json = serde_json::json!({
            "palette": "tokyo-night",
            "models": []
        });
        let mut val = serde_json::to_value(&json).unwrap();
        let stripped = AppConfig::strip_clinepass(&mut val);
        assert!(!stripped);
    }
}

// W13: additional regression suite — pure addition, sibling file, never touches any module
// above.
#[cfg(test)]
#[path = "app_config_test.rs"]
mod app_config_test;
