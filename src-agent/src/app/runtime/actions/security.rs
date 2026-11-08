//! Action handlers for the `/security` daemon control panel.
//!
//! Actions:
//! - `CloseSecurity`        — return to Chat.
//! - `SecurityRestart`      — stop then start the daemon (`r`; the only daemon-lifecycle
//!   action with its own key — start/stop/toggle fold into the Daemon checkbox).
//! - `SecurityToggleTool`   — toggle the SELECTED row. The tools pane mixes the daemon
//!   checkbox (row 0) and the YOLO checkbox (row 1) with the tool inventory, so this
//!   branches on the selected [`SecSel`]: `Daemon` starts/stops the daemon; `Yolo`
//!   arms/disarms the Layer-1 YOLO flag (gated on the daemon running; disarming while in
//!   Yolo drops `agent_mode` back to Auto); `Tool(i)` flips that tool's membership in
//!   `state.rest.sec_inactive`.
//! - `SecurityToggleDomain` — toggle every tool sharing the selected tool's domain
//!   (no-op when the daemon or YOLO checkbox is selected — it has no domain).
//! - `SecurityInstall`      — install/repair one dependency, then re-fetch install-health.
//!
//! After every lifecycle action the open `Mode::Security` state is refreshed from
//! the live manager so the panel reflects the new daemon state immediately. The plain
//! refreshes carry `install_health` untouched (it is a heavy IPC round-trip); only the
//! install path re-fetches it.

use anyhow::Result;

use crate::app::mode::{Mode, SecSel};
use crate::app::sec::InstallHealthEntry;
use crate::app::state::{AgentMode, AppState};

/// Handle `Action::CloseSecurity`: return to Chat.
pub(super) fn handle_close_security(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Start the daemon. Called when the Daemon checkbox is toggled ON (from
/// `handle_security_toggle_tool`), and from the headless IPC
/// [`crate::ipc::proto::ClientRequest::SetSecurityEnabled`] path.
///
/// Sets `security_enabled` to true so subsequent turns advertise sec_ tools.
pub(crate) fn handle_security_start(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = true;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.start(state.rest.sec_token.clone());
    }
    state.rest.fg_mut().status = "security: starting daemon…".into();
    refresh_security_state(state, None);
    Ok(())
}

/// Stop the daemon. Called when the Daemon checkbox is toggled OFF (from
/// `handle_security_toggle_tool`), and from the headless IPC
/// [`crate::ipc::proto::ClientRequest::SetSecurityEnabled`] path.
///
/// Clears `security_enabled` so sec_ tools are no longer advertised. ENFORCES the
/// "YOLO requires a running daemon" invariant: stopping the daemon always disarms YOLO
/// (and drops out of `Yolo` agent mode) so a harness-bypass can never outlive the daemon.
pub(crate) fn handle_security_stop(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = false;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.stop();
    }
    disarm_yolo_for_stop(state);
    state.rest.fg_mut().status = "security: daemon stopped".into();
    refresh_security_state(state, None);
    Ok(())
}

/// Set Layer-1 YOLO arm flag without requiring the Security panel to be open.
/// Same gate as the panel YOLO checkbox: arming needs a running security daemon.
/// Disarming while in `Yolo` drops `agent_mode` to `Auto`.
pub(crate) fn handle_set_yolo_armed(state: &mut AppState, armed: bool) -> Result<()> {
    let running = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status().running)
        .unwrap_or(false);
    if armed && !running {
        state.rest.fg_mut().status = "yolo locked — start the daemon first".into();
        anyhow::bail!("yolo arm refused: security daemon not running");
    }
    state.rest.yolo_armed = armed;
    if armed {
        state.rest.fg_mut().status = "yolo armed — switch with /mode yolo or Shift+Tab".into();
    } else {
        if state.rest.agent_mode() == AgentMode::Yolo {
            state.rest.set_agent_mode(AgentMode::Auto);
        }
        state.rest.fg_mut().status = "yolo disarmed".into();
    }
    refresh_security_state(state, None);
    Ok(())
}

/// Disarm YOLO when the daemon is stopped (any stop route: the Daemon checkbox, the stop
/// handler, or the stop portion of a restart). Keeps "YOLO requires a running daemon" true:
/// clears `rest.yolo_armed`, and if currently sitting in `Yolo` agent mode, falls back to
/// `Auto` so the bypass turns off the instant the daemon goes down.
fn disarm_yolo_for_stop(state: &mut AppState) {
    if state.rest.yolo_armed {
        state.rest.yolo_armed = false;
        if state.rest.agent_mode() == AgentMode::Yolo {
            state.rest.set_agent_mode(AgentMode::Auto);
        }
    }
}

/// Handle `Action::SecurityRestart`: stop then start the daemon.
///
/// Keeps `security_enabled = true` so sec_ tools remain advertised after restart. The
/// stop portion of a restart still enforces the YOLO invariant — restarting disarms YOLO
/// (acceptable: the user re-arms after the daemon is back up).
pub(super) fn handle_security_restart(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = true;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.restart(state.rest.sec_token.clone());
    }
    disarm_yolo_for_stop(state);
    state.rest.fg_mut().status = "security: restarting daemon…".into();
    refresh_security_state(state, None);
    Ok(())
}

/// What `Action::SecurityToggleTool` resolved the selected row to, lifted out of the
/// panel before any `rest` mutation so the borrow is released.
enum ToggleTarget {
    /// The daemon checkbox: start the daemon if stopped, stop it if running.
    Daemon,
    /// The YOLO checkbox: arm/disarm (gated on the daemon running).
    Yolo,
}

/// Handle `Action::SecurityToggleTool`: toggle whatever row is selected in the tools
/// pane (daemon or YOLO checkboxes only — concrete tools live in `sec_catalog`).
///
/// - `SecSel::Daemon` → toggle the daemon: STOP it when running, START it when stopped,
///   reusing the existing stop/start handlers (so the stop path also enforces the
///   YOLO-disarm invariant).
/// - `SecSel::Yolo` → flip the Layer-1 YOLO arm flag (the checkbox), GATED on the daemon
///   running: refused (no state change) when the daemon is stopped. When running, arming
///   merely UNLOCKS the `Yolo` agent mode (the user still switches in via `/mode yolo` /
///   Shift+Tab); disarming while currently in `Yolo` drops `agent_mode` back to `Auto` so
///   the harness-bypass can never outlive its arm.
///
/// A no-op when nothing is selected. Refreshes the panel after.
pub(super) fn handle_security_toggle_tool(state: &mut AppState) -> Result<()> {
    let target = if let Mode::Security(s) = state.mode() {
        match s.selected_sec() {
            Some(SecSel::Daemon) => Some(ToggleTarget::Daemon),
            Some(SecSel::Yolo) => Some(ToggleTarget::Yolo),
            None => None,
        }
    } else {
        None
    };

    match target {
        Some(ToggleTarget::Daemon) => {
            let running = state
                .rest
                .sec_manager
                .as_ref()
                .map(|m| m.status().running)
                .unwrap_or(false);
            if running {
                handle_security_stop(state)?;
            } else {
                handle_security_start(state)?;
            }
            return Ok(());
        }
        Some(ToggleTarget::Yolo) => {
            let running = state
                .rest
                .sec_manager
                .as_ref()
                .map(|m| m.status().running)
                .unwrap_or(false);
            if !running {
                state.rest.fg_mut().status = "yolo locked — start the daemon first".into();
            } else {
                handle_set_yolo_armed(state, !state.rest.yolo_armed)?;
            }
        }
        None => {}
    }
    refresh_security_state(state, None);
    Ok(())
}

/// Handle `Action::SecurityToggleDomain`: no per-tool grid on the dashboard.
///
/// Domain enable/disable is not exposed from the dashboard (tools are catalog-driven).
/// Kept as a key binding for compatibility; reports a short status.
pub(super) fn handle_security_toggle_domain(state: &mut AppState) -> Result<()> {
    state.rest.fg_mut().status =
        "security: domain toggle is catalog-driven (use sec_catalog / Images pane)".into();
    refresh_security_state(state, None);
    Ok(())
}

/// Handle `Action::SecurityInstall(key)`: install/repair one dependency by manifest
/// key, then re-fetch install-health so the dependency pane's present-flags update.
///
/// v1 is BLOCKING: a Tier-2 download can take seconds, and async progress would need
/// the deferred streaming protocol (not built yet). The status line reports the
/// daemon's message (or the failure). After the install we re-fetch `health()` ONCE
/// and seed it into the open panel via `refresh_security_state` — this is the ONLY
/// refresh path that pays the heavy health round-trip; the lifecycle refreshes carry
/// the existing health untouched. A missing/unparseable health response leaves the
/// previous list in place (we pass `None`), so the pane never blanks on a transient
/// IPC hiccup.
pub(super) fn handle_security_install(key: String, state: &mut AppState) -> Result<()> {
    let fresh: Option<Vec<InstallHealthEntry>> = if let Some(m) = state.rest.sec_manager.as_ref() {
        // Capture the install outcome + re-probe BEFORE writing status: `m` holds an
        // immutable borrow of `state.rest`, and the per-session status write (C6) needs a
        // mutable `fg_mut()` borrow — so resolve everything that touches `m` first, drop
        // the borrow, then update the foreground session's status.
        let install_status = match m.install(&key) {
            Ok(msg) => format!("security: {msg}"),
            Err(e) => format!("security: install '{key}' failed: {e}"),
        };
        // Re-probe install-health regardless of the install's outcome — even a failed
        // install may have changed what is present, and a success certainly did.
        let health = m.health().ok();
        state.rest.fg_mut().status = install_status;
        health
    } else {
        state.rest.fg_mut().status = format!("security: install '{key}' failed: no daemon");
        None
    };
    refresh_security_state(state, fresh);
    Ok(())
}

/// Re-read the live daemon status from the manager and refresh the open
/// `Mode::Security` state so the panel updates immediately. If the mode is
/// not `Security` (the action was dispatched from somewhere else), this is a
/// no-op — the panel will pick up fresh status the next time it opens.
///
/// `fresh_health` is the install path's freshly-probed install-health:
/// - `None` (the lifecycle refreshes) → `install_health` is CARRIED untouched, so no
///   extra (heavy) `health()` IPC call is made.
/// - `Some(list)` (the install path) → seed the panel's `install_health` with the new
///   list so the dependency pane's present-flags reflect the install.
fn refresh_security_state(state: &mut AppState, fresh_health: Option<Vec<InstallHealthEntry>>) {
    let status = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| {
            let base = m.status();
            m.enrich_status(base)
        })
        .unwrap_or_default();
    let inactive = state.rest.sec_inactive.clone();
    let yolo_armed = state.rest.yolo_armed;
    let lane_label = state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|x| x.settings.cyber_lane)
        .map(|l| l.label().to_string())
        .unwrap_or_else(|| "none".into());
    if let Mode::Security(s) = state.mode_mut() {
        // `refresh` preserves `install_health`; overwrite it only when the install path
        // handed us a fresh probe. `refresh` re-clamps both cursors afterwards.
        if let Some(health) = fresh_health {
            s.install_health = health;
        }
        s.lane_label = lane_label;
        s.refresh(status, inactive, yolo_armed);
    }
}

#[cfg(test)]
mod security_baseline_tests {
    use super::*;

    #[test]
    fn yolo_cannot_be_armed_without_a_running_daemon() {
        let mut state = AppState::new(Mode::Chat);
        assert!(handle_set_yolo_armed(&mut state, true).is_err());
        assert!(!state.rest.yolo_armed);
    }

    /// The panel's YOLO row used to flip `yolo_armed` inline, so disarming from
    /// the panel left `agent_mode` on `Yolo` while `/yolo` and the checkbox
    /// dropped to `Auto`. Both paths must leave Yolo.
    #[test]
    fn disarming_yolo_drops_out_of_yolo_mode() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.yolo_armed = true;
        state.rest.set_agent_mode(AgentMode::Yolo);
        handle_set_yolo_armed(&mut state, false).unwrap();
        assert!(!state.rest.yolo_armed);
        assert_eq!(state.rest.agent_mode(), AgentMode::Auto);
    }

    #[test]
    fn stopping_security_disarms_yolo_and_leaves_yolo_mode() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.security_enabled = true;
        state.rest.yolo_armed = true;
        state.rest.set_agent_mode(AgentMode::Yolo);
        handle_security_stop(&mut state).unwrap();
        assert!(!state.rest.security_enabled);
        assert!(!state.rest.yolo_armed);
        assert_eq!(state.rest.agent_mode(), AgentMode::Auto);
    }
}

/// What `apply_lane` did, for the status line and the IPC reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaneOutcome {
    Set(crate::app::sec::lane::Lane),
    Cleared,
    /// Refused, with the reason a user can act on.
    Refused(&'static str),
}

/// Set or clear this session's cyber lane.
///
/// The lane is per-session (`Settings::cyber_lane`) while the ROLE assignment stays
/// global, so two windows can run different lanes at once. Persisted immediately:
/// a lane that vanished on restart would leave a session silently ungated while
/// still advertising the toolkit.
///
/// Fail-closed at set time: a lane whose role has no model is refused here rather
/// than accepted and then refusing every call. That is the difference between a
/// setting that looks applied and one that works.
pub(crate) fn apply_lane(
    state: &mut AppState,
    sess_idx: usize,
    lane: Option<crate::app::sec::lane::Lane>,
) -> Result<LaneOutcome> {
    let Some(mut session) = state.rest.sessions[sess_idx].session.take() else {
        return Ok(LaneOutcome::Refused("no session"));
    };
    let outcome = match lane {
        None => {
            session.settings.cyber_lane = None;
            LaneOutcome::Cleared
        }
        Some(l) => {
            let resolves =
                crate::app::resolve::resolve_role(&state.rest.config, &session.settings, l.role())
                    .is_some();
            if resolves {
                session.settings.cyber_lane = Some(l);
                LaneOutcome::Set(l)
            } else {
                LaneOutcome::Refused(crate::app::sec::active::Inactive::NoModel.reason())
            }
        }
    };
    if matches!(outcome, LaneOutcome::Set(_) | LaneOutcome::Cleared) {
        // Lane change invalidates pins that may not be legal on the new surface.
        let mut ws = crate::app::sec::working_set::load(&session.path);
        ws.clear_pins();
        if let Some(l) = lane {
            ws.set_lane(l.label());
        } else {
            ws.set_lane("");
        }
        let _ = crate::app::sec::working_set::store(&session.path, &ws);
        session.save()?;
    }
    state.rest.sessions[sess_idx].session = Some(session);
    Ok(outcome)
}

/// `l` in the `/security` panel: set or clear this session's lane.
pub(super) fn handle_security_lane(
    state: &mut AppState,
    sess_idx: usize,
    lane: Option<crate::app::sec::lane::Lane>,
) -> Result<()> {
    let status = match apply_lane(state, sess_idx, lane)? {
        LaneOutcome::Set(l) => format!("security: lane = {}", l.label()),
        LaneOutcome::Cleared => "security: lane cleared".to_string(),
        LaneOutcome::Refused(why) => format!("security: lane not set — {why}"),
    };
    state.rest.fg_mut().status = status;
    refresh_security_state(state, None);
    Ok(())
}

/// `a` — enable every advertised tool (clear the inactive set).
pub(super) fn handle_security_enable_all(state: &mut AppState) -> Result<()> {
    state.rest.sec_inactive.clear();
    state.rest.fg_mut().status = "security: all tools enabled".into();
    refresh_security_state(state, None);
    Ok(())
}

/// `n` — disable every currently advertised tool.
pub(super) fn handle_security_disable_all(state: &mut AppState) -> Result<()> {
    let names: Vec<String> = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status().tools.into_iter().map(|t| t.name).collect())
        .unwrap_or_default();
    for n in &names {
        state.rest.sec_inactive.insert(n.clone());
    }
    state.rest.fg_mut().status = format!("security: {} tools disabled", names.len());
    refresh_security_state(state, None);
    Ok(())
}

/// `u` — re-extract the embedded daemon package and restart so a stale wire
/// vocabulary (bool `risk`) is replaced without a full `--force` pip rebuild.
pub(super) fn handle_security_refresh_toolkit(state: &mut AppState) -> Result<()> {
    match crate::security::refresh_daemon_sources() {
        Ok(()) => {
            state.rest.fg_mut().status = "security: toolkit sources refreshed — restarting…".into();
            // Keep security_enabled; restart disarms YOLO by design.
            handle_security_restart(state)?;
            // Toolkit refresh can change what the host runtime looks like to the panel.
            crate::security::runtime_status_kick(true);
        }
        Err(e) => {
            state.rest.fg_mut().status = format!("security: refresh failed: {e}");
        }
    }
    Ok(())
}

/// `p` on Images — pull the selected composition (blocking, like install).
pub(super) fn handle_security_image_pull(id: String, state: &mut AppState) -> Result<()> {
    state.rest.fg_mut().status = format!("security: pulling image '{id}'…");
    if let Mode::Security(s) = state.mode_mut() {
        s.images_busy = true;
    }
    let outcome = match crate::security::pull_composition(&id, false) {
        Ok(()) => format!("security: pulled '{id}'"),
        Err(e) => format!("security: pull '{id}' failed: {e}"),
    };
    state.rest.fg_mut().status = outcome;
    if let Mode::Security(s) = state.mode_mut() {
        s.images_busy = false;
        s.refresh_images();
    }
    refresh_security_state(state, None);
    Ok(())
}

/// Enter/`a` on Images — activate a previously pulled composition and restart
/// the daemon so `KOMA_SEC_IMAGE` / `KOMA_SEC_COMPOSITION` are re-read.
pub(super) fn handle_security_image_activate(id: String, state: &mut AppState) -> Result<()> {
    match crate::security::activate_composition(&id) {
        Ok(()) => {
            // Reuse the stop+start path so the child picks up the new pin.
            handle_security_restart(state)?;
            state.rest.fg_mut().status = format!("security: activated '{id}'; daemon restarted");
        }
        Err(e) => {
            state.rest.fg_mut().status = format!("security: activate '{id}' failed: {e}");
        }
    }
    if let Mode::Security(s) = state.mode_mut() {
        s.refresh_images();
    }
    refresh_security_state(state, None);
    Ok(())
}

/// `x`/Delete on Images — drop a non-active local composition.
pub(super) fn handle_security_image_remove(id: String, state: &mut AppState) -> Result<()> {
    let outcome = match crate::security::remove_composition(&id) {
        Ok(()) => format!("security: removed '{id}'"),
        Err(e) => format!("security: remove '{id}' failed: {e}"),
    };
    state.rest.fg_mut().status = outcome;
    if let Mode::Security(s) = state.mode_mut() {
        s.refresh_images();
    }
    refresh_security_state(state, None);
    Ok(())
}

/// `r` on Images — rebuild the composition list from inventory.
pub(super) fn handle_security_image_refresh(state: &mut AppState) -> Result<()> {
    if let Mode::Security(s) = state.mode_mut() {
        s.refresh_images();
    }
    state.rest.fg_mut().status = "security: images list refreshed".into();
    refresh_security_state(state, None);
    Ok(())
}

/// `c` — try to bring Docker up via Colima when present; otherwise surface a hint.
pub(super) fn handle_security_ensure_runtime(state: &mut AppState) -> Result<()> {
    // Force a blocking probe: the user asked for a runtime check/start.
    let (ok, label) = crate::security::runtime_status_refresh(true);
    if ok {
        state.rest.fg_mut().status = format!("security: runtime ok — {label}");
        refresh_security_state(state, None);
        return Ok(());
    }
    // Prefer Colima when installed (mac path). Non-interactive: just run it.
    let colima = std::process::Command::new("colima")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if colima {
        state.rest.fg_mut().status = "security: starting colima…".into();
        match std::process::Command::new("colima").arg("start").output() {
            Ok(out) if out.status.success() => {
                let (_, label) = crate::security::runtime_status_refresh(true);
                state.rest.fg_mut().status = format!("security: colima started — {label}");
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                let tail = err.trim();
                state.rest.fg_mut().status = format!(
                    "security: colima start failed{}",
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(": {tail}")
                    }
                );
                crate::security::runtime_status_kick(true);
            }
            Err(e) => {
                state.rest.fg_mut().status = format!("security: colima start failed: {e}");
                crate::security::runtime_status_kick(true);
            }
        }
    } else {
        let (_, label) = crate::security::runtime_status_refresh(true);
        state.rest.fg_mut().status =
            format!("security: {label} — open Docker / run koma --security-install");
    }
    refresh_security_state(state, None);
    Ok(())
}
