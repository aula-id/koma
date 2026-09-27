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
    let previous = state.rest.config.clone();
    match AppConfig::try_load() {
        Ok(config) => state.rest.config = config,
        Err(error) => {
            state.rest.fg_mut().status = format!("catalogue reload failed: {error}");
            return;
        }
    }
    let dead_models: std::collections::HashSet<String> = previous
        .models
        .iter()
        .filter(|m| {
            !state
                .rest
                .config
                .models
                .iter()
                .any(|live| live.uuid == m.uuid)
        })
        .map(|m| m.uuid.clone())
        .collect();
    let live_connections: std::collections::HashSet<&str> = state
        .rest
        .config
        .providers
        .iter()
        .map(|p| p.uuid.as_str())
        .chain(
            state
                .rest
                .config
                .oauth_conns
                .iter()
                .map(|p| p.uuid.as_str()),
        )
        .collect();
    let dead_providers: std::collections::HashSet<String> = previous
        .providers
        .iter()
        .map(|p| &p.uuid)
        .chain(previous.oauth_conns.iter().map(|p| &p.uuid))
        .filter(|uuid| !live_connections.contains(uuid.as_str()))
        .cloned()
        .collect();
    if !dead_models.is_empty() || !dead_providers.is_empty() {
        let config = state.rest.config.clone();
        crate::app::cascade::rebind_consumers_after_model_removal(
            Some(state),
            &config,
            &dead_models,
            &dead_providers,
            false,
        );
    }

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
