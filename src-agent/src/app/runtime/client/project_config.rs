//! Config projection for the GUI host-relay bridge (the `Config` envelope both
//! host states push) — split out of `project.rs` for file size (pure code
//! motion, no behaviour change).
//!
//! [`ConfigProjection`] snapshots the daemon's authoritative config
//! (providers/models/mcp/palette) off either an attached [`crate::ipc::proto::GlobalSnapshot`]
//! or a directly-loaded [`crate::model::app_config::AppConfig`] (the swapper's
//! pre-attach path); [`push_config`] serialises it into a [`PushEnvelope::Config`]
//! and dedups on `last.config_json`.
//!
//! `color_hex` moved HERE (not `push_proto.rs`): its only callers
//! (`push_palette_from_config`, `push_config`) both live in this file — the
//! minimal-bump placement. `push_palette_from_config` is bumped to `pub(super)`
//! (not private) because `project.rs`'s `serialize_and_push` — a SIBLING module
//! now, after this split — calls it too (the chat Snapshot's palette uses the
//! SAME resolved colours as the swapper Config palette).

use super::push_loop::PushState;
use super::push_proto::{
    PushEnvelope, PushMcpServer, PushModel, PushPalette, PushPaletteInfo, PushProvider,
};

/// Resolve a ratatui [`Color`] to a `#rrggbb` string, mirroring the fallbacks the
/// GUI host uses elsewhere (near-black bg, near-white fg for non-Rgb palettes).
fn color_hex(c: ratatui::style::Color, fallback: &str) -> String {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_string(),
    }
}

/// The GUI-relevant slice of the daemon's authoritative config, cached by
/// [`push_loop`] from each incoming full [`crate::ipc::proto::StateSnapshot`] so the
/// `Config` envelope can be (re)built + diffed independently of the frame stream — e.g.
/// re-emitted on a `Ready` reload without waiting for the next snapshot. Mirrors the
/// four `GlobalSnapshot` config fields: `models` is the GLOBAL scope, `session_models`
/// the foreground session's LOCAL override scope.
pub(super) struct ConfigProjection {
    providers: Vec<crate::model::app_config::ProviderConn>,
    models: Vec<crate::model::app_config::ModelEntry>,
    session_models: Vec<crate::model::app_config::ModelEntry>,
    mcp_servers: Vec<crate::model::app_config::McpServerEntry>,
    oauth_conn_uuids: Vec<String>,
    palette: PushPalette,
    palette_name: String,
    /// Live per-server tool counts from `McpManager::server_status_cached()`.
    mcp_status: std::collections::HashMap<String, usize>,
    /// Per-server error strings from `McpManager::server_errors()`.
    mcp_errors: std::collections::HashMap<String, String>,
    /// W12b: model_uuid → the extension id that flagged it its recommended `default`
    /// (inverted from `AppConfig::ext_preferred_models`). Drives the additive `recommendedBy`
    /// hint on the model wire rows. Populated on the `from_app_config` (swapper) path which
    /// holds the full config; empty on the attached `from_global` path (the `GlobalSnapshot`
    /// does not yet carry it — deferred with the React picker-badge rendering).
    ext_recommended: std::collections::HashMap<String, String>,
}

impl ConfigProjection {
    /// Snapshot the config slice off a [`crate::ipc::proto::GlobalSnapshot`].
    pub(super) fn from_global(g: &crate::ipc::proto::GlobalSnapshot) -> Self {
        Self {
            providers: g.providers.clone(),
            models: g.config_models.clone(),
            session_models: g.session_models.clone(),
            mcp_servers: g.mcp_servers.clone(),
            oauth_conn_uuids: g.oauth_conn_uuids.clone(),
            palette: palette_from_global(g),
            palette_name: g.palette.clone(),
            mcp_status: std::collections::HashMap::new(),
            mcp_errors: std::collections::HashMap::new(),
            // Attached path: the GlobalSnapshot doesn't carry ext_preferred_models yet
            // (deferred with the picker-badge UI); no recommend hints on this path.
            ext_recommended: std::collections::HashMap::new(),
        }
    }

    /// Snapshot the config slice directly off an in-memory [`AppConfig`].
    ///
    /// Used by the GUI SWAPPER (`host_swapper`), which holds no daemon snapshot to
    /// source config from — it reads the loaded global config directly so the Connector
    /// shows the real providers/models/mcp on FIRST open. `session_models` (the per-
    /// session LOCAL override scope) is empty here: the swapper has no foreground session.
    pub(super) fn from_app_config(cfg: &crate::model::app_config::AppConfig) -> Self {
        Self {
            providers: cfg.providers.clone(),
            models: cfg.models.clone(),
            session_models: Vec::new(),
            mcp_servers: cfg.mcp_servers.clone(),
            oauth_conn_uuids: cfg.oauth_conns.iter().map(|c| c.uuid.clone()).collect(),
            palette: push_palette_from_config(cfg),
            palette_name: cfg.palette.clone(),
            mcp_status: std::collections::HashMap::new(),
            mcp_errors: std::collections::HashMap::new(),
            // Invert ext_id → model_uuid into model_uuid → ext_id for the recommendedBy hint.
            ext_recommended: cfg
                .ext_preferred_models
                .iter()
                .map(|(ext, model)| (model.clone(), ext.clone()))
                .collect(),
        }
    }

    /// Inject live MCP server status (tool counts + error strings) from the
    /// `McpManager`. Called by the push loop after each `from_global` to
    /// overlay runtime connection state onto the config projection.
    pub(super) fn set_mcp_status(
        &mut self,
        status: std::collections::HashMap<String, usize>,
        errors: std::collections::HashMap<String, String>,
    ) {
        self.mcp_status = status;
        self.mcp_errors = errors;
    }
}

/// Build a [`PushPalette`] (the React chat/chrome palette roles) from an
/// [`crate::model::app_config::AppConfig`], resolving the TUI [`crate::view::theme::Palette`]
/// so a non-default theme's colours (bg/fg/accent/dim/panel) are all correct. Fallbacks
/// mirror the dark palette. Shared by the Snapshot palette + the swapper Config palette.
pub(super) fn push_palette_from_config(cfg: &crate::model::app_config::AppConfig) -> PushPalette {
    let pal = crate::view::theme::palette(cfg);
    PushPalette {
        bg: color_hex(pal.bg, "#000000"),
        fg: color_hex(pal.fg, "#c8d3f5"),
        accent: color_hex(pal.accent, "#39ff14"),
        dim: color_hex(pal.dim, "#adadad"),
        panel: color_hex(pal.panel, "#2b2f38"),
        warn: color_hex(pal.warn, "#ffb43c"),
        success: color_hex(pal.success, "#00c853"),
        info: color_hex(pal.info, "#50c8ff"),
        error: color_hex(pal.error, "#ff3c3c"),
    }
}

/// Rebuild a [`PushPalette`] from a [`crate::ipc::proto::GlobalSnapshot`] (the ATTACHED
/// path's Config source). The renderer's palette selection lives entirely in the
/// `palette`-registry NAME (theme/accent are deprecated and unread — see `AppConfig`), so a
/// minimal [`crate::model::app_config::AppConfig`] carrying just that name resolves to the
/// exact same [`crate::view::theme::Palette`] the attached Snapshot pushes.
fn palette_from_global(g: &crate::ipc::proto::GlobalSnapshot) -> PushPalette {
    let cfg = crate::model::app_config::AppConfig {
        palette: g.palette.clone(),
        ..Default::default()
    };
    push_palette_from_config(&cfg)
}

/// Map a persisted [`crate::model::app_config::ModelRole`] to its lowercase wire token
/// (matches the React role tokens + the config serde form).
fn role_token(r: crate::model::app_config::ModelRole) -> &'static str {
    use crate::model::app_config::ModelRole;
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
    }
}

/// Build one [`PushModel`] from a persisted [`crate::model::app_config::ModelEntry`],
/// tagged with its `scope` (`"global"` / `"local"`). Roles fold in the legacy single-
/// role field via `effective_roles`.
fn push_model(m: &crate::model::app_config::ModelEntry, scope: &'static str) -> PushModel {
    PushModel {
        id: m.uuid.clone(),
        name: m.name.clone(),
        model_id: m.model_id.clone(),
        provider: m.provider_uuid.clone(),
        route: m.route.clone().unwrap_or_default(),
        roles: m.effective_roles().into_iter().map(role_token).collect(),
        scope,
        free: false,
        // A local override remembers the global uuid it was cloned from (set by
        // `set_session_main`); `None` on every global entry.
        source_uuid: m.source_uuid.clone(),
        // Set by `push_config` from the projection's ext_recommended map (None by default).
        recommended_by: None,
    }
}

/// Build the SYNTHETIC "advertised free" [`PushModel`] the host prepends to the model
/// quick-picker (wave-3+4 free-pin): the keyless koma-free tier as a special top row.
///
/// Its `id` is the opaque [`crate::service::koma_free::KOMA_FREE_SENTINEL`] (NOT a real
/// `ModelEntry` uuid) so a pick round-trips as `SetSessionMain { model_uuid:
/// Some(sentinel) }` and routes through the `/free` find-or-create flow. `provider` is
/// bound to an EXISTING koma-free `ProviderConn` uuid when one is already provisioned (so
/// React's `modelId`+`provider` active-match lights the checkmark after a free pick),
/// else empty (it is minted lazily on first selection). `scope:"global"` + `free:true`.
fn koma_free_synthetic_model(providers: &[crate::model::app_config::ProviderConn]) -> PushModel {
    let provider = providers
        .iter()
        .find(|p| p.api_type == crate::model::app_config::ApiType::KomaFree)
        .map(|p| p.uuid.clone())
        .unwrap_or_default();
    PushModel {
        id: crate::service::koma_free::KOMA_FREE_SENTINEL.to_string(),
        name: "koma free".to_string(),
        model_id: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        provider,
        route: String::new(),
        roles: vec!["main"],
        scope: "global",
        free: true,
        // The synthetic free row is not a clone of any global — no source identity.
        source_uuid: None,
        // The synthetic free row is never an extension recommendation.
        recommended_by: None,
    }
}

/// Serialise `cfg` into a [`PushEnvelope::Config`] and push it if it changed since the
/// last call. Called every frame from [`push_loop`]; `last.config_json` dedups so an
/// unchanged catalogue is silent, and a `Ready` reset re-emits the full current config.
/// A `None` projection (no snapshot seen yet) is a no-op.
pub(super) fn push_config(cfg: Option<&ConfigProjection>, push: &dyn Fn(String), last: &mut PushState) {
    let Some(cfg) = cfg else { return };
    use crate::model::app_config::McpTransport;

    let providers: Vec<PushProvider> = cfg
        .providers
        .iter()
        .map(|p| PushProvider {
            id: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
            has_key: !p.api_key.is_empty(),
            is_koma_free: p.api_type == crate::model::app_config::ApiType::KomaFree,
        })
        .collect();

    // Resolve the (at most one) koma-free-backed provider so real minted entries
    // (global via `ensure_koma_free_config`, or local via `/free`) can be told apart
    // from an ordinary model that merely happens to share the "koma free" display name.
    let koma_free_provider_uuid: Option<&str> = cfg
        .providers
        .iter()
        .find(|p| p.api_type == crate::model::app_config::ApiType::KomaFree)
        .map(|p| p.uuid.as_str());
    let is_koma_free_backed = |m: &crate::model::app_config::ModelEntry| {
        koma_free_provider_uuid.is_some_and(|uuid| m.provider_uuid == uuid)
    };
    let has_real_koma_free_entry = cfg.models.iter().any(is_koma_free_backed)
        || cfg.session_models.iter().any(is_koma_free_backed);

    // Invariant: the synthetic "advertised free" row is a placeholder for the
    // not-yet-minted state ONLY — once a real koma-free-backed entry exists (global or
    // local), it supersedes the synthetic row instead of duplicating it; that real entry
    // gets `free:true` so the FREE badge moves onto it. (React re-sorts `free` to the top
    // regardless, but ordering the synthetic row first here keeps the raw list honest.)
    let mut models: Vec<PushModel> = if has_real_koma_free_entry {
        Vec::new()
    } else {
        vec![koma_free_synthetic_model(&cfg.providers)]
    };
    models.extend(cfg.models.iter().map(|m| {
        let mut pm = push_model(m, "global");
        if is_koma_free_backed(m) {
            pm.free = true;
        }
        // W12b: additive picker hint — which extension recommends this model as its default.
        pm.recommended_by = cfg.ext_recommended.get(&m.uuid).cloned();
        pm
    }));
    models.extend(cfg.session_models.iter().map(|m| {
        let mut pm = push_model(m, "local");
        if is_koma_free_backed(m) {
            pm.free = true;
        }
        pm.recommended_by = cfg.ext_recommended.get(&m.uuid).cloned();
        pm
    }));

    // The current session Main override (the quick-picker's selected value): the local
    // entry that holds the Main role, if any (else `null` = inherit the global Main).
    let session_main_uuid = cfg
        .session_models
        .iter()
        .find(|m| {
            m.effective_roles()
                .contains(&crate::model::app_config::ModelRole::Main)
        })
        .map(|m| m.uuid.clone());

    let mcp: Vec<PushMcpServer> = cfg
        .mcp_servers
        .iter()
        .map(|s| PushMcpServer {
            id: s.uuid.clone(),
            name: s.name.clone(),
            enabled: s.enabled,
            transport: match s.transport {
                McpTransport::Stdio => "stdio",
                McpTransport::Http => "http",
            },
            command: s.command.clone(),
            args: s.args.join(" "),
            env: s
                .env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", "),
            url: s.url.clone(),
            tool_count: cfg.mcp_status.get(&s.uuid).copied().unwrap_or(0),
            error: cfg.mcp_errors.get(&s.uuid).cloned(),
        })
        .collect();

    // FIRST-RUN: no usable Main route = a global OR local model that (a) holds the Main
    // role AND (b) is bound to a provider OR an OAuth connection that actually exists.
    // An empty config (no providers/oauth_conns, or a Main model whose backing entry was
    // deleted) → onboarding. This is the projection-level proxy for the daemon's
    // `resolve_role(Main).is_usable()` gate (which needs a `Settings` this config-only
    // projection doesn't carry) — mirroring `resolve.rs`'s `from_entry`, which falls back
    // to `config.oauth_conns` when `config.providers` has no match (Claude/Codex/xAI
    // login-backed Main models).
    let has_usable_main = cfg
        .models
        .iter()
        .chain(cfg.session_models.iter())
        .any(|m| {
            m.effective_roles()
                .contains(&crate::model::app_config::ModelRole::Main)
                && (cfg.providers.iter().any(|p| p.uuid == m.provider_uuid)
                    || cfg.oauth_conn_uuids.iter().any(|u| *u == m.provider_uuid))
        });
    let needs_onboarding = !has_usable_main;

    // Available theme registry keys for the onboarding theme step + Settings picker.
    let themes: Vec<&'static str> = crate::view::theme::PALETTES
        .iter()
        .map(|(name, _)| *name)
        .collect();

    // Full palette catalogue WITH resolved colours for the Settings Appearance grid: call
    // each registry constructor and flatten its 11 role colours to `#rrggbb` in the fixed
    // order the GUI paints its movie-strip cards from — reusing the SAME `color_hex`
    // conversion + fallbacks `push_palette_from_config` uses for the chat palette.
    let palettes: Vec<PushPaletteInfo> = crate::view::theme::PALETTES
        .iter()
        .map(|(name, build)| {
            let p = build();
            PushPaletteInfo {
                name: (*name).to_string(),
                colors: vec![
                    color_hex(p.bg, "#000000"),
                    color_hex(p.fg, "#c8d3f5"),
                    color_hex(p.dim, "#adadad"),
                    color_hex(p.accent, "#39ff14"),
                    color_hex(p.panel, "#2b2f38"),
                    color_hex(p.sel_bg, "#39ff14"),
                    color_hex(p.sel_fg, "#000000"),
                    color_hex(p.success, "#00c853"),
                    color_hex(p.warn, "#ffb43c"),
                    color_hex(p.error, "#ff3c3c"),
                    color_hex(p.info, "#50c8ff"),
                ],
            }
        })
        .collect();

    let env = PushEnvelope::Config {
        providers,
        models,
        mcp,
        palette: cfg.palette.clone(),
        session_main_uuid,
        themes,
        palettes,
        theme: cfg.palette_name.clone(),
        needs_onboarding,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        if last.config_json.as_deref() != Some(json.as_str()) {
            last.config_json = Some(json.clone());
            push(json);
        }
    }
}
