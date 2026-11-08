//! Security mode state: the working state for the `/security` daemon control panel.
//!
//! A control view (no editor) with FOUR body panes: the isolation **dashboard**
//! (default: daemon + YOLO + runtime summary), **Images** (guest compositions),
//! Dependency install-health, and Refused-descriptors. Tools are loaded dynamically
//! via sec_catalog/sec_run — this panel is not a per-tool enable grid.

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
    /// Legacy tool-row index (no longer navigable on the dashboard; kept for IPC).
    Tool(usize),
}

/// Which body pane the `/security` panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecPane {
    /// Isolation dashboard (default): daemon, YOLO, runtime/catalog summary.
    #[default]
    Tools,
    /// Guest composition images (pull / activate / remove).
    Images,
    /// Dependency install-health.
    Deps,
    /// Handshake refusals (malformed / ungated descriptors).
    Refused,
}

impl SecPane {
    /// Cycle Tools → Images → Deps → Refused → Tools.
    pub fn next(self) -> Self {
        match self {
            SecPane::Tools => SecPane::Images,
            SecPane::Images => SecPane::Deps,
            SecPane::Deps => SecPane::Refused,
            SecPane::Refused => SecPane::Tools,
        }
    }
}

/// One row in the Images pane (known composition + local inventory).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SafetyImageRow {
    /// Composition id (e.g. `minimal`, `recon`).
    pub id: String,
    /// Short UI blurb from the static catalogue.
    pub blurb: String,
    /// Full docker tag for this composition.
    pub tag: String,
    /// Human-readable size (`1.2G`, `—`, …).
    pub size_label: String,
    /// Present in local `images.json` with a non-empty digest.
    pub local: bool,
    /// Currently active guest composition.
    pub active: bool,
    /// Compact status token: `active` / `local` / `remote` (or a busy note).
    #[serde(default)]
    pub status: String,
}

/// Working state for the `/security` daemon control panel.
///
/// Holds a snapshot of the daemon status (refreshed on open + after each control key),
/// the per-dependency install-health list (fetched ONCE on open and after an install —
/// it is a heavy IPC round-trip, so it is carried, never re-fetched on a plain refresh),
/// the guest-image composition list, and a cursor for each pane. No drafts, no editor —
/// control panel only.
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
    /// Known guest compositions + local inventory for the Images pane.
    pub images: Vec<SafetyImageRow>,
    /// Images-pane cursor: index into [`Self::images`].
    pub images_selected: usize,
    /// `true` while a long-running image op is in flight (optional; pull is blocking in v1).
    pub images_busy: bool,
    /// Active body pane.
    pub pane: SecPane,
    /// Dependency-pane cursor: an index into [`SecurityState::health_items`] (the
    /// install-health rows in tier-grouped RENDER order), NOT into the flat
    /// `install_health` vector — same nav/render-sync trick as `selected`.
    pub health_selected: usize,
    /// Refused-pane cursor: index into `status.refused`.
    pub refused_selected: usize,
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
    /// Session cyber-lane label for the status strip (`research` / `red` / `blue` / `none`).
    pub lane_label: String,
}

impl SecurityState {
    /// Build from a live status snapshot, the disabled-tool set, the YOLO arm flag, and
    /// the install-health list, both cursors at the top, showing the tools pane by
    /// default. Seeds the Images list from disk inventory.
    pub fn new(
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
        install_health: Vec<InstallHealthEntry>,
    ) -> Self {
        let mut st = Self {
            status,
            selected: 0,
            inactive,
            yolo_armed,
            install_health,
            images: Vec::new(),
            images_selected: 0,
            images_busy: false,
            pane: SecPane::Tools,
            health_selected: 0,
            refused_selected: 0,
            health_fetching: false,
            health_frame: 0,
            lane_label: "none".into(),
        };
        st.refresh_images();
        st
    }

    /// Compatibility: true when the deps pane is showing (old `health_view` bool).
    pub fn health_view(&self) -> bool {
        self.pane == SecPane::Deps
    }

    /// Rebuild [`Self::images`] from the static catalogue + `images.json` inventory.
    ///
    /// Size labels come from inventory `size_bytes` when present (recorded at pull time).
    pub fn refresh_images(&mut self) {
        let inv = crate::security::read_images_inventory();
        let version = env!("CARGO_PKG_VERSION");
        // Stale inventory from another agent version: treat nothing as local/active.
        let version_ok = inv.version.is_empty() || inv.version == version;
        let mut rows = Vec::with_capacity(crate::security::list_known_compositions().len());
        for (id, blurb) in crate::security::list_known_compositions() {
            let tag = crate::security::composition_tag(id);
            let entry = if version_ok {
                inv.images.get(*id)
            } else {
                None
            };
            let local = entry.map(|e| !e.digest.is_empty()).unwrap_or(false);
            let active = version_ok && inv.active == *id && local;
            let size_label = entry
                .map(|e| format_image_size(e.size_bytes))
                .unwrap_or_else(|| "—".into());
            let status = if active {
                "active".into()
            } else if local {
                "local".into()
            } else {
                "remote".into()
            };
            rows.push(SafetyImageRow {
                id: (*id).to_string(),
                blurb: (*blurb).to_string(),
                tag,
                size_label,
                local,
                active,
                status,
            });
        }
        self.images = rows;
        self.clamp_cursors();
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
        // Dashboard only: daemon + YOLO. Concrete tools are dynamic (sec_catalog);
        // they are summarized below the controls, not toggled here.
        vec![SecSel::Daemon, SecSel::Yolo]
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

    /// Currently selected Images row, if any.
    pub fn selected_image(&self) -> Option<&SafetyImageRow> {
        self.images.get(self.images_selected)
    }

    /// Move the cursor up by one (saturating at 0) in the ACTIVE pane.
    pub fn move_up(&mut self) {
        match self.pane {
            SecPane::Deps => {
                self.health_selected = self.health_selected.saturating_sub(1);
            }
            SecPane::Refused => {
                self.refused_selected = self.refused_selected.saturating_sub(1);
            }
            SecPane::Images => {
                self.images_selected = self.images_selected.saturating_sub(1);
            }
            SecPane::Tools => {
                self.selected = self.selected.saturating_sub(1);
            }
        }
    }

    /// Move the cursor down by one in the ACTIVE pane, clamped to that pane's last row.
    pub fn move_down(&mut self) {
        match self.pane {
            SecPane::Deps => {
                let max = self.health_items().len().saturating_sub(1);
                if self.health_selected < max {
                    self.health_selected += 1;
                }
            }
            SecPane::Refused => {
                let max = self.status.refused.len().saturating_sub(1);
                if self.refused_selected < max {
                    self.refused_selected += 1;
                }
            }
            SecPane::Images => {
                let max = self.images.len().saturating_sub(1);
                if self.images_selected < max {
                    self.images_selected += 1;
                }
            }
            SecPane::Tools => {
                let max = self.tool_items().len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                }
            }
        }
    }

    /// Cycle Tools → Images → Deps → Refused → Tools, re-clamping the newly active cursor.
    pub fn cycle_pane(&mut self) {
        self.pane = self.pane.next();
        self.clamp_cursors();
    }

    /// Back-compat alias for [`Self::cycle_pane`].
    #[allow(dead_code)]
    pub fn toggle_health_view(&mut self) {
        self.cycle_pane();
    }

    /// Replace the status snapshot + disabled-tool set + YOLO arm flag (called after
    /// start/stop/restart, a tool/domain toggle, or the YOLO arm toggle to reflect the
    /// new state without re-opening the panel).
    ///
    /// PRESERVES `install_health` (carried from `&self`): re-fetching it is a heavy IPC
    /// round-trip, so a plain lifecycle refresh must not pay it. The install path seeds
    /// fresh health out-of-band (see `refresh_security_state`).
    ///
    /// PRESERVES `images` the same way — image ops call [`Self::refresh_images`] explicitly.
    pub fn refresh(
        &mut self,
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
    ) {
        self.status = status;
        self.inactive = inactive;
        self.yolo_armed = yolo_armed;
        // `install_health` / `images` intentionally untouched here.
        self.clamp_cursors();
    }

    /// Clamp ALL pane cursors against their current navigable-row counts.
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
        let refused_len = self.status.refused.len();
        if refused_len == 0 {
            self.refused_selected = 0;
        } else if self.refused_selected >= refused_len {
            self.refused_selected = refused_len - 1;
        }
        let images_len = self.images.len();
        if images_len == 0 {
            self.images_selected = 0;
        } else if self.images_selected >= images_len {
            self.images_selected = images_len - 1;
        }
    }
}

fn format_image_size(bytes: u64) -> String {
    if bytes == 0 {
        return "—".into();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1}G", b / GB)
    } else if b >= MB {
        format!("{:.0}M", b / MB)
    } else if b >= KB {
        format!("{:.0}K", b / KB)
    } else {
        format!("{bytes}B")
    }
}
