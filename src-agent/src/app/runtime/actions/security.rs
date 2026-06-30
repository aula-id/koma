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
    state.rest.status = "ready".into();
    Ok(())
}

/// Start the daemon. Called when the Daemon checkbox is toggled ON (from
/// `handle_security_toggle_tool`).
///
/// Sets `security_enabled` to true so subsequent turns advertise sec_ tools.
pub(super) fn handle_security_start(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = true;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.start(state.rest.sec_token.clone());
    }
    state.rest.status = "security: starting daemon…".into();
    refresh_security_state(state, None);
    Ok(())
}

/// Stop the daemon. Called when the Daemon checkbox is toggled OFF (from
/// `handle_security_toggle_tool`).
///
/// Clears `security_enabled` so sec_ tools are no longer advertised. ENFORCES the
/// "YOLO requires a running daemon" invariant: stopping the daemon always disarms YOLO
/// (and drops out of `Yolo` agent mode) so a harness-bypass can never outlive the daemon.
pub(super) fn handle_security_stop(state: &mut AppState) -> Result<()> {
    state.rest.security_enabled = false;
    if let Some(m) = state.rest.sec_manager.as_ref() {
        m.stop();
    }
    disarm_yolo_for_stop(state);
    state.rest.status = "security: daemon stopped".into();
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
        if state.rest.agent_mode == AgentMode::Yolo {
            state.rest.agent_mode = AgentMode::Auto;
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
    state.rest.status = "security: restarting daemon…".into();
    refresh_security_state(state, None);
    Ok(())
}

/// What `Action::SecurityToggleTool` resolved the selected row to, lifted out of the
/// panel before any `rest` mutation so the borrow is released. Mirrors [`SecSel`] but
/// carries the tool's resolved *name* (not its index) for the tool case.
enum ToggleTarget {
    /// The daemon checkbox: start the daemon if stopped, stop it if running.
    Daemon,
    /// The YOLO checkbox: arm/disarm (gated on the daemon running).
    Yolo,
    /// A tool row, resolved to its name.
    Tool(String),
}

/// Handle `Action::SecurityToggleTool`: toggle whatever row is selected in the tools
/// pane. The pane mixes the daemon checkbox (row 0) and the YOLO checkbox (row 1) with the
/// tool inventory, so this reads the selected [`SecSel`] (resolved through the panel's
/// render-ordered item list) and branches:
///
/// - `SecSel::Daemon` → toggle the daemon: STOP it when running, START it when stopped,
///   reusing the existing stop/start handlers (so the stop path also enforces the
///   YOLO-disarm invariant).
/// - `SecSel::Yolo` → flip the Layer-1 YOLO arm flag (the checkbox), GATED on the daemon
///   running: refused (no state change) when the daemon is stopped. When running, arming
///   merely UNLOCKS the `Yolo` agent mode (the user still switches in via `/mode yolo` /
///   Shift+Tab); disarming while currently in `Yolo` drops `agent_mode` back to `Auto` so
///   the harness-bypass can never outlive its arm.
/// - `SecSel::Tool(i)` → flip `status.tools[i]`'s membership in `state.rest.sec_inactive`
///   (active → disabled, disabled → active).
///
/// A no-op when nothing is selected. Refreshes the panel after.
pub(super) fn handle_security_toggle_tool(state: &mut AppState) -> Result<()> {
    // Resolve the selected row (and, for a tool, its name) out of the open panel BEFORE
    // mutating `rest` — `selected_sec()` walks the same render-ordered list the cursor and
    // view use, so this targets exactly the highlighted row.
    let target = if let Mode::Security(s) = state.mode() {
        match s.selected_sec() {
            Some(SecSel::Daemon) => Some(ToggleTarget::Daemon),
            Some(SecSel::Yolo) => Some(ToggleTarget::Yolo),
            Some(SecSel::Tool(i)) => s
                .status
                .tools
                .get(i)
                .map(|t| ToggleTarget::Tool(t.name.clone())),
            None => None,
        }
    } else {
        None
    };

    match target {
        // Daemon checkbox row: toggle the daemon via the existing start/stop handlers.
        // Branch on the LIVE manager running flag (authoritative), not the mode-state copy.
        // The stop handler enforces the YOLO-disarm invariant; both handlers refresh the
        // panel, so the trailing refresh below is just belt-and-suspenders.
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
        // YOLO checkbox row: arm/disarm — GATED on the daemon running.
        Some(ToggleTarget::Yolo) => {
            let running = state
                .rest
                .sec_manager
                .as_ref()
                .map(|m| m.status().running)
                .unwrap_or(false);
            if !running {
                // Daemon stopped: refuse the arm (YOLO requires a running daemon). Leave
                // `yolo_armed` untouched and tell the user what to do.
                state.rest.status = "yolo locked — start the daemon first".into();
            } else {
                state.rest.yolo_armed = !state.rest.yolo_armed;
                if state.rest.yolo_armed {
                    state.rest.status = "yolo armed — switch with /mode yolo or Shift+Tab".into();
                } else {
                    // Disarmed: if we're sitting in Yolo right now, fall straight back to
                    // Auto so the bypass turns off the instant it's disarmed.
                    if state.rest.agent_mode == AgentMode::Yolo {
                        state.rest.agent_mode = AgentMode::Auto;
                    }
                    state.rest.status = "yolo disarmed".into();
                }
            }
        }
        // Tool row: flip its active state.
        Some(ToggleTarget::Tool(name)) => {
            if state.rest.sec_inactive.contains(&name) {
                state.rest.sec_inactive.remove(&name);
                state.rest.status = format!("security: {name} enabled");
            } else {
                state.rest.sec_inactive.insert(name.clone());
                state.rest.status = format!("security: {name} disabled");
            }
        }
        // Tool row whose index didn't resolve (stale cursor), or nothing selected: no-op.
        None => {}
    }
    refresh_security_state(state, None);
    Ok(())
}

/// Handle `Action::SecurityToggleDomain`: toggle every tool sharing the selected
/// tool's domain.
///
/// If ALL tools in that domain are currently active, disable them all; otherwise (any
/// already disabled) enable them all. Resolves the selected row through the panel's
/// render-ordered item list and acts ONLY on a `SecSel::Tool(i)` (using `status.tools[i]`
/// for the domain); when the daemon or YOLO checkbox is selected it has no domain, so this
/// is a no-op with an explanatory status. Refreshes the panel after.
pub(super) fn handle_security_toggle_domain(state: &mut AppState) -> Result<()> {
    // Resolve the selected tool's domain + every tool name in it out of the panel. Only a
    // tool row has a domain; the daemon and YOLO checkboxes resolve to `None` here.
    let domain_tools: Option<(String, Vec<String>)> = if let Mode::Security(s) = state.mode() {
        match s.selected_sec() {
            Some(SecSel::Tool(i)) => s.status.tools.get(i).map(|sel| {
                let domain = sel.domain.clone();
                let names = s
                    .status
                    .tools
                    .iter()
                    .filter(|t| t.domain == domain)
                    .map(|t| t.name.clone())
                    .collect::<Vec<_>>();
                (domain, names)
            }),
            // Daemon/YOLO checkbox (or nothing) selected: no domain to toggle.
            _ => None,
        }
    } else {
        None
    };
    if let Some((domain, names)) = domain_tools {
        // Disable the whole domain only when every member is currently active;
        // otherwise enable all (so a mixed/partly-disabled domain flips fully on).
        let all_active = names.iter().all(|n| !state.rest.sec_inactive.contains(n));
        if all_active {
            for n in &names {
                state.rest.sec_inactive.insert(n.clone());
            }
            state.rest.status = format!("security: domain [{domain}] disabled");
        } else {
            for n in &names {
                state.rest.sec_inactive.remove(n);
            }
            state.rest.status = format!("security: domain [{domain}] enabled");
        }
    } else {
        // No tool row selected (the YOLO checkbox has no domain).
        state.rest.status = "security: no domain selected".into();
    }
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
        match m.install(&key) {
            Ok(msg) => state.rest.status = format!("security: {msg}"),
            Err(e) => state.rest.status = format!("security: install '{key}' failed: {e}"),
        }
        // Re-probe install-health regardless of the install's outcome — even a failed
        // install may have changed what is present, and a success certainly did.
        m.health().ok()
    } else {
        state.rest.status = format!("security: install '{key}' failed: no daemon");
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
        .map(|m| m.status())
        .unwrap_or_default();
    let inactive = state.rest.sec_inactive.clone();
    let yolo_armed = state.rest.yolo_armed;
    if let Mode::Security(s) = state.mode_mut() {
        // `refresh` preserves `install_health`; overwrite it only when the install path
        // handed us a fresh probe. `refresh` re-clamps both cursors afterwards.
        if let Some(health) = fresh_health {
            s.install_health = health;
        }
        s.refresh(status, inactive, yolo_armed);
    }
}
