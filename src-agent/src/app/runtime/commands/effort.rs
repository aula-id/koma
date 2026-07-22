//! Effort picker command: `/effort`

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{EffortPickerState, Mode};
use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Append `opt` to `out` unless it's already present (case-sensitive). Keeps the
/// option list deduped while preserving the order options are added in.
pub(super) fn push_unique(out: &mut Vec<String>, opt: &str) {
    if !out.iter().any(|o| o == opt) {
        out.push(opt.to_string());
    }
}

/// Build the `/effort` option list from a model's derived [`EffortCaps`].
///
/// Returns `None` when the model has no reasoning control at all (the caller
/// toasts and does NOT open the menu). Otherwise:
/// - discrete efforts reported → `["default","off"] + efforts` (deduped, model
///   order preserved); `"off"` dropped when reasoning is mandatory.
/// - supported but no discrete efforts (on/off only) → `["default","off","max"]`
///   (`"max"` == thinking on); `"off"` dropped when mandatory.
///
/// `"default"` is always first so the model-default choice is one keypress away.
pub(super) fn build_effort_options(
    caps: &crate::service::openrouter::EffortCaps,
) -> Option<Vec<String>> {
    if !caps.supported {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    push_unique(&mut out, "default");
    if !caps.mandatory {
        push_unique(&mut out, "off");
    }
    if caps.efforts.is_empty() {
        // On/off-only model: "max" stands in for "thinking on".
        push_unique(&mut out, "max");
    } else {
        for e in &caps.efforts {
            push_unique(&mut out, e);
        }
    }
    Some(out)
}

/// Index of the option matching the session's stored `effort` (empty → the
/// `"default"` entry). Falls back to 0 when the stored value isn't offered.
pub(super) fn preselect_effort(options: &[String], effort: &str) -> usize {
    let want = if effort.is_empty() { "default" } else { effort };
    options.iter().position(|o| o == want).unwrap_or(0)
}

/// Outcome of deriving the `/effort` menu for the current model, shared by the
/// TUI `/effort` command and the GUI's `GetEffortOptions` daemon request — both
/// need the IDENTICAL per-model derivation (incl. the cold-cache fetch-arm side
/// effect below), just with different presentations of the result:
/// - TUI: `Loading`/`Unsupported` become a status-line message; `Ready` opens
///   `Mode::Effort`.
/// - GUI: all three become a `DaemonEvent::EffortOptions` reply (`state`
///   `"loading"`/`"unsupported"`/`"ready"`) so the picker never hangs.
pub(crate) enum EffortMenu {
    /// No usable (cached, Main-endpoint-matching) catalogue yet — a fetch has
    /// just been armed (or was already in flight). `String` is the status
    /// message to show while waiting (mirrors the TUI's prior inline text).
    Loading(String),
    /// The model has no reasoning control at all (or there's no active
    /// session/client to derive one from). `String` explains why; the caller
    /// shows it and does NOT open a menu.
    Unsupported(String),
    /// A menu is ready to show.
    Ready {
        options: Vec<String>,
        selected: usize,
        note: String,
    },
}

/// Build a [`EffortMenu::Ready`]/[`EffortMenu::Unsupported`] outcome from
/// already-derived [`EffortCaps`], shared by BOTH capability sources: the live
/// `models_cache` catalogue (OpenRouter) and the curated `catalogue_overlay`
/// (non-OpenRouter). Keeps the option-list/note/preselect logic in one place
/// so the two callers can't drift.
fn ready_from_caps(state: &AppState, caps: &crate::service::openrouter::EffortCaps) -> EffortMenu {
    match build_effort_options(caps) {
        Some(options) => {
            let note = if caps.efforts.is_empty() {
                "thinking on/off only".to_string()
            } else if caps.mandatory {
                "reasoning is always on for this model".to_string()
            } else {
                "pick a thinking effort".to_string()
            };
            let stored = state
                .rest
                .fg()
                .session
                .as_ref()
                .map(|s| s.settings.effort.clone())
                .unwrap_or_default();
            let selected = preselect_effort(&options, &stored);
            EffortMenu::Ready {
                options,
                selected,
                note,
            }
        }
        None => {
            // No reasoning control: don't open the menu, just say so.
            EffortMenu::Unsupported("model has no thinking control".to_string())
        }
    }
}

/// Derive the `/effort` menu for the current model (or report why one isn't
/// available yet). Needs an active session + client (the menu is per-model and
/// the catalogue fetch uses the client's endpoint).
///
/// Side effect: when the Main route resolves AND it's an OpenRouter endpoint
/// (or the catalogue overlay doesn't cover it), this ARMS a debounced
/// catalogue fetch for its endpoint (if one isn't already pending/in-flight)
/// so a SUBSEQUENT call has capabilities — this fires for BOTH callers (TUI
/// open, GUI `GetEffortOptions`), which is exactly what lets a GUI-triggered
/// cold fetch warm the cache for a following TUI/GUI open.
pub(crate) fn effort_menu(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> EffortMenu {
    // `_c` only gates "is there a usable client?"; the catalogue is now
    // fetched on demand by the debounced tick, not here.
    let (Some(_c), Some(settings)) = (
        client.as_ref(),
        state.rest.fg().session.as_ref().map(|s| s.settings.clone()),
    ) else {
        return EffortMenu::Unsupported("no active session".to_string());
    };
    let model = settings.model.clone();
    // Resolve the MAIN role (the effort menu is per the chat model, served
    // by the Main endpoint). Snapshot the route into an owned local so it
    // doesn't borrow `state.rest` across the mutation below.
    let main = crate::app::resolve::resolve_role(
        &state.rest.config,
        &settings,
        crate::model::app_config::ModelRole::Main,
    );

    // Non-OpenRouter endpoints (Codex `chatgpt.com/backend-api/codex`,
    // Claude `api.anthropic.com`, xAI, DeepSeek, ...) don't expose a real
    // `GET /models` listing, so the `models_cache` path below can NEVER
    // resolve for them — without this, `/effort` sits in `Loading` forever
    // (or churns "couldn't fetch capabilities — retrying..." once the armed
    // fetch inevitably 404s/400s). Check the curated `catalogue_overlay`
    // FIRST, before arming any fetch, so a hit short-circuits straight to
    // `Ready` with NO network round-trip and NO Loading stall. OpenRouter
    // itself is EXCLUDED from this path: it self-describes via a real
    // `/models` call, so its live cache stays authoritative and the overlay
    // is never consulted for it.
    if let Some(r) = main.as_ref() {
        if !crate::service::openrouter::is_openrouter(&r.endpoint) {
            let overlay = crate::service::catalogue_overlay::models_for(&r.endpoint);
            if overlay.iter().any(|m| m.id == r.model_id) {
                let caps = crate::service::openrouter::effort_caps(&overlay, &r.model_id);
                return ready_from_caps(state, &caps);
            }
        }
    }

    // Arm a debounced fetch for the Main endpoint so a SUBSEQUENT
    // `/effort` open has capabilities. This open uses a matching successful
    // cache if available; otherwise it reports loading/error and does NOT
    // open a guessed generic menu.
    // Capture whether THIS endpoint had a prior fetch failure BEFORE we clear
    // the marker below. The clear wipes it in this same call, so the status
    // branch further down must read this captured flag, not the field.
    let mut prev_fetch_failed = false;
    if let Some(r) = main.as_ref() {
        // Clear any prior fetch failure so user-triggered /effort retries.
        if state.rest.models_cache_failed.as_deref() == Some(r.endpoint.as_str()) {
            prev_fetch_failed = true;
            state.rest.models_cache_failed = None;
        }
        // Only arm the fetch if we don't already have a pending/in-flight
        // request for this endpoint — prevents rapid /effort opens from
        // constantly pushing the debounce forward.
        let already_pending = state
            .rest
            .catalogue_pending
            .as_ref()
            .is_some_and(|p| p.endpoint == r.endpoint);
        let already_fetching =
            state.rest.catalogue_fetching.as_deref() == Some(r.endpoint.as_str());
        if !already_pending && !already_fetching {
            state
                .rest
                .request_catalogue(&r.endpoint, &r.api_key, &r.oauth_uuid);
        }
    }
    // Only trust `models_cache` when it was fetched for the Main endpoint;
    // a cache for some OTHER provider's endpoint must not drive THIS model's
    // capability menu.
    let cache_for_main = main
        .as_ref()
        .map(|r| state.rest.models_cache_endpoint.as_deref() == Some(r.endpoint.as_str()))
        .unwrap_or(false);

    // Build the option list + capability note from the (cached) catalogue.
    // MODEL-ID FIX (mirrors run.rs's WINDOW-SIZING FIX): look up caps by the
    // RESOLVED Main model id (what we actually send), NOT the legacy
    // `settings.model` — a per-session or config Main override must resolve
    // capabilities for the model actually in use. Falls back to
    // `settings.model` only when `main` didn't resolve.
    let model_for_caps = main.as_ref().map(|r| r.model_id.as_str()).unwrap_or(&model);
    if let Some(models) = state.rest.models_cache.as_ref().filter(|_| cache_for_main) {
        let caps = crate::service::openrouter::effort_caps(models, model_for_caps);
        ready_from_caps(state, &caps)
    } else {
        // Cache not available or doesn't match Main endpoint.
        // Report loading instead of a generic menu.
        let status = if main.is_some() {
            if prev_fetch_failed {
                "couldn't fetch capabilities — retrying..."
            } else {
                "fetching model capabilities..."
            }
        } else {
            "model capabilities unavailable"
        };
        EffortMenu::Loading(status.to_string())
    }
}

/// Handle the `/effort` command: open the effort picker for the current model.
///
/// Opening a picker overlay is always safe mid-stream (read-only view; the
/// turn keeps streaming), so there is no busy guard here. The effort value is
/// only written when the user CONFIRMS a selection inside the picker, which is
/// a separate handler. Unchanged behavior from before the `effort_menu`
/// extraction: `Loading`/`Unsupported` set the SAME status-line text the
/// inline logic used to, and `Ready` opens the SAME `Mode::Effort`.
pub(super) fn handle_effort(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    match effort_menu(state, client) {
        EffortMenu::Loading(msg) | EffortMenu::Unsupported(msg) => {
            state.rest.fg_mut().status = msg;
        }
        EffortMenu::Ready {
            options,
            selected,
            note,
        } => {
            *state.mode_mut() = Mode::Effort(Box::new(EffortPickerState {
                options,
                selected,
                note,
            }));
        }
    }
    Ok(())
}
