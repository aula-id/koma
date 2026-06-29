//! Security mode state: the working state for the `/security` daemon control panel.
//!
//! A read-only status view (no sub-modes, no editor) showing daemon running/installed
//! flags plus the tool inventory. Navigation moves the cursor over the tool list.

use crate::app::sec::SecStatus;

/// Working state for the `/security` daemon control panel.
///
/// Holds a snapshot of the daemon status (refreshed on open + after each control key)
/// and the tool-list cursor. No drafts, no sub-modes — control panel only.
#[derive(Debug, Clone)]
pub struct SecurityState {
    /// Latest status snapshot from the daemon manager (or a default when no manager).
    pub status: SecStatus,
    /// Selected index into `status.tools` (the tool-inventory cursor).
    pub selected: usize,
}

impl SecurityState {
    /// Build from a live status snapshot, cursor at the top.
    pub fn new(status: SecStatus) -> Self {
        Self { status, selected: 0 }
    }

    /// Move the cursor up by one (saturating at 0).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the cursor down by one (clamped to the last tool index).
    pub fn move_down(&mut self) {
        let max = self.status.tools.len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }

    /// Replace the status snapshot (called after start/stop/restart to reflect
    /// the new daemon state without re-opening the panel).
    pub fn refresh(&mut self, status: SecStatus) {
        self.status = status;
        // Clamp the cursor in case the tool count shrank (daemon stopped → 0 tools).
        let max = self.status.tools.len().saturating_sub(1);
        if self.selected > max && !self.status.tools.is_empty() {
            self.selected = max;
        } else if self.status.tools.is_empty() {
            self.selected = 0;
        }
    }
}
