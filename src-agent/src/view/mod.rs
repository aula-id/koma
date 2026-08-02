//! View layer — render dispatcher ("V" in MVC).
//!
//! The single entry-point [`draw`] is called once per event-loop tick by the
//! runtime after state has been updated.  It inspects the current [`Mode`] and
//! forwards to the appropriate module:
//!
//! - [`chat`]           – the main conversation view (messages + input bar)
//! - [`key_input`]      – the first-run / reconfigure credentials form
//! - [`session_picker`] – the `--resume` session list with search bar
//! - [`settings`]       – the in-app `/settings` overlay
//! - [`effort`]         – the `/effort` reasoning-effort picker overlay
//!
//! No logic lives here; all rendering decisions belong to the sub-modules.

pub mod agents;
pub mod bash;
pub mod chat;
pub mod effort;
pub mod extensions;
pub mod extscreen;
pub mod help;
pub mod key_input;
pub mod loading;
pub mod markdown;
pub mod mcp;
pub mod message_rewind;
pub mod model_cmd;
pub mod onboard;
pub mod onboard_provider;
pub mod quit_confirm;
pub mod scroll;
pub mod security;
pub mod session_hub;
pub mod session_picker;
pub mod settings;
pub mod store;
pub mod theme;
pub mod todo;
pub mod usage;

use crate::app::mode::Mode;
use crate::app::resolve::resolve_turn_model;
use crate::app::state::{AppState, AppStateRest};
use ratatui::style::Style;
use ratatui::Frame;

/// Resolve the concrete model id driving the foreground session's NEXT turn,
/// mirroring the logic the request layer uses (`resolve_turn_model`): Main,
/// except while the session is in `AgentMode::Plan` with a Planner assigned
/// that differs from Main, in which case the label shows the Planner's model.
/// Session overrides win; falls back to the legacy `settings.model` field;
/// defaults to empty string when there is no session at all.
/// Clear a rect and fill it with a solid background, so overlays/popups render on
/// the theme's raised-surface color instead of the terminal's default background
/// (which is what a bare `Clear` leaves behind). Draw your Block/Paragraph AFTER
/// this — widgets with default (bg: None) styles won't overwrite the fill.
pub(crate) fn clear_and_fill(
    frame: &mut ratatui::Frame,
    rect: ratatui::layout::Rect,
    bg: ratatui::style::Color,
) {
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame
        .buffer_mut()
        .set_style(rect, ratatui::style::Style::default().bg(bg));
}

fn resolved_main_model(rest: &AppStateRest) -> String {
    let Some(session) = rest.fg().session.as_ref() else {
        return String::new();
    };
    // The thin client's shadow clears `config.models`/`config.providers` every
    // snapshot (`client/shadow.rs`) and never projects `settings.session_models`
    // either (`client_shadow/session.rs::shadow_session` only seeds `name` +
    // `model`), so `resolve_role`/`resolve_turn_model` can NEVER find an
    // assignment there — `resolve_role_dispatch`'s last-resort koma-free
    // substitute fires on EVERY call, silently masking the real model behind
    // the free-tier route id. The daemon already precomputes the correct id
    // (`resolved_model_id`, via plain `resolve_role` — see
    // `ipc/snapshot/projection/core.rs`) and the client seeds it straight into
    // `settings.model`, so that field is the trustworthy source there.
    //
    // Distinguish shadow-vs-standalone the same way `Mode::Mcp`'s arm above
    // does: `rest.mcp_manager` is `None` ONLY for the thin client
    // (`lifecycle::build_startup` leaves it `None` under `--daemon`, letting
    // the daemon's own MCP manager own it) and always `Some` in the old
    // standalone `--local`/`alone` TUI (`event_loop::run_loop`, the one other
    // caller of this view — see `lifecycle/mod.rs`'s `if opts.daemon` branch).
    // This is a direct mode discriminator, unlike inferring it from whether
    // Main happens to resolve usably: a degraded standalone Main (deleted
    // provider, expired OAuth) legitimately resolves to "not usable" too, and
    // gating on usability would wrongly show the frozen `settings.model`
    // instead of the live (koma-free) route dispatch actually uses —
    // disagreeing with the `main_fallback_reason` toast for that exact case.
    if rest.mcp_manager.is_none() {
        return session.settings.model.clone();
    }
    if let Some(r) = resolve_turn_model(&rest.config, &session.settings, rest.agent_mode) {
        // Standalone: trust the live resolver. Suppress only the pure soft-
        // fallback case where the answer is still the unused serde default
        // (`openai/gpt-4o-mini`) AND a real Main ModelEntry exists in the
        // catalogue — that means the entry's provider is dangling and the
        // legacy string is lying about the active model. Prefer empty over a
        // phantom gpt label the user never configured.
        if r.model_id == crate::config::DEFAULT_MODEL
            && session.settings.model == crate::config::DEFAULT_MODEL
        {
            let has_main_entry = session
                .settings
                .session_models
                .iter()
                .chain(rest.config.models.iter())
                .any(|e| {
                    e.effective_roles()
                        .contains(&crate::model::app_config::ModelRole::Main)
                });
            if has_main_entry {
                return String::new();
            }
        }
        return r.model_id;
    }
    // No resolution at all — do NOT fall back to the dead DEFAULT_MODEL string.
    String::new()
}

/// Render the entire terminal frame for the current application state.
///
/// Called by the runtime on every UI refresh tick.  Delegates to the
/// mode-specific draw function; only one mode is active at a time.
///
/// The palette is computed once here and passed to every sub-draw so all
/// colour decisions flow through a single source of truth.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let palette = theme::palette(&state.rest.config);
    // Paint the whole frame with the theme canvas background first, so every
    // otherwise-unstyled cell — across ALL modes at once — picks up the palette bg.
    // Mode renderers draw their own styled cells on top; only untouched cells keep
    // this. (`frame.area()` is hoisted into a local so it isn't a shared borrow of
    // `frame` while `buffer_mut()` holds the mutable borrow.)
    let area = frame.area();
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(palette.bg));
    // The catalogue is now per-endpoint and fetched on demand: pass BOTH the
    // cached models and the endpoint they were fetched for, so each omnisearch view
    // can tell "this is my provider's catalogue" (filter locally) from "still
    // fetching / stale" (show `searching models…`) and "fetched but empty"
    // (`no models — type an id`).
    let cache = state.rest.models_cache.as_deref().unwrap_or(&[]);
    let cache_endpoint = state.rest.models_cache_endpoint.as_deref();
    match state.mode() {
        Mode::Chat => {
            let resolved_model = resolved_main_model(&state.rest);
            chat::draw(frame, &state.rest, &resolved_model, &palette);
        }
        Mode::Onboard(o) => onboard::draw(frame, o, &palette),
        Mode::OnboardProvider(op) => {
            onboard_provider::draw(frame, op, cache, cache_endpoint, &palette)
        }
        Mode::KeyInput(form) => {
            key_input::draw(frame, &state.rest, form, cache, cache_endpoint, &palette)
        }
        Mode::SessionPicker(p) => session_picker::draw(frame, &state.rest, p, &palette),
        Mode::SessionHub(h) => session_hub::draw(frame, &state.rest, h, &palette),
        Mode::Settings(s) => {
            if s.page == crate::app::mode::settings::SettingsPage::Menu {
                let resolved_model = resolved_main_model(&state.rest);
                chat::draw(frame, &state.rest, &resolved_model, &palette);
                let chunks = chat::layout_chunks(&state.rest, frame.area());
                settings::render_menu_overlay(frame, s, &palette, chunks[4], chunks[1]);
            } else {
                settings::draw(
                    frame,
                    &state.rest,
                    s,
                    cache,
                    cache_endpoint,
                    &palette,
                    frame.area(),
                );
            }
        }
        Mode::Agents(a) => agents::draw(
            frame,
            &state.rest,
            a,
            &state.rest.config,
            state.rest.fg().session.as_ref().map(|s| &s.settings),
            &palette,
        ),
        Mode::Mcp(m) => {
            // Live per-server tool counts from the MCP manager snapshot (owned map
            // so the manager lock isn't held across the draw). The local TUI reads
            // the live manager; a thin client owns NO manager, so it falls back to
            // the status projected into the shadowed state (`m.shadow_status`). Feeds
            // the LIST + detail status display.
            let status = state
                .rest
                .mcp_manager
                .as_ref()
                .map(|mgr| mgr.server_status_cached())
                .or_else(|| m.shadow_status.clone());
            mcp::draw(frame, m, status.as_ref(), &palette);
        }
        Mode::Extensions(e) => extensions::draw(frame, e, &palette),
        Mode::ExtScreen(s) => extscreen::draw(frame, s, &palette),
        Mode::ExtStore(s) => store::draw(frame, s, &palette),
        Mode::Security(s) => security::draw(frame, s, &palette),
        Mode::Bash(b) => {
            let resolved_model = resolved_main_model(&state.rest);
            chat::draw(frame, &state.rest, &resolved_model, &palette);
            let chunks = chat::layout_chunks(&state.rest, frame.area());
            // chunks[4] = input box, chunks[1] = transcript (6-chunk layout)
            bash::render_bash_overlay(frame, chunks[4], chunks[1], &b.jobs, b.selected, &palette);
        }
        Mode::Todo(t) => {
            let resolved_model = resolved_main_model(&state.rest);
            chat::draw(frame, &state.rest, &resolved_model, &palette);
            let chunks = chat::layout_chunks(&state.rest, frame.area());
            // chunks[4] = input box, chunks[1] = transcript (6-chunk layout)
            todo::render_todo_overlay(
                frame,
                chunks[4],
                chunks[1],
                &state.rest,
                &t.items,
                t.selected,
                t.completed_count(),
                &palette,
            );
        }
        Mode::Help(h) => help::draw(frame, &state.rest, h, &palette),
        Mode::Effort(e) => effort::draw(frame, &state.rest, e, &palette),
        Mode::Model(m) => {
            let resolved_model = resolved_main_model(&state.rest);
            chat::draw(frame, &state.rest, &resolved_model, &palette);
            let chunks = chat::layout_chunks(&state.rest, frame.area());
            model_cmd::render_overlay(frame, m, &palette, chunks[4], chunks[1]);
        }
        Mode::Loading(s) => loading::draw(frame, s, &palette),
        Mode::Usage(nav) => {
            // The dashboard renders from a pre-fetched ledger projection so the SAME
            // draw path serves the local TUI and the daemon's thin client. The client
            // receives the projection in the snapshot (`rest.usage_data`); a local TUI
            // leaves that `None` and collects it live from the ledger here every frame
            // (unchanged behaviour). See `model::usage::UsageData`.
            let data = state
                .rest
                .usage_data
                .clone()
                .unwrap_or_else(|| usage::collect_usage_data(nav, &state.rest));
            usage::draw(frame, &state.rest, nav, &data, &palette);
        }
        Mode::MessageRewind(rw) => {
            // Draw the normal chat view first, THEN the rewind overlay on top —
            // exactly how the `/bash` overlay layers over chat (same input/transcript
            // rects from `layout_chunks`).
            let resolved_model = resolved_main_model(&state.rest);
            chat::draw(frame, &state.rest, &resolved_model, &palette);
            let chunks = chat::layout_chunks(&state.rest, frame.area());
            // chunks[4] = input box, chunks[1] = transcript (6-chunk layout)
            message_rewind::draw(frame, chunks[4], chunks[1], &state.rest, rw, &palette);
        }
        Mode::QuitConfirm(s) => quit_confirm::draw(frame, s, &palette),
    }
}
