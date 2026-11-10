//! Working state for the `/security` control panel.
//!
//! Three body panes: the isolation **dashboard** (gates, YOLO, guest map, catalog),
//! **Images** (guest compositions), and **Refused** descriptors. Tools are a
//! scrollable catalog — not a per-tool enable grid, and not arrow targets for
//! the daemon / YOLO gates.

use crate::app::sec::SecStatus;

/// One catalog tool row. `ready_mark` is overlaid in Rust from the image inventory
/// (`ready` | `local` | `remote`), never taken from the catalog payload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CatalogRow {
    pub id: String,
    pub blurb: String,
    pub image: String,
    pub exec: String,
    pub domain: String,
    /// `"ready"` | `"missing"` | `"local"` | `"remote"`.
    pub ready_mark: String,
    /// The daemon's probe of the active guest: can it run this tool? `None`
    /// until the guest has been probed; the mark then follows the image pin.
    pub guest_ready: Option<bool>,
}

/// Catalog list filter. `f` cycles All → ReadyHere → NeedsImage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CatalogFilter {
    #[default]
    All,
    ReadyHere,
    NeedsImage,
}

impl CatalogFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ReadyHere => "ready",
            Self::NeedsImage => "needs",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::ReadyHere,
            Self::ReadyHere => Self::NeedsImage,
            Self::NeedsImage => Self::All,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ready" | "ready_here" | "readyhere" => Self::ReadyHere,
            "needs" | "needs_image" | "needsimage" => Self::NeedsImage,
            _ => Self::All,
        }
    }
}

/// Which body pane the `/security` panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecPane {
    /// Isolation dashboard (default): gates, guest map, catalog.
    #[default]
    Tools,
    /// Guest composition images (pull / activate / remove).
    Images,
    /// Handshake refusals (malformed / ungated descriptors).
    Refused,
}

impl SecPane {
    /// Cycle Tools → Images → Refused → Tools.
    pub fn next(self) -> Self {
        match self {
            SecPane::Tools => SecPane::Images,
            SecPane::Images => SecPane::Refused,
            SecPane::Refused => SecPane::Tools,
        }
    }

    /// Parse pane id from client (`tools` / `images` / `refused`).
    ///
    /// `deps` and any unknown token are not a pane — callers treat that as Tools.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tools" | "tool" | "dashboard" | "home" => Some(Self::Tools),
            "images" | "image" | "guest" => Some(Self::Images),
            "refused" | "refuse" => Some(Self::Refused),
            _ => None,
        }
    }

    /// Wire / UI id.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Images => "images",
            Self::Refused => "refused",
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

/// Guest-map domain order. `minimal` is last; it has no `-2` / ext / src siblings.
const MAP_DOMAINS: &[&str] = &[
    "recon", "web", "identity", "cloud", "rev", "crypto", "pxn", "dfir", "lab", "minimal",
];

/// Images that are not a pullable composition. Readiness falls back to "any guest active".
const PSEUDO_IMAGES: &[&str] = &["", "smoke", "none", "external", "debian", "empty"];

/// Working state for the `/security` daemon control panel.
#[derive(Debug, Clone)]
pub struct SecurityState {
    /// Latest status snapshot from the daemon manager (or a default when no manager).
    pub status: SecStatus,
    /// Tools-pane cursor: an index into [`Self::filtered_catalog`], not into gates.
    pub selected: usize,
    /// Tool names the user has disabled, mirrored from `state.rest.sec_inactive`.
    pub inactive: std::collections::HashSet<String>,
    /// YOLO-mode arm flag, mirrored from `state.rest.yolo_armed`.
    pub yolo_armed: bool,
    /// Known guest compositions + local inventory for the Images pane and the map.
    pub images: Vec<SafetyImageRow>,
    /// Images-pane cursor: index into [`Self::images`].
    pub images_selected: usize,
    /// `true` while a long-running image op is in flight (optional; pull is blocking in v1).
    pub images_busy: bool,
    /// Active body pane.
    pub pane: SecPane,
    /// Refused-pane cursor: index into `status.refused`.
    pub refused_selected: usize,
    /// Session cyber-lane label for the status strip (`research` / `red` / `blue` / `none`).
    pub lane_label: String,
    /// Foreground agent-mode label (`auto` / `yolo` / …), mirrored like `lane_label`.
    pub agent_mode_label: String,
    /// Full catalog. Marks are overlaid; the cursor walks [`Self::filtered_catalog`].
    pub catalog: Vec<CatalogRow>,
    pub catalog_filter: CatalogFilter,
    pub catalog_query: String,
    pub catalog_searching: bool,
    /// Composition id from the guest-map cell. `None` = all.
    pub map_filter: Option<String>,
}

impl SecurityState {
    /// Build from a live status snapshot, the disabled-tool set, and the YOLO arm flag.
    /// Seeds images from disk and the catalog from the embedded index (marks overlaid).
    pub fn new(
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
    ) -> Self {
        let mut st = Self {
            status,
            selected: 0,
            inactive,
            yolo_armed,
            images: Vec::new(),
            images_selected: 0,
            images_busy: false,
            pane: SecPane::Tools,
            refused_selected: 0,
            lane_label: "none".into(),
            agent_mode_label: "auto".into(),
            catalog: Vec::new(),
            catalog_filter: CatalogFilter::All,
            catalog_query: String::new(),
            catalog_searching: false,
            map_filter: None,
        };
        st.refresh_images();
        st.seed_embedded_catalog();
        st
    }

    /// Replace the catalog with the embedded index, then overlay marks.
    pub fn seed_embedded_catalog(&mut self) {
        self.set_catalog_index(crate::security::embedded_catalog_rows());
    }

    /// Replace catalog rows from a parsed index (file or daemon). Marks start remote,
    /// then overlay from the current image inventory.
    pub fn set_catalog_index(&mut self, rows: Vec<crate::security::CatalogIndexRow>) {
        self.catalog = rows
            .into_iter()
            .map(|r| CatalogRow {
                id: r.id,
                blurb: r.blurb,
                image: r.image,
                exec: r.exec,
                domain: r.domain,
                ready_mark: "remote".into(),
                guest_ready: r.ready,
            })
            .collect();
        self.overlay_ready_marks();
        self.clamp_cursors();
    }

    /// Rebuild [`Self::images`] from the static catalogue + `images.json` inventory,
    /// then recompute catalog marks (readiness rides the inventory, not the payload).
    pub fn refresh_images(&mut self) {
        let inv = crate::security::read_images_inventory();
        // Images float on `latest`; a pull from another koma version still counts.
        let version_ok = true;
        let mut rows = Vec::with_capacity(crate::security::list_known_compositions().len());
        for (id, blurb) in crate::security::list_known_compositions() {
            let tag = crate::security::composition_tag(id);
            let entry = if version_ok {
                inv.images.get(*id)
            } else {
                None
            };
            let local = entry.map(|e| !e.digest.is_empty()).unwrap_or(false);
            let active = version_ok && local && inv.is_active(id);
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
        self.overlay_ready_marks();
        self.clamp_cursors();
    }

    fn overlay_ready_marks(&mut self) {
        overlay_ready_marks(&mut self.catalog, &self.images);
    }

    /// Indices into [`Self::catalog`] that pass filter, map cell, and query.
    /// The cursor and the renderer both walk this list.
    pub fn filtered_catalog(&self) -> Vec<usize> {
        let q = self.catalog_query.to_ascii_lowercase();
        self.catalog
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                let filter_ok = match self.catalog_filter {
                    CatalogFilter::All => true,
                    CatalogFilter::ReadyHere => row.ready_mark == "ready",
                    CatalogFilter::NeedsImage => row.ready_mark != "ready",
                };
                if !filter_ok {
                    return false;
                }
                if let Some(id) = self.map_filter.as_deref() {
                    if !row.image.eq_ignore_ascii_case(id) {
                        return false;
                    }
                }
                if q.is_empty() {
                    return true;
                }
                row.id.to_ascii_lowercase().contains(&q)
                    || row.blurb.to_ascii_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Selected catalog row, if the filtered window has one.
    pub fn selected_catalog(&self) -> Option<&CatalogRow> {
        let idx = *self.filtered_catalog().get(self.selected)?;
        self.catalog.get(idx)
    }

    /// Currently selected Images row, if any.
    pub fn selected_image(&self) -> Option<&SafetyImageRow> {
        self.images.get(self.images_selected)
    }

    /// Guest-map cells in row-major order, with a leading `None` ("all") position.
    pub fn map_cell_ids(&self) -> Vec<Option<String>> {
        let mut cells = vec![None];
        for domain in MAP_DOMAINS {
            for id in map_column_ids(domain, &self.images) {
                cells.push(Some(id));
            }
        }
        cells
    }

    /// Left/Right on the Tools pane. `dir` is -1 or +1; wraps, including "all".
    pub fn cycle_map(&mut self, dir: i32) {
        let cells = self.map_cell_ids();
        if cells.is_empty() {
            self.map_filter = None;
            return;
        }
        let cur = cells
            .iter()
            .position(|c| c.as_deref() == self.map_filter.as_deref())
            .unwrap_or(0);
        let n = cells.len() as i32;
        let next = (cur as i32 + dir).rem_euclid(n) as usize;
        self.map_filter = cells[next].clone();
        self.selected = 0;
        self.clamp_cursors();
    }

    /// `f` — cycle the catalog readiness filter and reset the tool cursor.
    pub fn cycle_filter(&mut self) {
        self.catalog_filter = self.catalog_filter.next();
        self.selected = 0;
        self.clamp_cursors();
    }

    /// Move the cursor up by one (saturating at 0) in the ACTIVE pane.
    pub fn move_up(&mut self) {
        match self.pane {
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
                let max = self.filtered_catalog().len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                }
            }
        }
    }

    /// Cycle Tools → Images → Refused → Tools, re-clamping the newly active cursor.
    pub fn cycle_pane(&mut self) {
        self.pane = self.pane.next();
        self.clamp_cursors();
    }

    pub fn set_pane(&mut self, pane: SecPane) {
        self.pane = pane;
        self.clamp_cursors();
    }

    /// Replace the status snapshot + disabled-tool set + YOLO arm flag.
    ///
    /// PRESERVES catalog and images — those reload explicitly (`u` / image ops).
    pub fn refresh(
        &mut self,
        status: SecStatus,
        inactive: std::collections::HashSet<String>,
        yolo_armed: bool,
    ) {
        self.status = status;
        self.inactive = inactive;
        self.yolo_armed = yolo_armed;
        self.clamp_cursors();
    }

    /// Clamp ALL pane cursors against their current navigable-row counts.
    pub(crate) fn clamp_cursors(&mut self) {
        let tool_len = self.filtered_catalog().len();
        if tool_len == 0 {
            self.selected = 0;
        } else if self.selected >= tool_len {
            self.selected = tool_len - 1;
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

/// Column ids that exist in `images` for `domain`, in map order:
/// base, `-2` (only if present), ext, src.
pub fn map_column_ids(domain: &str, images: &[SafetyImageRow]) -> Vec<String> {
    let has = |id: &str| images.iter().any(|r| r.id == id);
    let mut ids = Vec::new();
    if has(domain) {
        ids.push(domain.to_string());
    }
    let two = format!("{domain}-2");
    if has(&two) {
        ids.push(two);
    }
    let ext = format!("{domain}-ext");
    if has(&ext) {
        ids.push(ext);
    }
    let src = format!("{domain}-src");
    if has(&src) {
        ids.push(src);
    }
    ids
}

/// True when `image` is a pullable composition id (not smoke/none/external/debian/empty).
pub fn is_real_composition(image: &str) -> bool {
    let img = image.trim().to_ascii_lowercase();
    !PSEUDO_IMAGES.contains(&img.as_str())
}

/// Overlay readiness onto catalog rows from the image inventory.
///
/// - smoke/none/external/debian/empty: `ready` if any image row is active, else `remote`
/// - else match `images.id` case-insensitively: active → ready, local → local, else remote
pub fn ready_mark_for(image: &str, images: &[SafetyImageRow]) -> String {
    let key = image.trim().to_ascii_lowercase();
    if PSEUDO_IMAGES.contains(&key.as_str()) {
        return if images.iter().any(|r| r.active) {
            "ready".into()
        } else {
            "remote".into()
        };
    }
    match images
        .iter()
        .find(|r| r.id.eq_ignore_ascii_case(image.trim()))
    {
        Some(r) if r.active => "ready".into(),
        Some(r) if r.local => "local".into(),
        _ => "remote".into(),
    }
}

pub fn overlay_ready_marks(rows: &mut [CatalogRow], images: &[SafetyImageRow]) {
    let active = images.iter().any(|r| r.active);
    for row in rows {
        let mark = ready_mark_for(&row.image, images);
        // The probe of the active guest wins over the pin: an image can ship a
        // tool another composition lists, and a build can miss one it lists.
        row.ready_mark = match (active, row.guest_ready) {
            (true, Some(true)) => "ready".into(),
            (true, Some(false)) if mark == "ready" => "missing".into(),
            _ => mark,
        };
    }
}

/// Domain order the guest map walks. Exposed for the view.
pub fn map_domains() -> &'static [&'static str] {
    MAP_DOMAINS
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

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: &str, local: bool, active: bool) -> SafetyImageRow {
        SafetyImageRow {
            id: id.into(),
            blurb: String::new(),
            tag: String::new(),
            size_label: "—".into(),
            local,
            active,
            status: if active {
                "active".into()
            } else if local {
                "local".into()
            } else {
                "remote".into()
            },
        }
    }

    fn row(id: &str, image: &str) -> CatalogRow {
        CatalogRow {
            id: id.into(),
            blurb: String::new(),
            image: image.into(),
            exec: "net".into(),
            domain: "recon".into(),
            ready_mark: "remote".into(),
            guest_ready: None,
        }
    }

    #[test]
    fn pane_cycle_is_tools_images_refused() {
        assert_eq!(SecPane::Tools.next(), SecPane::Images);
        assert_eq!(SecPane::Images.next(), SecPane::Refused);
        assert_eq!(SecPane::Refused.next(), SecPane::Tools);
    }

    #[test]
    fn catalog_ready_mark_follows_image_rows() {
        let images = vec![image("recon", true, true), image("web", true, false)];
        let mut rows = vec![row("a", "recon"), row("b", "WEB"), row("c", "pxn")];
        overlay_ready_marks(&mut rows, &images);
        assert_eq!(rows[0].ready_mark, "ready");
        assert_eq!(rows[1].ready_mark, "local");
        assert_eq!(rows[2].ready_mark, "remote");

        assert_eq!(ready_mark_for("smoke", &images), "ready");
        assert_eq!(ready_mark_for("none", &images), "ready");
        assert_eq!(ready_mark_for("external", &[image("web", true, false)]), "remote");
        assert_eq!(ready_mark_for("debian", &[]), "remote");
        assert_eq!(ready_mark_for("empty", &[]), "remote");
    }

    #[test]
    fn guest_probe_overrides_the_pin_while_an_image_is_active() {
        let images = vec![image("recon", true, true), image("web", true, false)];
        let probed = |id: &str, img: &str, ready: bool| CatalogRow {
            guest_ready: Some(ready),
            ..row(id, img)
        };
        let mut rows = vec![
            probed("built", "recon", true),
            probed("failed-build", "recon", false),
            probed("shared-bin", "web", true),
            probed("elsewhere", "web", false),
        ];
        overlay_ready_marks(&mut rows, &images);
        let marks: Vec<_> = rows.iter().map(|r| r.ready_mark.as_str()).collect();
        assert_eq!(marks, ["ready", "missing", "ready", "local"]);

        // No active guest: a stale probe says nothing about what can run.
        let mut rows = vec![probed("built", "recon", true)];
        overlay_ready_marks(&mut rows, &[image("recon", true, false)]);
        assert_eq!(rows[0].ready_mark, "local");
    }
}
