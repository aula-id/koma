//! Per-session configuration persisted to `settings.json`.
//!
//! Each session directory contains exactly one `settings.json` that is read
//! on `Session::load` and written on every `Session::save`. The file is
//! human-editable — users can change the model or tweak compaction settings
//! without touching the TUI.
//!
//! **API key storage:** the key is stored per-session by design. This lets
//! users run separate sessions against different OpenRouter accounts (e.g. a
//! personal key vs. a work key) without a global config file. An empty string
//! means "not configured"; the UI will prompt before the first send.
//!
//! On-disk format (pretty-printed JSON):
//! ```json
//! {
//!   "api_key": "sk-or-...",
//!   "model": "openai/gpt-4o",
//!   "name": "my-session",
//!   "compaction": { "preserve_n": 20 }
//! }
//! ```

use std::fmt;
use std::path::Path;
use anyhow::Result;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use crate::config::{
    DEFAULT_AWARENESS_MODEL, DEFAULT_AWARENESS_PROVIDER, DEFAULT_CLASSIFIER_MODEL,
    DEFAULT_CLASSIFIER_PROVIDER, DEFAULT_MODEL, DEFAULT_PRESERVE_N, DEFAULT_PROVIDER,
};

/// Backwards-compatible deserializer for a field that is now a `Vec<String>`
/// but historically may have been written as a plain JSON string.
///
/// Accepts either form so OLD `settings.json` files (where e.g. `workdir` was a
/// single string) still load cleanly:
/// - a JSON string `"…"`  → `vec!["…".to_string()]`
/// - a JSON array `["…"]` → the sequence as a `Vec<String>` (verbatim)
///
/// Serialisation is unaffected: the field always writes back as an array. An
/// empty string deserialises to `vec![""]`; callers trim/drop empties downstream.
fn string_or_vec<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string or a sequence of strings")
        }

        // Old format: a single bare string becomes a one-element vec.
        fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![v.to_string()])
        }

        // Owned-string variant (some deserializers hand ownership directly).
        fn visit_string<E>(self, v: String) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![v])
        }

        // New format: a sequence is collected into the vec verbatim.
        fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::new();
            while let Some(item) = seq.next_element::<String>()? {
                out.push(item);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

/// Controls conversation compaction behaviour (the `/compact` command).
///
/// When the history grows long, the oldest messages are summarised and
/// replaced. `preserve_n` determines how many of the most-recent messages are
/// kept verbatim after compaction (the "tail" that stays in full detail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compaction {
    #[serde(default = "default_preserve_n")]
    pub preserve_n: usize,
}

fn default_preserve_n() -> usize {
    DEFAULT_PRESERVE_N
}

impl Default for Compaction {
    fn default() -> Self {
        Self {
            preserve_n: DEFAULT_PRESERVE_N,
        }
    }
}

/// Controls which internet-access tier is active for this session.
///
/// `Simple` (the default) keeps `web_search` (DDG) and `web_fetch` on the
/// lightweight in-process raw-HTTP path (low overhead, no subprocess). `Full`
/// upgrades `web_fetch` to the opt-in scrapion browser backend (a Firefox
/// subprocess that renders JS and beats Cloudflare), which costs more tokens
/// and spawns an external process; `web_search` is unchanged. Full requires the
/// environment to be provisioned (`koma --internet-fullmode-install`); until
/// then `web_fetch` silently stays on the raw-HTTP path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InternetMode {
    #[default]
    Simple,
    Full,
}

impl InternetMode {
    /// Lowercase label / wire token (`"simple"` | `"full"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Full => "full",
        }
    }

    /// The opposite mode (Simple <-> Full).
    pub fn toggled(self) -> Self {
        match self {
            Self::Simple => Self::Full,
            Self::Full => Self::Simple,
        }
    }

    /// Parse a user token (case-insensitive, trimmed). `None` for empty or
    /// anything unrecognised — callers treat that as "toggle".
    pub fn from_token(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "simple" => Some(Self::Simple),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

impl std::fmt::Display for InternetMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-session user-configurable settings.
///
/// Deserialized from (and serialized to) `<session_dir>/settings.json`.
/// All fields have serde defaults so a partially-written or newly-created
/// file deserialises without error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // api_key is intentionally ALWAYS serialised (no skip_serializing_if),
    // even when empty, so the on-disk round-trip is unambiguous — an absent
    // key in JSON would deserialise to "" via the `default` attribute, but
    // re-serialising would then omit it, creating a surprising diff.
    #[serde(default)]
    pub api_key: String,
    /// OpenRouter model identifier, e.g. `"openai/gpt-4o"`.
    #[serde(default = "default_model")]
    pub model: String,
    /// Human-readable session name (also used as the directory slug after
    /// `rename_session` normalises it).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub compaction: Compaction,
    /// OpenRouter provider slug to strict-pin (e.g. `"anthropic"`, `"together"`).
    /// Empty string means use OpenRouter default routing; the `provider` field is
    /// then omitted from the request body entirely.
    #[serde(default)]
    pub provider: String,
    /// Reasoning/thinking effort for the interactive chat, set via `/effort`.
    /// Free-form token: `""` (default) = model default, `"off"`/`"none"` =
    /// thinking off, or an effort level (`"low"`/`"high"`/`"max"`/…). Mapped to
    /// the request `reasoning` object by the OpenRouter client. Defaults to `""`
    /// so old `settings.json` files load unchanged.
    #[serde(default)]
    pub effort: String,
    /// Working directories for this session, as a managed path list. The FIRST
    /// non-empty entry is the effective workdir (see `Session::workdir`); the
    /// rest also count toward the harness workspace allow-set. Seeded with the
    /// process's cwd at session creation time. Used to locate AGENT.md / AGENTS.md.
    ///
    /// `string_or_vec` keeps OLD configs loadable: a plain string `"…"` is read
    /// as `vec!["…"]`; arrays load verbatim. Always serialised as an array.
    #[serde(default, deserialize_with = "string_or_vec")]
    pub workdir: Vec<String>,
    /// Whether the project-awareness summary is generated and injected into the
    /// system prompt. When false, no secondary-model call is made.
    #[serde(default = "default_awareness_enabled")]
    pub awareness_enabled: bool,
    /// Awareness model source. `false` (default) uses the dedicated
    /// `awareness_model` / `awareness_provider` below; `true` reuses this
    /// session's own `model` / `provider` for the summary call.
    #[serde(default = "default_awareness_inherit")]
    pub awareness_inherit: bool,
    /// Dedicated model for the awareness summary when `awareness_inherit` is
    /// false. A small/cheap model is plenty for a few-sentence summary.
    #[serde(default = "default_awareness_model")]
    pub awareness_model: String,
    /// Dedicated provider slug (strict-pinned) for the awareness summary when
    /// `awareness_inherit` is false. Empty means OpenRouter default routing.
    #[serde(default = "default_awareness_provider")]
    pub awareness_provider: String,
    /// Master switch for the safety harness ("Pass B"). When false (the
    /// default), the agentic loop behaves EXACTLY as it did before the harness
    /// existed: no workspace check, no prompt/tool-call classification, no
    /// secondary-model calls. Opt-in only.
    #[serde(default = "default_classifier_enabled")]
    pub classifier_enabled: bool,
    /// Model used for the safety classifier (prompt + tool-call verdicts).
    /// A dedicated safeguard model judges whether a request/call is safe.
    #[serde(default = "default_classifier_model")]
    pub classifier_model: String,
    /// Provider slug (strict-pinned) for the classifier call. Empty means
    /// OpenRouter default routing.
    #[serde(default = "default_classifier_provider")]
    pub classifier_provider: String,
    /// Extra folders the session is allowed to operate in, beyond the launch
    /// directory (which is always allowed at runtime). The workspace check (WC)
    /// passes when the session workdir is the launch dir OR appears here. Empty
    /// by default; ignored entirely when `classifier_enabled` is false.
    #[serde(default)]
    pub allowed_folders: Vec<String>,
    /// Master switch for the "short-send" payload reshaper. When true (the
    /// default), the API-bound history is compressed before each send: the older
    /// turns are replaced by a rolling summary + a verbatim tail, with heavy
    /// blobs rehydrated on demand. When false the full history is sent as before
    /// (kill switch). Display + on-disk state are unaffected either way.
    #[serde(default = "default_short_send_enabled")]
    pub short_send_enabled: bool,
    /// How many of the newest messages short-send keeps verbatim (the tail that
    /// is sent in full; everything older is folded into the summary). Defaults to
    /// `6`. Old `settings.json` files load unchanged via the serde default.
    #[serde(default = "default_short_send_tail_n")]
    pub short_send_tail_n: i64,
    /// Enable cache-warmth-adaptive summarization. When true, the runtime may
    /// trigger a sliding-window summary when it detects the prompt cache has gone
    /// cold, keeping costs low on providers with a sliding/refreshing prompt cache
    /// (e.g. Anthropic). When false (the default), no such adaptation is attempted.
    #[serde(default = "default_sliding_cache")]
    pub sliding_cache: bool,
    /// Internet-access tier for this session. `Simple` (default) keeps
    /// `web_fetch`/`web_search` on the lightweight in-process raw-HTTP path.
    /// `Full` upgrades `web_fetch` to the opt-in scrapion browser backend
    /// (Firefox subprocess, higher token usage); `web_search` is unchanged.
    #[serde(default = "default_internet_mode")]
    pub internet_mode: InternetMode,
    /// Per-session override layer for the global model catalogue: models the user
    /// saved for THIS session only (the `/settings` "Save session" path). They are
    /// never written to the global `config.json`; they live here so they survive a
    /// reload without leaking into other sessions. Mirrors the global
    /// [`crate::model::app_config::ModelEntry`] shape (each still references a
    /// provider by uuid).
    ///
    /// `#[serde(skip)]`: this is NO LONGER persisted in the per-session
    /// `settings.json`. In the pwd-keyed layout it lives in the shared
    /// [`LocalConfig`] at [`crate::model::store::shared_settings_path`] (one
    /// catalogue per working directory). The field stays in the in-memory struct
    /// so `resolve_role`/`resolve_agent` are unchanged; `Session::load` overlays it
    /// from the shared file and `Session::save` writes it back there.
    #[serde(skip)]
    pub session_models: Vec<crate::model::app_config::ModelEntry>,
    /// The bare filename of the SSH identity key this session uses for
    /// git-over-SSH (e.g. `"id_ed25519"`). Stored as a filename only — never a
    /// full path, never key file contents. `None` means no key has been selected;
    /// the `git_cred` tool lists available keys (from `~/.ssh`) and sets this
    /// field when the user calls `action="select"`.
    #[serde(default)]
    pub git_ssh_key: Option<String>,
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_awareness_enabled() -> bool {
    true
}

fn default_awareness_inherit() -> bool {
    false
}

fn default_awareness_model() -> String {
    DEFAULT_AWARENESS_MODEL.to_string()
}

fn default_awareness_provider() -> String {
    DEFAULT_AWARENESS_PROVIDER.to_string()
}

fn default_classifier_enabled() -> bool {
    true
}

fn default_classifier_model() -> String {
    DEFAULT_CLASSIFIER_MODEL.to_string()
}

fn default_classifier_provider() -> String {
    DEFAULT_CLASSIFIER_PROVIDER.to_string()
}

fn default_short_send_enabled() -> bool {
    true
}

fn default_short_send_tail_n() -> i64 {
    6
}

fn default_sliding_cache() -> bool {
    false
}

fn default_internet_mode() -> InternetMode {
    InternetMode::Simple
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: DEFAULT_MODEL.to_string(),
            name: String::new(),
            compaction: Compaction::default(),
            provider: DEFAULT_PROVIDER.to_string(),
            effort: String::new(),
            workdir: Vec::new(),
            awareness_enabled: default_awareness_enabled(),
            awareness_inherit: default_awareness_inherit(),
            awareness_model: DEFAULT_AWARENESS_MODEL.to_string(),
            awareness_provider: DEFAULT_AWARENESS_PROVIDER.to_string(),
            classifier_enabled: default_classifier_enabled(),
            classifier_model: DEFAULT_CLASSIFIER_MODEL.to_string(),
            classifier_provider: DEFAULT_CLASSIFIER_PROVIDER.to_string(),
            allowed_folders: Vec::new(),
            short_send_enabled: default_short_send_enabled(),
            short_send_tail_n: default_short_send_tail_n(),
            sliding_cache: default_sliding_cache(),
            internet_mode: InternetMode::Simple,
            session_models: Vec::new(),
            git_ssh_key: None,
        }
    }
}

impl Settings {
    /// Deserialise from a `settings.json` file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Serialise (pretty-printed) to `path`, creating or overwriting the file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// Shared per-working-directory model setup.
///
/// In the pwd-keyed storage layout every session opened from the same working
/// directory shares ONE `settings.json` in the bucket directory (see
/// [`crate::model::store::shared_settings_path`]). That shared file holds only
/// the model catalogue for the directory — the per-session behavioural knobs
/// stay in each session's own [`Settings`]. This is the deserialised form of
/// that shared file.
///
/// Wired in by `Session::load` (overlays `session_models` onto the in-memory
/// [`Settings`]) and `Session::save` (writes this file back). The single
/// `session_models` field carries `#[serde(default)]` so a missing or partially
/// written file loads as an empty catalogue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Model catalogue shared by all sessions in this working directory. Mirrors
    /// the global [`crate::model::app_config::ModelEntry`] shape; each entry still
    /// references a provider by uuid.
    #[serde(default)]
    pub session_models: Vec<crate::model::app_config::ModelEntry>,
}

impl LocalConfig {
    /// Load the shared per-dir config from `path`.
    ///
    /// Returns an empty default when the file is missing or blank (a directory
    /// that has never had its model setup written yet), but propagates a genuine
    /// parse error so a corrupt file is surfaced rather than silently dropped.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return Ok(Self::default()), // absent → empty catalogue
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self::default()); // blank file → empty catalogue
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Serialise (pretty-printed) to `path`, creating or overwriting the file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
