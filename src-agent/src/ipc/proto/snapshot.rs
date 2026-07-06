// ─── full-state snapshot and mode payload projections (pure data) ────────────

use serde::{Deserialize, Serialize};

use crate::dto::chat::ChatMessage;

use super::ModeSnapshot;

/// A complete, frozen projection of the daemon's renderable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct StateSnapshot {
    pub foreground_id: Option<String>,
    pub sessions: Vec<SessionSnapshot>,
    pub global: GlobalSnapshot,
}

/// A per-session projection of everything the client needs to render ONE session tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SessionSnapshot {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub committed_reasoning: Vec<Option<String>>,
    pub streaming: Option<String>,
    pub stream_reasoning: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub tokens_cached: u64,
    pub waiting: bool,
    pub awaiting_approval: bool,
    pub approval_reason: Option<String>,
    pub pending_tool_calls: Vec<crate::dto::chat::ToolCall>,
    pub tool_idx: usize,
    pub working: bool,
    pub finished_unseen: bool,
    pub subagents: Vec<SubAgentSnapshot>,
    pub pending_subagents: Vec<PendingSubagentSnapshot>,
    pub resolved_model_id: String,
    /// Truncated previews of the foreground session's queued mid-turn steer messages
    /// (full text lives daemon-side). Drives the pending panel between transcript +
    /// composer. Empty = no panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_steer: Vec<String>,
}

/// A plain-data projection of one SubAgent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SubAgentSnapshot {
    pub id: usize,
    pub name: String,
    pub label: String,
    pub status: String,
    /// Whether this sub-agent is backgrounded (detached). Projected so the
    /// client's `!detached` render/cadence checks work — without it the client
    /// defaults every agent to attached and keeps animating a backgrounded one.
    #[serde(default)]
    pub detached: bool,
    pub steps: usize,
    pub transcript: Vec<String>,
    pub messages: Vec<ChatMessage>,
    /// Live in-progress assistant report text for the current (not-yet-committed)
    /// turn, mirrored from `SubAgent::live_text`. Lets the full-screen viewer render
    /// the streaming report on a thin client as it arrives, instead of only after the
    /// turn commits into `messages`. `#[serde(default)]` keeps version-skewed peers
    /// safe (empty when absent).
    #[serde(default)]
    pub live_text: String,
    /// Out-of-band, index-aligned reasoning for `messages` (mirrors
    /// `SessionSnapshot::committed_reasoning`). `ChatMessage::reasoning` is
    /// `#[serde(skip)]`, so the viewer's thinking blocks must ride this side
    /// channel to survive IPC. Empty vec = no reasoning on any message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub committed_reasoning: Vec<Option<String>>,
}

/// A plain-data projection of one queued PendingSubagent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PendingSubagentSnapshot {
    pub id: usize,
    pub agent_name: String,
    pub prompt: String,
}

/// Projection of the mode-independent, NON-session global UI state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GlobalSnapshot {
    pub input: String,
    pub cursor: usize,
    pub scroll: u16,
    pub follow: bool,
    pub status: String,
    pub work_elapsed_ms: Option<u64>,
    pub theme: String,
    pub accent: String,
    /// Active palette registry name (see `view::theme::PALETTES`). Projected verbatim
    /// like `accent`; the thin client rebuilds its palette from this.
    pub palette: String,
    pub mode: ModeSnapshot,
    pub toast: Option<(String, String)>,
    pub models_cache: Option<Vec<crate::dto::openrouter::ModelInfo>>,
    pub models_cache_endpoint: Option<String>,
    pub models_cache_failed: Option<String>,
    pub agent_viewer: Option<usize>,
    pub agent_viewer_scroll: u16,
    pub agent_viewer_follow: bool,
    pub subagents_open: bool,
    pub subagent_sel: usize,
    pub palette_sel: usize,
    pub pending_attachments: Vec<crate::dto::chat::Attachment>,
    pub file_palette: Option<Vec<String>>,
    pub agent_mode: String,
    /// Latest published koma version when newer than the running one (for the
    /// header update badge), else None. Projected from the daemon's version check.
    pub latest_version: Option<String>,
}

// -- mode payload projections (stage 2: core interactive modes) ----------------

/// A serde-safe projection of the first-run connection chooser (`Mode::Onboard`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OnboardSnapshot {
    pub cursor: usize,
}

/// A serde-safe projection of the guided provider onboarding wizard
/// (`Mode::OnboardProvider`).
///
/// The model-result list is NOT carried: the thin client recomputes it from the
/// globally-projected `models_cache` (+ the compiled-in Codex static list) exactly as
/// the `/settings` model omnisearch does, so no separate results projection is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OnboardProviderSnapshot {
    /// Wire token for the active step: `"login"` | `"model_select"`.
    pub step: String,
    /// The reused OAuth connect-flow state (picker / wait / paste / failed).
    pub oauth_flow: OAuthFlowSnapshot,
    /// The just-created connection's uuid (set on login success).
    pub new_conn_uuid: String,
    /// Signed-in provider wire token: `Some("codex"|"kilocode")`, `None` pre-login.
    pub provider: Option<String>,
    /// Model omnisearch query.
    pub query: String,
    /// Highlighted row in the filtered model list.
    pub result_sel: usize,
}

/// A serde-safe projection of the first-run setup wizard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct KeyInputSnapshot {
    pub step: usize,
    pub field: usize,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub query: String,
    pub result_sel: usize,
    pub first_run: bool,
    pub from_picker: bool,
}

/// A serde-safe projection of the startup warming splash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct LoadingSnapshot {
    pub elapsed_ms: u64,
    pub frame: u64,
    pub workspace: WarmStatusWire,
    pub awareness: WarmStatusWire,
}

/// A serde-safe mirror of WarmStatus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum WarmStatusWire {
    Pending,
    Running,
    Done(String),
    Skipped,
    Failed,
}

/// A serde-safe projection of one COOKING row in the session hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CookingEntrySnapshot {
    pub name: String,
    pub kind: String,
    pub working: bool,
    pub is_foreground: bool,
    /// The session's UUID, used by the client-side confirm bar to resolve the armed
    /// target by identity. `None` for the synthetic `[+ new session]` row. Added so
    /// the client renderer can search `cooking` by `session_id` (matching the daemon
    /// handler's identity-based `pending_kill`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A serde-safe projection of one HISTORY row in the session hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HistoryEntrySnapshot {
    pub name: String,
    pub last_active_secs: u64,
}

/// A serde-safe projection of the two-pane session hub.
///
/// `history` carries the ALREADY-FILTERED rows (the daemon projects only the rows
/// matching `history_query`), so `history_selected` indexes straight into it on the
/// client. `pending_kill` carries the targeted session's UUID (identity-based, not an
/// index), so the client's confirm bar resolves the target by searching `cooking` for
/// a matching `session_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SessionHubSnapshot {
    pub cooking: Vec<CookingEntrySnapshot>,
    pub history: Vec<HistoryEntrySnapshot>,
    pub focus_cooking: bool,
    pub cooking_selected: usize,
    pub history_selected: usize,
    pub history_query: String,
    pub pending_kill: Option<String>,
}

/// A serde-safe mirror of ModelEndpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelEndpointWire {
    pub name: Option<String>,
    pub provider_name: Option<String>,
    pub price_prompt: Option<String>,
    pub price_completion: Option<String>,
    pub uptime_last_30m: Option<f64>,
}

/// A serde-safe projection of one API-provider draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProviderDraftSnapshot {
    pub uuid: String,
    pub name: String,
    pub endpoint: String,
    pub api_type: String,
    pub api_key: String,
}

/// A serde-safe projection of one OAuth connection draft (provider-cycle merge
/// in the Models Select modal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OAuthDraftSnapshot {
    pub uuid: String,
    pub label: String,
    /// Wire token: `"codex"` | `"kilocode"`.
    pub provider: String,
    pub key: String,
    /// Display status computed at build time: `"active"` / `"renews in Nd"` /
    /// `"expired"` / `"no expiry"`. `#[serde(default)]` keeps an older peer's
    /// snapshot (pre-OAuth-submenu) decoding cleanly (empty string).
    #[serde(default)]
    pub status: String,
}

/// A serde-safe projection of the `/settings` OAuth submenu's connect-flow state
/// ([`crate::app::mode::settings::OAuthFlowState`]). Flat rather than a wire enum
/// (simplest shape): `kind` is the active variant's tag and every variant's data
/// rides in whichever of the other fields it needs; unused fields sit at their
/// default. `url` is dual-purpose — the Codex authorization URL for `codex_wait`,
/// the Kilo Code verification URL for `kilo_wait`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct OAuthFlowSnapshot {
    /// `"idle"` | `"starting"` | `"pick"` | `"codex_wait"` | `"codex_paste"` |
    /// `"kilo_wait"` | `"failed"`.
    pub kind: String,
    /// `Pick`'s cursor (0=Codex, 1=Kilo Code, 2=Codex paste-token).
    pub cursor: usize,
    /// `codex_wait`'s authorization URL / `kilo_wait`'s verification URL.
    pub url: String,
    /// `kilo_wait`'s device code the user approves.
    pub user_code: String,
    /// `codex_paste`'s in-progress token draft.
    pub input: String,
    /// `failed`'s human-readable reason.
    pub error: String,
    /// `codex_wait`/`kilo_wait`'s braille-spinner frame counter, advanced
    /// daemon-side every tick while the flow is in flight.
    pub frame: u8,
    /// `codex_wait`/`kilo_wait`'s "url copied to clipboard" confirmation flag,
    /// set after a successful `c` (copy-url) key press.
    pub copied: bool,
}

/// A serde-safe projection of one model draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelDraftSnapshot {
    pub uuid: String,
    pub name: String,
    pub model_id: String,
    pub provider_idx: usize,
    pub roles: Vec<String>,
    pub route: Option<String>,
    pub session_only: bool,
}

/// A serde-safe projection of the add-provider modal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProviderModalSnapshot {
    pub name: String,
    pub endpoint: String,
    pub api_type: String,
    pub api_key: String,
    pub field: usize,
}

/// A serde-safe projection of the role multi-select picker overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RolePickerSnapshot {
    pub checked: Vec<bool>,
    pub cursor: usize,
}

/// A serde-safe projection of the add/edit-model modal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelModalSnapshot {
    pub editing_idx: Option<usize>,
    pub uuid: String,
    pub name: String,
    pub provider_idx: usize,
    pub model_id: String,
    pub field: usize,
    pub roles: Vec<String>,
    pub role_picker: Option<RolePickerSnapshot>,
    pub query: String,
    pub result_sel: usize,
    pub route: Option<String>,
    pub route_sel: usize,
    pub endpoints: Option<Vec<ModelEndpointWire>>,
    pub endpoints_loading: bool,
    pub endpoints_for: Option<String>,
    /// Scope chosen at open time: `false` = global, `true` = session-local.
    #[serde(default)]
    pub session_only: bool,
}

/// A serde-safe projection of the filesystem directory picker overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PathPickerSnapshot {
    pub query: String,
    pub matches: Vec<String>,
    pub sel: usize,
    pub replace_idx: Option<usize>,
}

/// A serde-safe projection of the /settings dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SettingsSnapshot {
    pub cat: usize,
    pub field: usize,
    pub in_detail: bool,
    pub editing: bool,
    pub api_key: String,
    pub model: String,
    pub provider: String,
    pub name: String,
    pub theme: String,
    pub accent: String,
    pub workdir: Vec<String>,
    pub awareness_enabled: bool,
    pub awareness_inherit: bool,
    pub awareness_model: String,
    pub awareness_provider: String,
    pub classifier_enabled: bool,
    pub classifier_model: String,
    pub classifier_provider: String,
    pub allowed_folders: Vec<String>,
    pub short_send_enabled: bool,
    pub sliding_cache: bool,
    pub bash_saving: bool,
    pub internet_mode: String,
    pub cwd: String,
    pub list_editing: bool,
    pub list_sel: usize,
    pub picker: Option<PathPickerSnapshot>,
    pub providers: Vec<ProviderDraftSnapshot>,
    #[serde(default)]
    pub oauth_drafts: Vec<OAuthDraftSnapshot>,
    /// Selected row in the OAuth submenu's connections list (`#[serde(default)]`
    /// keeps an older peer's snapshot decoding cleanly).
    #[serde(default)]
    pub oauth_sel: usize,
    #[serde(default)]
    pub oauth_armed: Option<usize>,
    #[serde(default)]
    pub oauth_flow: OAuthFlowSnapshot,
    pub prov_sel: usize,
    pub prov_delete_armed: bool,
    pub prov_modal: Option<ProviderModalSnapshot>,
    pub models: Vec<ModelDraftSnapshot>,
    pub model_sel: usize,
    pub model_delete_armed: bool,
    pub model_modal: Option<ModelModalSnapshot>,
    /// Wire token for [`ModelFilterMode`]: `"all"`, `"local"`, or `"global"`.
    #[serde(default)]
    pub model_filter: String,
}

// -- mode payload projections (stage 3: secondary full-screen views) -----------

/// A serde-safe projection of the /usage dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct UsageSnapshot {
    pub view: String,
    pub range: String,
    pub metric: String,
    pub data: crate::model::usage::UsageData,
}

/// A serde-safe projection of one message-rewind entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RewindEntrySnapshot {
    pub vec_index: usize,
    pub content: String,
}

/// A serde-safe projection of the message-rewind picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RewindSnapshot {
    pub entries: Vec<RewindEntrySnapshot>,
    pub selected: usize,
}

/// A serde-safe projection of the /effort reasoning-effort picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct EffortSnapshot {
    pub options: Vec<String>,
    pub selected: usize,
    pub note: String,
}

/// A serde-safe projection of one --resume session-picker row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SessionMetaSnapshot {
    pub id: String,
    pub name: String,
    pub modified_secs: u64,
    pub message_count: usize,
    pub locked: bool,
}

/// A serde-safe projection of the --resume session picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PickerSnapshot {
    pub query: String,
    pub all: Vec<SessionMetaSnapshot>,
    pub filtered_idx: Vec<usize>,
    pub selected: usize,
}

/// A serde-safe projection of a registered model entry, KEYLESS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CatalogueModelSnapshot {
    pub uuid: String,
    pub name: String,
    pub model_id: String,
    pub provider_uuid: String,
}

/// A serde-safe projection of an API-provider connection, KEYLESS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CatalogueProviderSnapshot {
    pub uuid: String,
    pub name: String,
    pub endpoint: String,
}

/// A serde-safe projection of the full-screen nano text editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TextEditorSnapshot {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    pub scroll: usize,
}

/// A serde-safe projection of the /agents tool multi-select picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ToolPickerSnapshot {
    pub options: Vec<String>,
    pub checked: Vec<bool>,
    pub cursor: usize,
    pub filter: String,
}

/// A serde-safe projection of the /agents single-select model picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct AgentModelPickerSnapshot {
    pub options: Vec<(Option<String>, String)>,
    pub cursor: usize,
}

/// Lightweight agent entry for IPC display.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgentEntry {
    pub name: String,
    pub description: String,
    pub conditions: String,
    /// "session" | "project" | "global" | "builtin"
    pub source: String,
    pub model_uuid: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub prompt: String,
}

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
