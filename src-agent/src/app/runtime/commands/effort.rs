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

/// Handle the `/effort` command: open the effort picker for the current model.
///
/// Needs an active session + client (the menu is per-model and the
/// catalogue fetch uses the client). Opening a picker overlay is always safe
/// mid-stream (read-only view; the turn keeps streaming), so there is no busy
/// guard here. The effort value is only written when the user CONFIRMS a
/// selection inside the picker, which is a separate handler.
pub(super) fn handle_effort(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    // `_c` only gates "is there a usable client?"; the catalogue is now
    // fetched on demand by the debounced tick, not here.
    let (Some(_c), Some(settings)) = (
        client.as_ref(),
        state.rest.fg().session.as_ref().map(|s| s.settings.clone()),
    ) else {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    };
    let model = settings.model.clone();
    // Resolve the MAIN role (the effort menu is per the chat model, served
    // by the Main endpoint). Snapshot the route into an owned local so it
    // doesn't borrow `state.rest` across the mutation below. Honour THIS
    // session's `/free` toggle — like /compact, a free-mode session's Main
    // route is the keyless koma-free tier, not whatever is configured.
    let free_mode = state.rest.fg().free_mode;
    let main = crate::app::resolve::resolve_role_free(
        &state.rest.config,
        &settings,
        crate::model::app_config::ModelRole::Main,
        free_mode,
    );

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
        let already_pending = state.rest.catalogue_pending.as_ref()
            .is_some_and(|p| p.endpoint == r.endpoint);
        let already_fetching = state.rest.catalogue_fetching.as_deref()
            == Some(r.endpoint.as_str());
        if !already_pending && !already_fetching {
            state.rest.request_catalogue(&r.endpoint, &r.api_key, &r.oauth_uuid);
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
    let (options, note) = if let Some(models) =
        state.rest.models_cache.as_ref().filter(|_| cache_for_main)
    {
        let caps = crate::service::openrouter::effort_caps(models, &model);
        match build_effort_options(&caps) {
            Some(opts) => {
                let note = if caps.efforts.is_empty() {
                    "thinking on/off only".to_string()
                } else if caps.mandatory {
                    "reasoning is always on for this model".to_string()
                } else {
                    "pick a thinking effort".to_string()
                };
                (opts, note)
            }
            None => {
                // No reasoning control: don't open the menu, just say so.
                state.rest.fg_mut().status = "model has no thinking control".into();
                return Ok(());
            }
        }
    } else {
        // Cache not available or doesn't match Main endpoint.
        // Show a status instead of a generic menu.
        let status = if main.is_some() {
            if prev_fetch_failed {
                "couldn't fetch capabilities — retrying..."
            } else {
                "fetching model capabilities..."
            }
        } else {
            "model capabilities unavailable"
        };
        state.rest.fg_mut().status = status.into();
        return Ok(());
    };

    let stored = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.effort.clone())
        .unwrap_or_default();
    let selected = preselect_effort(&options, &stored);
    *state.mode_mut() = Mode::Effort(Box::new(EffortPickerState {
        options,
        selected,
        note,
    }));
    Ok(())
}
