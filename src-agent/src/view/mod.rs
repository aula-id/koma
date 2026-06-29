//! View layer — render dispatcher ("V" in MVC).
//!
//! The single entry-point [`draw`] is called once per event-loop tick by the
//! runtime after state has been updated.  It inspects the current [`Mode`] and
//! forwards to the appropriate module:
//!
//! - [`chat`]           – the main conversation view (messages + input bar)
//! - [`key_input`]      – the first-run / reconfigure credentials form
//! - [`session_picker`] – the `--resume` session list with search bar
//! - [`settings`]       – the in-app `/settings` dashboard
//! - [`effort`]         – the `/effort` reasoning-effort picker overlay
//!
//! No logic lives here; all rendering decisions belong to the sub-modules.

pub mod agents;
pub mod chat;
pub mod mcp;
pub mod effort;
pub mod key_input;
pub mod loading;
pub mod message_rewind;
pub mod markdown;
pub mod quit_confirm;
pub mod session_hub;
pub mod session_picker;
pub mod settings;
pub mod theme;
pub mod usage;

use ratatui::Frame;
use crate::app::mode::Mode;
use crate::app::resolve::{resolve_role};
use crate::app::state::AppState;
use crate::model::app_config::ModelRole;

/// Render the entire terminal frame for the current application state.
///
/// Called by the runtime on every UI refresh tick.  Delegates to the
/// mode-specific draw function; only one mode is active at a time.
///
/// The palette is computed once here and passed to every sub-draw so all
/// colour decisions flow through a single source of truth.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let palette = theme::palette(&state.rest.config);
    // The catalogue is now per-endpoint and fetched on demand: pass BOTH the
    // cached models and the endpoint they were fetched for, so each omnisearch view
    // can tell "this is my provider's catalogue" (filter locally) from "still
    // fetching / stale" (show `searching models…`) and "fetched but empty"
    // (`no models — type an id`).
    let cache = state.rest.models_cache.as_deref().unwrap_or(&[]);
    let cache_endpoint = state.rest.models_cache_endpoint.as_deref();
    match &state.mode {
        Mode::Chat => {
            // Resolve the actual Main model that will be used for chat requests.
            // Session overrides win over the global catalogue; falls back to
            // settings.model (the legacy field) when nothing is configured.
            let resolved_model: String = state.rest.fg().session.as_ref()
                .and_then(|s| resolve_role(&state.rest.config, &s.settings, ModelRole::Main))
                .map(|r| r.model_id)
                .or_else(|| state.rest.fg().session.as_ref().map(|s| s.settings.model.clone()))
                .unwrap_or_default();
            chat::draw(frame, &state.rest, &resolved_model, &palette);
        }
        Mode::KeyInput(form) => key_input::draw(frame, form, cache, cache_endpoint, &palette),
        Mode::SessionPicker(p) => session_picker::draw(frame, p, &palette),
        Mode::SessionHub(h) => session_hub::draw(frame, h, &palette),
        Mode::Settings(s) => settings::draw(frame, s, cache, cache_endpoint, &palette),
        Mode::Agents(a) => agents::draw(
            frame,
            a,
            &state.rest.config,
            state.rest.fg().session.as_ref().map(|s| &s.settings),
            &palette,
        ),
        Mode::Mcp(m) => {
            // Live per-server tool counts from the MCP manager snapshot (owned map
            // so the manager lock isn't held across the draw); `None` when no
            // manager exists. Feeds the LIST + detail status display.
            let status = state.rest.mcp_manager.as_ref().map(|mgr| mgr.server_status());
            mcp::draw(frame, m, status.as_ref(), &palette);
        }
        Mode::Effort(e) => effort::draw(frame, e, &palette),
        Mode::Loading(s) => loading::draw(frame, s, &palette),
        Mode::Usage(nav) => {
            // The dashboard renders from a pre-fetched ledger projection so the SAME
            // draw path serves the local TUI and the daemon's thin client. The client
            // receives the projection in the snapshot (`rest.usage_data`); a local TUI
            // leaves that `None` and collects it live from the ledger here every frame
            // (unchanged behaviour). See `model::usage::UsageData`.
            let data = state.rest.usage_data.clone().unwrap_or_else(|| {
                usage::collect_usage_data(nav, &state.rest)
            });
            usage::draw(frame, &state.rest, nav, &data, &palette);
        }
        Mode::MessageRewind(rw) => message_rewind::draw(frame, rw, &palette),
        Mode::QuitConfirm(s) => quit_confirm::draw(frame, s, &palette),
    }
}
