//! Free command: `/free` — toggle THIS session onto the keyless koma-free tier.
//!
//! Session-scoped and ephemeral: flips the foreground session's
//! [`crate::app::state::SessionRuntime::free_mode`], never touching `AppConfig` or
//! the persisted `Settings`. While on, the Main role and the roles that inherit it
//! (Compactor / Awareness) resolve to the keyless koma-free route (see
//! [`crate::app::resolve::resolve_role_free`]); Planner and Safeguard are untouched.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::app_config::ModelRole;

/// Handle the `/free` command: toggle the foreground session's `free_mode`.
///
/// - off -> on: switch onto koma-free (works even with NO configured Main — the tier
///   is keyless), toast `switched to koma free`.
/// - on -> off: toast `back to <main model>` (the configured Main model id, resolved
///   with free-mode OFF), or `back to your model` when no Main is configured.
///
/// No config write: `free_mode` lives only on the session runtime and resets on a
/// fresh session.
pub(super) fn handle_free(state: &mut AppState) -> Result<()> {
    let now_on = !state.rest.fg().free_mode;
    state.rest.fg_mut().free_mode = now_on;

    if now_on {
        // Toggling ON: the koma-free `X-Koma` header must be stable across restarts
        // for rate-bucket continuity. `install_id` is serde-default + Default-minted,
        // but mint one defensively if it somehow got cleared, then PERSIST it — this
        // is the only config write `/free` performs (`free_mode` itself stays
        // session-scoped and is never saved). Mirrors the same defensive pattern in
        // `handle_setup_koma_free` (actions/onboard.rs).
        if state.rest.config.install_id.is_empty() {
            state.rest.config.install_id = crate::model::app_config::new_uuid();
        }
        let _ = state.rest.config.save();
        state
            .rest
            .fg_mut()
            .set_toast_info("switched to koma free".to_string());
        return Ok(());
    }

    // Toggled OFF: name the configured Main model in the toast. `free_mode` is already
    // false, so pass `false` to resolve the REAL configured Main route (not koma-free).
    let main_model = state.rest.fg().session.as_ref().and_then(|s| {
        crate::app::resolve::resolve_role_free(
            &state.rest.config,
            &s.settings,
            ModelRole::Main,
            false,
        )
    });
    let msg = match main_model.map(|r| r.model_id).filter(|m| !m.is_empty()) {
        Some(m) => format!("back to {m}"),
        None => "back to your model".to_string(),
    };
    state.rest.fg_mut().set_toast_info(msg);
    Ok(())
}
