//! App – UI mode enum and associated state types.
//!
//! The app is always in exactly one of five modes, represented by [`Mode`]:
//!
//! | Variant          | Meaning                                       |
//! |-----------------|-----------------------------------------------|
//! | `KeyInput`       | Credentials form (api key + model)            |
//! | `SessionPicker`  | `--resume` session list with live search      |
//! | `Chat`           | Normal conversation view                      |
//! | `Settings`       | In-app `/settings` dashboard                  |
//! | `Effort`         | `/effort` reasoning-effort picker overlay     |
//! | `Usage`          | `/usage` cost and token dashboard             |
//!
//! Mode-specific state is stored inline in the variant so the type system
//! ensures the runtime can only access data that is relevant to the active
//! mode.  [`KeyInputForm`], [`PickerState`], [`SettingsState`], and
//! [`EffortPickerState`] live here; `Chat` carries no extra state beyond
//! `AppStateRest`.

mod key_input;
mod effort;
mod loading;
mod picker;
mod quit_confirm;
mod rewind;
mod session_hub;
pub mod settings;
pub mod agents;
pub mod mcp;
pub mod help;
pub mod editor;
pub mod security;
pub mod bash;

pub use agents::{AgentEditField, AgentScope, AgentSubMode, AgentsState};
pub use mcp::{McpEditField, McpState, McpSubMode};
pub use security::SecurityState;
pub use bash::BashState;
// `HelpEntry` is part of the module's public surface and will be consumed by the
// daemon Help projection (follow-up); re-exported now so that lands without
// re-touching this line. `allow` silences the meanwhile-unused warning.
#[allow(unused_imports)]
pub use help::{HelpEntry, HelpKind, HelpState};
pub use effort::EffortPickerState;
pub use key_input::KeyInputForm;
pub use loading::{LoadingState, WarmStatus};
pub use picker::PickerState;
pub use session_hub::{CookingEntry, HistoryEntry, HubPane, SessionHub, SessionKind};
pub use quit_confirm::QuitConfirmState;
pub use rewind::{RewindEntry, RewindState};
pub use settings::{
    filter_models, SettingField, SettingsState, PICKER_MAX,
    SETTING_CATEGORIES,
};

// ── Usage dashboard nav state ────────────────────────────────────────────────

/// Which top-level view is active in the `/usage` dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageView {
    /// View A: global stats across all sessions (heatmap, KPI, top models…).
    #[default]
    Global,
    /// View B: current-session detail (models used, hourly heatmap, totals).
    Session,
}

/// Date-range selection for View A's KPI strip and panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageRange {
    /// Data from midnight UTC today onwards.
    #[default]
    Today,
    /// Last 7 days.
    Week,
    /// Last 365 days.
    Year,
}

impl UsageRange {
    /// How far back (in seconds from now) the range extends.
    #[allow(dead_code)]
    pub fn since_secs(self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match self {
            // Floor to local midnight so "today" starts at 00:00:00 local time.
            Self::Today => {
                let tz = crate::model::usage::local_utc_offset_secs();
                let local_now = now + tz;
                local_now - local_now % 86400 - tz
            }
            Self::Week  => now - 7 * 86400,
            Self::Year  => now - 365 * 86400,
        }
    }

    /// Short label shown in the range tab bar.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week  => "week",
            Self::Year  => "year",
        }
    }
}

/// Which metric drives the heatmap cell intensity and sparkline scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageMetric {
    /// Intensity proportional to USD cost.
    #[default]
    Cost,
    /// Intensity proportional to token count (in + out).
    Tokens,
}

/// Navigation / display state for the `/usage` dashboard.
///
/// Boxed in the `Mode::Usage` variant to keep the enum small (consistent with
/// `Settings` and `Agents`).
#[derive(Debug, Clone, Default)]
pub struct UsageNavState {
    /// Which top-level view (Global / Session) is shown.
    pub view: UsageView,
    /// Active date range for View A.
    pub range: UsageRange,
    /// Metric that drives heatmap intensity and sparkline scaling.
    pub metric: UsageMetric,
}

// ── Mode enum ────────────────────────────────────────────────────────────────

/// The mutually-exclusive UI modes of the application.
pub enum Mode {
    /// Credentials form: collects api key and model name before a session can
    /// start.  The inner [`KeyInputForm`] holds the in-progress field values
    /// and tracks which field is focused.
    KeyInput(KeyInputForm),
    /// `--resume` session picker: shows saved sessions and a live search bar.
    /// Opened by the `--resume` startup flag and by Esc-out of a picker-launched
    /// KeyInput. The `/resume` COMMAND now opens [`Mode::SessionHub`] instead.
    SessionPicker(PickerState),
    /// Unified two-pane session hub (`/resume`): merges the old `/swap` live picker
    /// and the disk picker into one overlay. The TOP "cooking" pane lists the
    /// currently-LIVE sessions (one per [`crate::app::state::SessionRuntime`] in
    /// `AppStateRest::sessions`), each with a ● working / ○ ready marker and the
    /// foreground one flagged `(current)`; the BOTTOM "history" pane lists the
    /// on-disk sessions MINUS any already live (dedup). Tab toggles the focused
    /// pane; Up/Down move the selection within it; Enter on cooking switches the
    /// foreground (no abort, no lock change), Enter on history loads that session
    /// into a NEW appended tab; Esc closes back to Chat. Boxed to keep `Mode`
    /// small, consistent with the other list variants.
    SessionHub(Box<SessionHub>),
    /// Normal chat view: messages are rendered and the user types in the
    /// input bar.  All chat-specific state lives in `AppStateRest`.
    Chat,
    /// Startup warming splash: a btop-style animated loading screen shown while a
    /// returning-into-Chat session warms ASYNCHRONOUSLY (catalogue fetch + project
    /// awareness summary run as background tasks instead of blocking the UI thread
    /// before the event loop starts). The inner [`LoadingState`] tracks the three
    /// step markers + the spinner frame; the loop switches to `Chat` once the
    /// catalogue + awareness steps are terminal (or the user presses Esc to skip).
    Loading(LoadingState),
    /// In-app settings dashboard (`/settings`): edit per-session credentials,
    /// the session name, and the global theme/accent. The inner
    /// [`SettingsState`] holds working drafts that are applied on save.
    ///
    /// Boxed: `SettingsState` is much larger than the other variants (it carries
    /// every draft plus the path-list/picker working state), so storing it inline
    /// would bloat `Mode` everywhere. The box keeps the enum small.
    Settings(Box<SettingsState>),
    /// In-app agent definitions manager (`/agents`): create / modify / delete
    /// the `.md` frontmatter agent files. The inner [`AgentsState`] holds the
    /// loaded registry snapshot, the LIST/DETAIL cursor, the sub-mode state
    /// machine, and the per-field working drafts. Boxed to keep `Mode` small,
    /// consistent with `Settings`.
    Agents(Box<AgentsState>),
    /// In-app MCP server manager (`/mcp`): create / modify / delete / enable-toggle
    /// the configured MCP servers, persisted to `config.json`. The inner
    /// [`McpState`] holds a snapshot of `config.mcp_servers`, the LIST/DETAIL cursor,
    /// the sub-mode state machine, and the per-field working drafts. A simpler clone
    /// of [`Self::Agents`] (no markdown files, no pickers, no body editor). Boxed to
    /// keep `Mode` small, consistent with `Agents`.
    Mcp(Box<McpState>),
    /// Full-screen, searchable command/keybinding reference + launcher (`/help`):
    /// replaces the old floating help overlay. The inner [`HelpState`] aggregates
    /// every entry in the COMMANDS + KEYBINDINGS registries into one filterable
    /// list (same query/select shape as [`Self::SessionPicker`]). Type to search;
    /// ↑/↓ select; Enter RUNS the highlighted command (keybinding rows are
    /// reference-only); Esc closes back to Chat. Boxed to keep `Mode` small,
    /// consistent with the other list variants.
    Help(Box<HelpState>),
    /// Reasoning/thinking-effort picker (`/effort`): a small overlay listing the
    /// effort options the current model supports. The inner [`EffortPickerState`]
    /// holds the option list, the cursor, and a one-line capability note. Boxed
    /// to keep `Mode` small and consistent with `Settings`.
    Effort(Box<EffortPickerState>),
    /// Cost and token usage dashboard (`/usage`): full-screen Bloomberg-terminal-
    /// style view with two tabs (Global / Session), range selector (1-4), metric
    /// toggle (m), and ESC to exit. The inner [`UsageNavState`] holds the active
    /// view, range, and metric selections. Boxed to keep `Mode` small.
    Usage(Box<UsageNavState>),
    /// Message-rewind picker (double-Esc while idle in Chat): a single-select
    /// list of the conversation's prior USER messages, NEWEST-FIRST, so the top
    /// row is the last message. Up/Down navigate; Esc cancels back to Chat; Enter
    /// rewinds the conversation to just before the chosen message and loads its
    /// text into the composer for editing. The inner [`RewindState`] holds the
    /// entry list and the cursor. Boxed to keep `Mode` small, consistent with the
    /// other list/dashboard variants.
    MessageRewind(Box<RewindState>),
    /// Quit-confirm overlay: shown when the user asks to quit (the `/quit`
    /// command or the quit keybind) while at least one session still has work in
    /// flight. Offers three keyed choices — `k` kill all & quit (abort every
    /// session, release all locks, exit), `d` detach & quit (leave conversations
    /// persisted on disk and exit without aborting), `Esc` cancel back to Chat.
    /// When NOTHING is working the quit happens immediately and this mode is
    /// never entered. The inner [`QuitConfirmState`] only carries the busy-session
    /// count for the warning text. Boxed for consistency with the other overlay
    /// variants (it is small, but the box keeps `Mode` uniform + cheap to move).
    QuitConfirm(Box<QuitConfirmState>),
    /// Security daemon control panel (`/security`): a full-screen status view +
    /// lifecycle controls (start/stop/restart/toggle) + the tool inventory from the
    /// daemon's last handshake. No editor sub-modes — control panel only. The inner
    /// [`SecurityState`] holds the daemon status snapshot + the tool-list cursor.
    /// Boxed to keep `Mode` small, consistent with the other full-screen variants.
    Security(Box<SecurityState>),
    /// Background bash-job panel (`/bash`): a READ-ONLY master/detail view of the
    /// foreground session's background bash jobs (the LEFT pane lists them, the
    /// RIGHT pane shows the selected job's command + status + live output tail).
    /// The only mutating action is killing a running job (`k`) — no editing, no
    /// create/delete, no pickers, no sub-modes. The inner [`BashState`] holds the
    /// (live-projected) job list + the LIST cursor. Boxed to keep `Mode` small,
    /// consistent with the other full-screen variants.
    Bash(Box<BashState>),
}
