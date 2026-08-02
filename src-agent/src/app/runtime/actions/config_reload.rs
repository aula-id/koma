//! Shared helpers for reloading global catalogue from disk and broadcasting
//! config changes to peer daemons.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::app_config::AppConfig;

/// Reload global catalogue from disk into this daemon and refresh dependent UI.
///
/// Called on Attach (so a reconnecting client always sees fresh config) and on
/// receiving a `ReloadGlobalCatalogue` IPC broadcast from a peer daemon that
/// just saved global config.
pub(crate) fn apply_global_catalogue_reload(state: &mut AppState) {
    // 1. Reload config from disk (AppConfig::load() has no cache — fresh every time).
    state.rest.config = AppConfig::load();

    // 2. Rebuild system prompt sub-agent roster (disk-backed agents may have changed).
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.rebuild_system();
    }

    // 3. Refresh open Settings/Agents modes where cheap.
    //    Uses take/put-back pattern (NOT mode_mut) because we need state.rest
    //    while the mode is owned outside the borrow.
    let mut mode = state.take_mode();
    match &mut mode {
        crate::app::mode::Mode::Settings(s) => {
            // Rebuild the full SettingsState from fresh config + session.
            if let Some(session) = state.rest.fg().session.as_ref() {
                let cfg = state.rest.config.clone();
                **s = crate::app::mode::settings::SettingsState::from(session, &cfg);
            }
        }
        crate::app::mode::Mode::Agents(a) => {
            if let Some(session) = state.rest.fg().session.as_ref() {
                a.reload(session);
            }
        }
        _ => {}
    }
    state.set_mode(mode);
}

/// Save config to disk and broadcast the change to all peer daemons.
///
/// The broadcast runs on a background OS thread so the calling daemon's event
/// loop never blocks on peer socket connects. Used instead of bare
/// `config.save()` at sites that mutate the global catalogue (models,
/// providers, theme, etc.).
pub(crate) fn save_config_and_broadcast(config: &AppConfig) -> Result<()> {
    config.save()?;
    // Spawn broadcast off the event loop so N socket connects can't stall us.
    std::thread::spawn(|| {
        crate::app::runtime::manage::broadcast_reload_global_catalogue();
    });
    Ok(())
}
