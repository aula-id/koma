//! Miscellaneous simple commands: `/mode`, `/settings`, `/agents`, `/select`,
//! `/help`, `/quit`, `/usage`.

use anyhow::Result;

use crate::app::mode::{AgentsState, HelpState, Mode, SettingsState};
use crate::app::state::{AgentMode, AppState};
use crate::app::version;
use crate::model::store;

/// Handle the `/mode` command.
///
/// `arg` (the token after `/mode`, already lowercased):
/// - `None` → armed-aware CYCLE (Auto→Normal→[Yolo when armed]→Auto), the same
///   transition as Shift+Tab.
/// - `Some("auto")` / `Some("normal")` → explicitly set that mode (always allowed).
/// - `Some("yolo")` → enter YOLO **only when armed** (Layer 2). When NOT armed it
///   REFUSES: the mode is left unchanged and the status explains how to unlock it.
/// - any other token → leave the mode unchanged and report the bad argument.
pub(super) fn handle_mode(state: &mut AppState, arg: Option<String>) -> Result<()> {
    match arg.as_deref() {
        None => {
            // Bare `/mode`: armed-aware cycle (identical to the Shift+Tab toggle).
            state.rest.agent_mode = state.rest.agent_mode.cycle(state.rest.yolo_armed);
        }
        Some("auto") => state.rest.agent_mode = AgentMode::Auto,
        Some("normal") => state.rest.agent_mode = AgentMode::Normal,
        Some("yolo") => {
            // Layer-2 gate: only an ARMED YOLO may be entered. Unarmed → refuse and
            // leave the mode untouched.
            if state.rest.yolo_armed {
                state.rest.agent_mode = AgentMode::Yolo;
            } else {
                state.rest.status = "yolo locked — enable it in /security first".into();
                return Ok(());
            }
        }
        Some(other) => {
            state.rest.status = format!("unknown mode: {other} (auto | normal | yolo)");
            return Ok(());
        }
    }
    state.rest.status = format!("mode: {}", state.rest.agent_mode.label());
    Ok(())
}

/// Handle the `/settings` command: open the settings modal.
///
/// Needs an active session (drafts seed from it); also blocked while a
/// request is in flight, mirroring the /compact guard.
pub(super) fn handle_settings(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    // No catalogue prefetch here anymore: the Models Select modal's
    // omnisearch fetches the EDITED provider's `/models` on demand
    // (debounced) the first time it opens / a key is typed — see
    // `controller::input::handle_settings`. A boot prefetch keyed to the
    // Main endpoint was wrong for editing a model on a DIFFERENT provider.
    let Some(session) = state.rest.fg().session.as_ref() else {
        state.rest.status = "no active session".into();
        return Ok(());
    };
    let st = SettingsState::from(session, &state.rest.config);
    state.mode = Mode::Settings(Box::new(st));
    Ok(())
}

/// Handle the `/agents` command: open the agents modal.
///
/// Needs an active session (the registry loads from it); also blocked
/// while a request is in flight, mirroring the /settings + /compact guards.
pub(super) fn handle_agents(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    let Some(session) = state.rest.fg().session.as_ref() else {
        state.rest.status = "no active session".into();
        return Ok(());
    };
    let st = AgentsState::from(session);
    state.mode = Mode::Agents(Box::new(st));
    Ok(())
}

/// Handle the `/select` command: arm text-selection mode.
pub(super) fn handle_select(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    if state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    state.rest.select_pending = true;
    Ok(())
}

/// Handle the `/help` command: open the full-screen, searchable help reference.
///
/// Read-only and self-contained (the entry list is built from the static
/// COMMANDS + KEYBINDINGS registries), so there is no busy/session guard — it
/// opens regardless of session state, like `/usage`.
pub(super) fn handle_help(state: &mut AppState) -> Result<()> {
    // Resolve the "Updating koma" version data from app state at open time: the
    // compiled-in version, plus the fetched latest IFF it is strictly newer.
    let current = store::current_version();
    let update = state
        .rest
        .latest_version
        .as_ref()
        .filter(|v| version::is_newer(&v.version, current))
        .map(|v| (v.version.clone(), v.message.clone()));
    let st = HelpState::new().with_version(current.to_string(), update);
    state.mode = Mode::Help(Box::new(st));
    Ok(())
}

/// Handle the `/quit` command: route through the shared quit chokepoint.
///
/// Identical behaviour to the quit keybind (`Action::Quit`): if no session is
/// working, quit immediately; if any session has work in flight, open the
/// kill-all / detach / cancel confirm overlay. The on-disk locks for every
/// session are released on the natural exit path.
pub(super) fn handle_quit(state: &mut AppState) -> Result<()> {
    super::super::actions::quit::request_quit(state);
    Ok(())
}

/// Handle the `/usage` command: open the cost/token usage dashboard.
///
/// Read-only; no waiting guard needed (the dashboard never writes).
pub(super) fn handle_usage(state: &mut AppState) -> Result<()> {
    state.mode = Mode::Usage(Box::default());
    Ok(())
}
