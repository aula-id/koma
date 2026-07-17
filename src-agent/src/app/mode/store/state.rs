//! [`ExtStoreState`] — the working state for the in-app `/store` marketplace browser.
//!
//! Unlike `/extension` (a read-only snapshot of the LOCAL registry), `/store` fetches
//! from the koma.run network API: `rows` is populated by an ASYNC catalogue fetch kicked
//! off on open (and on `r`-retry), `detail` by an async per-extension fetch kicked off on
//! Enter, and an install runs a further async download + on-loop verify/unpack. Every
//! fetch/install lands on the shared `AppStateRest::store_rx` channel, drained per-tick by
//! `event_loop::global::drains::drain_store` (see `app::ext::ext_store` for the kick-off
//! functions + the channel's event type). Navigation lives here; the network calls +
//! row/detail mapping live in the command/runtime layers (which own `AppStateRest` /
//! `AppState`).

use super::types::StoreSubMode;

/// One catalogue row — a mapped [`crate::ipc::proto::StoreItemWire`] plus the LOCALLY
/// baked `installed` flag (checked against `config.installed_extensions` at fetch/fold
/// time, since the store API has no notion of what's installed on this machine).
#[derive(Debug, Clone)]
pub struct StoreRow {
    /// Reverse-DNS store id.
    pub id: String,
    pub name: String,
    pub tagline: String,
    /// Tier wire string: `"free"` | `"paid"`.
    pub tier: String,
    /// Kind wire string: `"daemon"` | `"oneshot"`.
    pub kind: String,
    pub latest_version: String,
    pub author: String,
    /// Whether this id is already present in `config.installed_extensions`.
    pub installed: bool,
}

/// The `/store` detail pane's fetched data — a flattened mapping of
/// [`crate::ipc::proto::StoreDetailWire`]: `description` is `description_md` with its
/// markdown headers minimally stripped (plain wrapped text, no full markdown renderer);
/// `contributes_*` are the per-kind counts.
#[derive(Debug, Clone)]
pub struct StoreDetailData {
    pub description: String,
    pub contributes_models: u32,
    pub contributes_panels: u32,
    pub contributes_tools: u32,
    pub contributes_sub_agents: u32,
    pub requires: Vec<String>,
    pub versions: Vec<String>,
}

/// Working state for the in-app `/store` marketplace browser.
#[derive(Debug, Clone)]
pub struct ExtStoreState {
    /// Active sub-mode (Browse / Detail / InstallConfirm).
    pub sub_mode: StoreSubMode,
    /// The fetched catalogue rows, in the order the store API returned them.
    pub rows: Vec<StoreRow>,
    /// Selected index into `rows` (the LIST cursor).
    pub list_sel: usize,
    /// `true` while the Browse catalogue fetch is in flight.
    pub loading: bool,
    /// Last Browse fetch error, if any (shown with an `r`-to-retry hint).
    pub error: Option<String>,
    /// The selected extension's fetched detail, once loaded.
    pub detail: Option<StoreDetailData>,
    /// `true` while the Detail fetch is in flight.
    pub detail_loading: bool,
    /// Last Detail fetch error, if any.
    pub detail_error: Option<String>,
    /// `true` while an install download+verify+unpack is in flight.
    pub installing: bool,
    /// Last install error, if any (shown on the InstallConfirm/Detail pane).
    pub install_error: Option<String>,
    /// Whether a koma.run OAuth connection is on file — install is impossible without
    /// one. Baked at mode-open time and refreshed when InstallConfirm is armed.
    pub komarun_connected: bool,
}

impl ExtStoreState {
    /// The currently-selected catalogue row, if any.
    pub fn current(&self) -> Option<&StoreRow> {
        self.rows.get(self.list_sel)
    }

    /// Move the LIST cursor up.
    pub fn list_up(&mut self) {
        self.list_sel = self.list_sel.saturating_sub(1);
    }

    /// Move the LIST cursor down.
    pub fn list_down(&mut self) {
        if self.list_sel + 1 < self.rows.len() {
            self.list_sel += 1;
        }
    }
}
