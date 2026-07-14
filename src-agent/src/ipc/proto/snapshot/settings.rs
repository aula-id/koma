// ─── /settings dashboard + stage-3 secondary full-screen view payloads ────────

use serde::{Deserialize, Serialize};

use super::connector::{
    ModelDraftSnapshot, ModelModalSnapshot, OAuthDraftSnapshot, OAuthFlowSnapshot,
    ProviderDraftSnapshot, ProviderModalSnapshot,
};

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
    /// Active palette name (see `view::theme::PALETTES`). `#[serde(default)]`
    /// keeps an older peer's snapshot (pre-palette) decoding cleanly — a missing
    /// value falls back to the dark palette at render time.
    #[serde(default)]
    pub palette: String,
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
    #[serde(default)]
    pub coding_autosave: bool,
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
    /// Cursor index into `view::theme::PALETTES` for the Appearance palette list.
    /// `#[serde(default)]` keeps an older peer's snapshot decoding cleanly (→ 0).
    #[serde(default)]
    pub palette_sel: usize,
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
