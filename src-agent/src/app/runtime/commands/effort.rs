//! Effort picker command: `/effort`

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{EffortPickerState, Mode};
use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Generic effort menu used when the model catalogue can't be fetched (network
/// failure). Covers the common tokens so the user can still set something; the
/// accompanying note tells them capabilities are unknown.
pub(super) const GENERIC_EFFORTS: &[&str] = &["default", "off", "low", "medium", "high", "max"];

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
/// catalogue fetch uses the client). Blocked while a request is in flight,
/// mirroring the /settings + /compact guards.
pub(super) fn handle_effort(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    // `_c` only gates "is there a usable client?"; the catalogue is now
    // fetched on demand by the debounced tick, not here.
    let (Some(_c), Some(settings)) = (
        client.as_ref(),
        state.rest.fg().session.as_ref().map(|s| s.settings.clone()),
    ) else {
        state.rest.status = "no active session".into();
        return Ok(());
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

    // The catalogue is fetched ON DEMAND now (no boot/disk cache, no
    // block_on). Arm a debounced fetch for the Main endpoint so a SUBSEQUENT
    // `/effort` open has capabilities; this open uses whatever `models_cache`
    // already holds FOR THE MAIN ENDPOINT, else falls back to the generic
    // menu. `request_catalogue` no-ops when the cache already covers it.
    if let Some(r) = main.as_ref() {
        state.rest.request_catalogue(&r.endpoint, &r.api_key);
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
                state.rest.status = "model has no thinking control".into();
                return Ok(());
            }
        }
    } else {
        // Fetch failed (cache still None): generic fallback menu.
        (
            GENERIC_EFFORTS.iter().map(|s| s.to_string()).collect(),
            "couldn't fetch model capabilities".to_string(),
        )
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
