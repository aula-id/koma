//! Security mode state: the working state for the `/security` daemon control panel.
//!
//! A read-only status view (no sub-modes, no editor) with TWO toggleable body panes:
//! the Tool inventory (default) and a Dependency install-health list. Navigation moves
//! the cursor over whichever pane is active.

use crate::app::sec::{InstallHealthEntry, SecStatus};

/// Working state for the `/security` daemon control panel.
///
/// Holds a snapshot of the daemon status (refreshed on open + after each control key),
/// the per-dependency install-health list (fetched ONCE on open and after an install —
/// it is a heavy IPC round-trip, so it is carried, never re-fetched on a plain refresh),
/// and a cursor for each pane. No drafts, no editor — control panel only.
#[derive(Debug, Clone)]
pub struct SecurityState {
    /// Latest status snapshot from the daemon manager (or a default when no manager).
    pub status: SecStatus,
    /// Selected index into `status.tools` (the tool-inventory cursor).
    pub selected: usize,
    /// Tool names the user has disabled (the inactive set), mirrored from
    /// `state.rest.sec_inactive`. Empty = every tool active. The view dims a row whose
    /// name is in this set; the toggle actions flip membership on `state.rest` and
    /// refresh this copy.
    pub inactive: std::collections::HashSet<String>,
    /// Per-dependency install-health, fetched once on panel open (and re-fetched after
    /// an install). Empty when the daemon is stopped / no health data is available.
    pub install_health: Vec<InstallHealthEntry>,
    /// Which body pane is showing: `false` = tools (default), `true` = dependencies.
    pub health_view: bool,
    /// Selected index into `install_health` (the dependency-pane cursor).
    pub health_selected: usize,
}

impl SecurityState {
    /// Build from a live status snapshot + the disabled-tool set + the install-health
    /// list, both cursors at the top, showing the tools pane by default.
    pub fn new(
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        install_health: Vec<InstallHealthEntry>,
    ) -> Self {
        Self {
            status,
            selected: 0,
            inactive,
            install_health,
            health_view: false,
            health_selected: 0,
        }
    }

    /// Move the cursor up by one (saturating at 0) in the ACTIVE pane.
    pub fn move_up(&mut self) {
        if self.health_view {
            self.health_selected = self.health_selected.saturating_sub(1);
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Move the cursor down by one in the ACTIVE pane (clamped to that pane's last row).
    pub fn move_down(&mut self) {
        if self.health_view {
            let max = self.install_health.len().saturating_sub(1);
            if self.health_selected < max {
                self.health_selected += 1;
            }
        } else {
            let max = self.status.tools.len().saturating_sub(1);
            if self.selected < max {
                self.selected += 1;
            }
        }
    }

    /// Flip between the tools pane and the dependency pane, re-clamping the cursor of
    /// the pane that just became active so it points at a valid row.
    pub fn toggle_health_view(&mut self) {
        self.health_view = !self.health_view;
        self.clamp_cursors();
    }

    /// Replace the status snapshot + disabled-tool set (called after start/stop/restart
    /// or a tool/domain toggle to reflect the new state without re-opening the panel).
    ///
    /// PRESERVES `install_health` (carried from `&self`): re-fetching it is a heavy IPC
    /// round-trip, so a plain lifecycle refresh must not pay it. The install path seeds
    /// fresh health out-of-band (see `refresh_security_state`).
    pub fn refresh(&mut self, status: SecStatus, inactive: std::collections::HashSet<String>) {
        self.status = status;
        self.inactive = inactive;
        // `install_health` is intentionally untouched here — it is carried across refreshes.
        self.clamp_cursors();
    }

    /// Clamp BOTH pane cursors against their current row counts (called after any change
    /// that could shrink a list — daemon stopped → 0 tools, or a refreshed health list).
    fn clamp_cursors(&mut self) {
        let tool_max = self.status.tools.len().saturating_sub(1);
        if self.status.tools.is_empty() {
            self.selected = 0;
        } else if self.selected > tool_max {
            self.selected = tool_max;
        }
        let health_max = self.install_health.len().saturating_sub(1);
        if self.install_health.is_empty() {
            self.health_selected = 0;
        } else if self.health_selected > health_max {
            self.health_selected = health_max;
        }
    }
}
