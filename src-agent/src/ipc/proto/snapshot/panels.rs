// ─── /mcp, /help, /security, /bash, /todo and /agents full-screen panels ──────

use serde::{Deserialize, Serialize};

use super::settings::{
    AgentEntry, AgentModelPickerSnapshot, CatalogueModelSnapshot, CatalogueProviderSnapshot,
    TextEditorSnapshot, ToolPickerSnapshot,
};

/// A serde-safe projection of the `/mcp` server dashboard.
///
/// Mirrors [`crate::app::mode::mcp::McpState`] field-for-field. The configured
/// servers ride as `McpServerEntry` directly (it already derives serde + is pure
/// data — no key/secret material, so no lighter mirror is needed, exactly the
/// AgentsSnapshot stance of carrying the lightest serde-able server record). The
/// sub-mode / field / transport enums cross as wire tokens (see `tokens.rs`), and
/// `status` carries the daemon's LIVE per-server tool counts (uuid -> count) so a
/// thin client — which owns no MCP manager — can render the `● N tools` column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct McpSnapshot {
    pub servers: Vec<crate::model::app_config::McpServerEntry>,
    pub list_sel: usize,
    pub in_detail: bool,
    pub mode: String,
    pub field: String,
    pub editing: bool,
    pub draft_uuid: String,
    pub draft_name: String,
    pub draft_enabled: bool,
    pub draft_transport: String,
    pub draft_command: String,
    pub draft_args: String,
    pub draft_env: String,
    pub draft_url: String,
    /// Live per-server tool counts (server uuid -> tool count) from the daemon's
    /// MCP manager, projected so the client's status column matches the daemon's.
    pub status: std::collections::HashMap<String, usize>,
}

/// One TUI-screen row for [`ExtRowWire`] — a serde mirror of
/// [`crate::app::mode::ExtTuiScreen`] (also the SDK `TuiScreenDef`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtTuiScreenWire {
    pub id: String,
    pub title: String,
}

/// A serde-safe projection of ONE installed-extension row — mirrors
/// [`crate::app::mode::ExtRow`] field-for-field (all pure data: registry facts +
/// manifest-derived counts/screens + the LIVE running flag), so a thin client rebuilds and
/// renders the `/extension` dashboard faithfully instead of a blank Chat screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtRowWire {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tier: String,
    pub kind: String,
    pub enabled: bool,
    pub running: bool,
    pub description: String,
    pub granted: Vec<String>,
    pub tools: usize,
    pub panels: usize,
    pub sub_agents: usize,
    pub models: usize,
    pub tui_screens: Vec<ExtTuiScreenWire>,
    pub workspace_dir: Option<String>,
}

/// A serde-safe projection of the `/extension` dashboard.
///
/// Mirrors [`crate::app::mode::ExtensionsState`]: the installed-extension rows + the LIST
/// cursor + the sub-mode wire token (see `ext_submode_token` in `tokens.rs`) + the
/// tui-screen cursor + any in-state error. The client rebuilds it verbatim and renders the
/// same master/detail/confirm view; it never mutates it (keys forwarded to the daemon).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtensionsSnapshot {
    pub rows: Vec<ExtRowWire>,
    pub list_sel: usize,
    /// Sub-mode wire token: "browse" | "detail" | "uninstall_confirm".
    pub mode: String,
    pub screen_sel: usize,
    pub error: Option<String>,
}

/// A serde-safe projection of an OPEN extension-driven TUI screen.
///
/// Mirrors [`crate::app::mode::ExtScreenState`]: the ext/screen ids + declared title, the
/// current `Screen` model (an OPAQUE `serde_json::Value` carried verbatim), the menu cursor,
/// and the loading/error flags. A thin client rebuilds it and renders the same server-driven
/// screen off the projection (the daemon owns the invoke + folds every reply/push).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtScreenSnapshot {
    pub ext_id: String,
    pub screen_id: String,
    pub screen_title: String,
    pub screen: Option<serde_json::Value>,
    pub menu_cursor: usize,
    pub waiting: bool,
    pub error: Option<String>,
}

/// A serde-safe projection of one row in the `/help` reference.
///
/// Mirrors [`crate::app::mode::help::HelpEntry`] field-for-field. The `kind` enum
/// crosses as a wire token (see `help_kind_token` in `tokens.rs`), exactly as the
/// McpSnapshot tokenizes its sub-mode / transport enums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HelpEntrySnapshot {
    /// "command" | "keybinding" — the wire token for `HelpKind`.
    pub kind: String,
    pub key: String,
    pub desc: String,
}

/// A serde-safe projection of the full-screen, searchable `/help` reference.
///
/// Mirrors [`crate::app::mode::help::HelpState`] field-for-field. Each entry rides as
/// a `HelpEntrySnapshot` (its `kind` enum tokenized like McpSnapshot's enums), so a
/// thin client — which renders the same view::draw path — rebuilds and shows the help
/// screen instead of a blank Chat screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HelpSnapshot {
    pub query: String,
    pub all: Vec<HelpEntrySnapshot>,
    pub filtered_idx: Vec<usize>,
    pub selected: usize,
    /// The compiled-in koma version for the "Updating koma" block (mirrors
    /// [`crate::app::mode::help::HelpState::current_version`]).
    pub current_version: String,
    /// `Some((latest, message))` iff a newer koma version is available (mirrors
    /// [`crate::app::mode::help::HelpState::update`]); serde-safe (tuple of
    /// String + Option<String>).
    pub update: Option<(String, Option<String>)>,
}

/// A serde-safe projection of the `/security` daemon control panel.
///
/// Carries a full [`crate::app::sec::SecStatus`] (which already derives
/// Serialize + Deserialize) plus the tool-list cursor. The projection re-reads
/// LIVE status from the daemon manager at snapshot time (see
/// `ipc::snapshot::projection::modes::security_snapshot`) so the panel always
/// reflects current daemon state after start/stop/restart, rather than the
/// potentially-stale snapshot that was open when the mode was entered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SecuritySnapshot {
    /// Live status from the daemon manager (running, installed, tools).
    pub status: crate::app::sec::SecStatus,
    /// Selected index into `status.tools` (the tool-inventory cursor).
    pub selected: usize,
    /// Tool names the user has disabled (the inactive set), as a sorted Vec so the
    /// projection round-trips deterministically. Empty = every tool active. The view
    /// dims a row whose name is in this list and the daemon filters them out of the
    /// model's advertised tools.
    #[serde(default)]
    pub inactive: Vec<String>,
    /// Layer-1 YOLO arm flag, mirrored from `state.rest.yolo_armed` so the thin client's
    /// panel renders the ARMED/locked status row faithfully. `#[serde(default)]` keeps
    /// an older client decoding a newer daemon (and vice-versa) safe (defaults false).
    #[serde(default)]
    pub yolo_armed: bool,
    /// Per-dependency install-health, carried VERBATIM from the open mode state (NOT
    /// re-fetched at snapshot time — `health()` is a heavy IPC round-trip and the
    /// projection runs every frame). Empty when the daemon is stopped.
    #[serde(default)]
    pub install_health: Vec<crate::app::sec::InstallHealthEntry>,
    /// Which body pane is showing: `false` = tools (default), `true` = dependencies.
    #[serde(default)]
    pub health_view: bool,
    /// Selected index into `install_health` (the dependency-pane cursor).
    #[serde(default)]
    pub health_selected: usize,
    /// `true` while a non-blocking health probe is in flight on the daemon. Projected so
    /// the thin client renders the same "checking dependencies…" spinner state on the
    /// daemon info line. `#[serde(default)]` keeps version-skewed peers safe (false).
    #[serde(default)]
    pub health_fetching: bool,
    /// Braille spinner frame counter for the in-flight probe. MUST be projected (it is
    /// advanced daemon-side every tick) so the client's spinner actually animates rather
    /// than sitting on a single frame. `#[serde(default)]` keeps version-skewed peers safe.
    #[serde(default)]
    pub health_frame: u64,
}

/// A serde-safe projection of ONE background bash job for the `/bash` panel.
///
/// An already-rendered view of a [`crate::app::bgbash::BashJob`] (whose live
/// `Arc<Mutex<…>>` + `Instant` state can't cross the wire): `status` is the
/// status rendered to a label (`"running"` / `"exit {n}"` / `"killed"` /
/// `"error: {…}"`), `running` flags a still-live job, `elapsed_secs` is the
/// wall-clock age, and `output_tail` is the last slice of captured output. Built
/// LIVE every frame by `bash_job_views`, exactly as the agents list is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BashJobView {
    pub id: usize,
    pub command: String,
    pub status: String,
    pub running: bool,
    pub elapsed_secs: u64,
    pub output_tail: String,
}

/// A serde-safe projection of the `/bash` background-job panel.
///
/// Mirrors [`AgentsSnapshot`]'s shape (list + cursor): the job views + the LIST
/// cursor. The client rebuilds [`crate::app::mode::BashState`] from this verbatim
/// and renders the same master/detail view; it never mutates it (keys are
/// forwarded to the daemon).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BashSnapshot {
    pub jobs: Vec<BashJobView>,
    pub selected: usize,
}

/// A serde-safe projection of one todo item for the `/todo` panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TodoItemSnapshot {
    pub content: String,
    /// Wire token: "pending" | "in_progress" | "completed" | "cancelled"
    pub status: String,
    /// Wire token: "high" | "medium" | "low"
    pub priority: String,
    pub locked: bool,
}

/// A serde-safe projection of the `/todo` task-panel.
///
/// Mirrors [`BashSnapshot`]'s shape (list + cursor): the todo items + the LIST
/// cursor. The client rebuilds [`crate::app::mode::TodoState`] from this verbatim
/// and renders the same master/detail view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TodoSnapshot {
    pub items: Vec<TodoItemSnapshot>,
    pub selected: usize,
    pub pwd_hash: String,
}

/// One catalogue row for the `/store` marketplace browser — a slimmed mirror of
/// [`crate::ipc::proto::StoreItemWire`] plus the LOCALLY-baked `installed` flag (checked
/// against `config.installed_extensions` at fetch time, since the store API itself has
/// no notion of what's installed on THIS machine).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtStoreRowWire {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub tier: String,
    pub kind: String,
    pub latest_version: String,
    pub author: String,
    pub installed: bool,
}

/// The `/store` detail pane's data — a flattened mirror of
/// [`crate::ipc::proto::StoreDetailWire`]: `description_md` pre-stripped of markdown
/// headers (the TUI renders plain wrapped text, no full markdown renderer) and
/// `contributes` flattened to its four counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtStoreDetailWire {
    pub description: String,
    pub contributes_models: u32,
    pub contributes_panels: u32,
    pub contributes_tools: u32,
    pub contributes_sub_agents: u32,
    pub requires: Vec<String>,
    pub versions: Vec<String>,
}

/// A serde-safe projection of the `/store` marketplace browser.
///
/// Mirrors [`crate::app::mode::store::ExtStoreState`] field-for-field: the catalogue
/// rows + LIST cursor + sub-mode wire token (see `store_submode_token` in `tokens.rs`),
/// the Browse loading/error pair, the Detail loading/error pair + fetched detail, and
/// the InstallConfirm install-in-flight/error pair + the koma.run connection flag. The
/// client rebuilds it verbatim and renders the same browse/detail/confirm view; it
/// never mutates it (keys forwarded to the daemon).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtStoreSnapshot {
    pub rows: Vec<ExtStoreRowWire>,
    pub list_sel: usize,
    /// Sub-mode wire token: "browse" | "detail" | "install_confirm".
    pub mode: String,
    pub loading: bool,
    pub error: Option<String>,
    pub detail: Option<ExtStoreDetailWire>,
    pub detail_loading: bool,
    pub detail_error: Option<String>,
    pub installing: bool,
    pub install_error: Option<String>,
    pub komarun_connected: bool,
}

/// A serde-safe projection of the /agents dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct AgentsSnapshot {
    pub agents: Vec<AgentEntry>,
    pub list_sel: usize,
    pub in_detail: bool,
    pub mode: String,
    pub field: String,
    pub editing: bool,
    pub create_scope: String,
    pub draft_name: String,
    pub draft_description: String,
    pub draft_conditions: String,
    pub draft_model_uuid: Option<String>,
    pub draft_model_legacy: Option<String>,
    pub draft_tools: String,
    pub draft_body: String,
    pub tool_picker: Option<ToolPickerSnapshot>,
    pub model_picker: Option<AgentModelPickerSnapshot>,
    pub editor: Option<(String, TextEditorSnapshot)>,
    pub editor_clear_confirm: bool,
    pub catalogue_models: Vec<CatalogueModelSnapshot>,
    pub catalogue_providers: Vec<CatalogueProviderSnapshot>,
}
