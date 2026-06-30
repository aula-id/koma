//! Security mode state: the working state for the `/security` daemon control panel.
//!
//! A read-only status view (no sub-modes, no editor) with TWO toggleable body panes:
//! the Tool inventory (default) and a Dependency install-health list. Navigation moves
//! the cursor over whichever pane is active.

use crate::app::sec::{InstallHealthEntry, SecStatus};

/// One navigable row in the TOOLS pane.
///
/// The pane is a single flat list the cursor walks, but it mixes three kinds of row: the
/// daemon-running checkbox at the very top, then the YOLO-arm checkbox, then the tool
/// inventory grouped by domain. This enum names which kind a given cursor position is so
/// render and nav agree on the layout. `Tool(usize)` carries an index into `status.tools`
/// (the flat storage vector), NOT a position in the rendered list — the position is the
/// index into [`SecurityState::tool_items`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecSel {
    /// The daemon start/stop checkbox row (always rendered first). Toggling it starts the
    /// daemon when stopped, stops it when running — folding the old s/x/t keybinds into a
    /// navigable checkbox.
    Daemon,
    /// The YOLO-arm checkbox row (rendered second, after the daemon checkbox).
    Yolo,
    /// A tool row; the `usize` indexes into `status.tools`.
    Tool(usize),
}

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
    /// Tool-pane cursor: an index into [`SecurityState::tool_items`] (the navigable rows
    /// in RENDER order — the daemon checkbox, then the YOLO checkbox, then tools grouped by
    /// domain), NOT into the flat `status.tools` vector. This is what keeps nav order ==
    /// render order: both
    /// the renderer and the move/clamp logic walk the same `tool_items()` list, so the
    /// highlight always lands on the row visually below/above.
    pub selected: usize,
    /// Tool names the user has disabled (the inactive set), mirrored from
    /// `state.rest.sec_inactive`. Empty = every tool active. The view dims a row whose
    /// name is in this set; the toggle actions flip membership on `state.rest` and
    /// refresh this copy.
    pub inactive: std::collections::HashSet<String>,
    /// YOLO-mode arm flag, mirrored from `state.rest.yolo_armed` (like `inactive`).
    /// The panel renders the YOLO checkbox row from this; toggling that row (Space/Enter
    /// while it is selected) flips `state.rest.yolo_armed` and refreshes this copy.
    pub yolo_armed: bool,
    /// Per-dependency install-health, fetched once on panel open (and re-fetched after
    /// an install). Empty when the daemon is stopped / no health data is available.
    pub install_health: Vec<InstallHealthEntry>,
    /// Which body pane is showing: `false` = tools (default), `true` = dependencies.
    pub health_view: bool,
    /// Dependency-pane cursor: an index into [`SecurityState::health_items`] (the
    /// install-health rows in tier-grouped RENDER order), NOT into the flat
    /// `install_health` vector — same nav/render-sync trick as `selected`.
    pub health_selected: usize,
    /// `true` while a NON-BLOCKING health probe is in flight (the receiver lives in
    /// `state.rest.sec_health_rx`). Drives the loading spinner on the daemon info line;
    /// cleared by `service_global` when the probe lands. Projected to the thin client so
    /// it shows the same "checking dependencies…" state.
    pub health_fetching: bool,
    /// Braille spinner frame counter for the in-flight health probe, advanced each tick
    /// by `service_global` while `state.rest.sec_health_rx` is `Some`. Projected so the
    /// client animates the spinner from this counter (it owns no manager / receiver of
    /// its own).
    pub health_frame: u64,
}

impl SecurityState {
    /// Build from a live status snapshot, the disabled-tool set, the YOLO arm flag, and
    /// the install-health list, both cursors at the top, showing the tools pane by
    /// default.
    pub fn new(
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
        install_health: Vec<InstallHealthEntry>,
    ) -> Self {
        Self {
            status,
            selected: 0,
            inactive,
            yolo_armed,
            install_health,
            health_view: false,
            health_selected: 0,
            health_fetching: false,
            health_frame: 0,
        }
    }

    /// The navigable rows of the TOOLS pane, IN RENDER ORDER — the single source of truth
    /// shared by the renderer and the cursor logic (so nav order can never desync from
    /// what is on screen).
    ///
    /// Order: the daemon checkbox first (`SecSel::Daemon`), then the YOLO checkbox
    /// (`SecSel::Yolo`), then the tools grouped by domain. Domains are taken in first-seen
    /// (insertion) order, exactly as the renderer groups them; within each domain the tools
    /// keep their `status.tools` iteration order. The returned `usize` in each
    /// `SecSel::Tool(i)` indexes back into `status.tools`.
    pub fn tool_items(&self) -> Vec<SecSel> {
        let mut items = vec![SecSel::Daemon, SecSel::Yolo];
        // Distinct domains in first-seen order (matches the renderer's grouping).
        let mut domains: Vec<&str> = Vec::new();
        for t in &self.status.tools {
            if !domains.iter().any(|d| *d == t.domain) {
                domains.push(&t.domain);
            }
        }
        for domain in &domains {
            for (i, t) in self.status.tools.iter().enumerate() {
                if t.domain == *domain {
                    items.push(SecSel::Tool(i));
                }
            }
        }
        items
    }

    /// The navigable rows of the DEPENDENCIES pane, IN RENDER ORDER: indices into
    /// `install_health`, grouped by tier (distinct tiers ascending, then each entry of
    /// that tier in `install_health` order) — matching `render_deps`. The dependency
    /// counterpart of [`SecurityState::tool_items`].
    pub fn health_items(&self) -> Vec<usize> {
        let mut tiers: Vec<u8> = Vec::new();
        for e in &self.install_health {
            if !tiers.contains(&e.tier) {
                tiers.push(e.tier);
            }
        }
        tiers.sort_unstable();
        let mut items: Vec<usize> = Vec::new();
        for tier in &tiers {
            for (i, e) in self.install_health.iter().enumerate() {
                if e.tier == *tier {
                    items.push(i);
                }
            }
        }
        items
    }

    /// The currently-selected tool-pane row, resolved through `tool_items()` so it always
    /// reflects render order. `None` only when the list is somehow empty (it never is —
    /// the daemon + YOLO rows are always present — but the bounds-checked `get` keeps it
    /// total).
    pub fn selected_sec(&self) -> Option<SecSel> {
        self.tool_items().get(self.selected).copied()
    }

    /// Move the cursor up by one (saturating at 0) in the ACTIVE pane. `selected` /
    /// `health_selected` are positions in the respective `*_items()` list, so plain
    /// saturating decrement walks the rendered rows.
    pub fn move_up(&mut self) {
        if self.health_view {
            self.health_selected = self.health_selected.saturating_sub(1);
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Move the cursor down by one in the ACTIVE pane, clamped to that pane's last
    /// navigable row (the last index into the active `*_items()` list).
    pub fn move_down(&mut self) {
        if self.health_view {
            let max = self.health_items().len().saturating_sub(1);
            if self.health_selected < max {
                self.health_selected += 1;
            }
        } else {
            let max = self.tool_items().len().saturating_sub(1);
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

    /// Replace the status snapshot + disabled-tool set + YOLO arm flag (called after
    /// start/stop/restart, a tool/domain toggle, or the YOLO arm toggle to reflect the
    /// new state without re-opening the panel).
    ///
    /// PRESERVES `install_health` (carried from `&self`): re-fetching it is a heavy IPC
    /// round-trip, so a plain lifecycle refresh must not pay it. The install path seeds
    /// fresh health out-of-band (see `refresh_security_state`).
    pub fn refresh(
        &mut self,
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
    ) {
        self.status = status;
        self.inactive = inactive;
        self.yolo_armed = yolo_armed;
        // `install_health` is intentionally untouched here — it is carried across refreshes.
        self.clamp_cursors();
    }

    /// Clamp BOTH pane cursors against their current navigable-row counts (called after
    /// any change that could shrink a list — daemon stopped → tools vanish, or a refreshed
    /// health list). Cursors index the `*_items()` lists, so clamp against those, not the
    /// raw vectors.
    fn clamp_cursors(&mut self) {
        let tool_len = self.tool_items().len();
        if tool_len == 0 {
            self.selected = 0;
        } else if self.selected >= tool_len {
            self.selected = tool_len - 1;
        }
        let health_len = self.health_items().len();
        if health_len == 0 {
            self.health_selected = 0;
        } else if self.health_selected >= health_len {
            self.health_selected = health_len - 1;
        }
    }
}
