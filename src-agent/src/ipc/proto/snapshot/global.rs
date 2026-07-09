// ─── mode-independent global UI state + stage-2 core interactive mode payloads ─

use serde::{Deserialize, Serialize};

use crate::ipc::proto::ModeSnapshot;

use super::connector::OAuthFlowSnapshot;

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
    /// GLOBAL config catalogue projected authoritatively for the native-React GUI's
    /// Connector + MCP panels (the daemon owns `AppConfig`; a thin client can't read
    /// `config.json`). `providers`/`config_models`/`mcp_servers` mirror
    /// `AppConfig.{providers,models,mcp_servers}`; `session_models` is the foreground
    /// session's per-session model override layer (`settings.session_models`, the
    /// "local" scope). The GUI host derives a `Config` push envelope from these; a
    /// change to any of them forces a full snapshot (see `ipc::snapshot::diff`). The
    /// TUI client ignores these fields (its Agents view rebuilds a keyless catalogue
    /// separately), so they add wire data only, no shadow behaviour change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<crate::model::app_config::ProviderConn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_models: Vec<crate::model::app_config::ModelEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_models: Vec<crate::model::app_config::ModelEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<crate::model::app_config::McpServerEntry>,
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
