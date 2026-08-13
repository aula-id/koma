//! Per-mode shadow reconstruction: one `shadow_*` fn per interactive mode.
//!
//! The `/settings` dashboard cluster (its own reconstruction + modals/drafts +
//! shared value-mappers) lives in the sibling [`super::settings`] module
//! (file size); re-imported here for the two helpers `shadow_onboard_provider`
//! (below) reuses (the guided-onboarding OAuth flow mirrors the Settings
//! OAuth submenu).

use std::time::{Duration, Instant};

use super::settings::{shadow_oauth_flow, shadow_oauth_provider};
use crate::app::mode::agents::{
    AgentEditField, AgentScope, AgentSubMode, AgentsState, ModelPickerState, ToolPickerState,
};
use crate::app::mode::bash::BashState;
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::ext_screen::ExtScreenState;
use crate::app::mode::extensions::{ExtRow, ExtSubMode, ExtTuiScreen, ExtensionsState};
use crate::app::mode::help::{HelpEntry, HelpKind, HelpState};
use crate::app::mode::mcp::{McpEditField, McpState, McpSubMode};
use crate::app::mode::security::SecurityState;
use crate::app::mode::store::{ExtStoreState, StoreDetailData, StoreRow, StoreSubMode};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState,
    OnboardProviderState, OnboardProviderStep, OnboardState, PickerState, RewindEntry, RewindState,
    SessionHub, SessionKind, UsageMetric, UsageNavState, UsageRange, UsageView, WarmStatus,
};
use crate::ipc::proto::{
    AgentEntry, AgentModelPickerSnapshot, AgentsSnapshot, BashSnapshot, EffortSnapshot, ExtRowWire,
    ExtScreenSnapshot, ExtStoreRowWire, ExtStoreSnapshot, ExtensionsSnapshot, HelpSnapshot,
    KeyInputSnapshot, LoadingSnapshot, McpSnapshot, ModelCmdSnapshot, OnboardProviderSnapshot,
    OnboardSnapshot, PickerSnapshot, RemoteSnapshot, RewindSnapshot, SecuritySnapshot,
    SessionHubSnapshot, SkillCmdSnapshot, TextEditorSnapshot, ToolPickerSnapshot, WarmStatusWire,
};
use crate::model::app_config::McpTransport;
use crate::model::store::SessionMeta;

// ─── mode reconstruction (stage 2: core interactive modes) ───────────────────
//
// Each rebuilds a REAL mode-state value from its wire projection so the unmodified
// `view::draw` renders it. The client never mutates these (input is forwarded to
// the daemon); they only need to be faithful enough to DRAW. None hold a channel /
// `Instant`-clock that must keep ticking except `Loading::started`, which is
// re-anchored from the projected elapsed-ms so its footer counter matches.

/// Rebuild the first-run connection chooser ([`OnboardState`]) from its projection.
pub(crate) fn shadow_onboard(o: OnboardSnapshot) -> OnboardState {
    OnboardState { cursor: o.cursor }
}

/// Rebuild the guided provider onboarding wizard ([`OnboardProviderState`]) from its
/// projection. The reused OAuth connect-flow + the provider token decode through the
/// same helpers the Settings shadow uses (unknown provider → Codex, unknown step →
/// Login — never lost). The model-result list is recomputed at render time from the
/// projected catalogue, so it isn't reconstructed here.
pub(crate) fn shadow_onboard_provider(s: OnboardProviderSnapshot) -> OnboardProviderState {
    OnboardProviderState {
        step: match s.step.as_str() {
            "model_select" => OnboardProviderStep::ModelSelect,
            _ => OnboardProviderStep::Login,
        },
        oauth_flow: shadow_oauth_flow(s.oauth_flow),
        new_conn_uuid: s.new_conn_uuid,
        provider: s.provider.as_deref().map(shadow_oauth_provider),
        query: s.query,
        result_sel: s.result_sel,
    }
}

/// Rebuild the first-run wizard form ([`KeyInputForm`]) from its projection.
pub(crate) fn shadow_key_input(f: KeyInputSnapshot) -> KeyInputForm {
    KeyInputForm {
        step: f.step,
        field: f.field,
        endpoint: f.endpoint,
        api_key: f.api_key,
        model: f.model,
        query: f.query,
        result_sel: f.result_sel,
        first_run: f.first_run,
        from_picker: f.from_picker,
    }
}

/// Rebuild the loading splash ([`LoadingState`]) from its projection. The footer's
/// elapsed clock is re-anchored (`now - elapsed`) so it continues from the daemon's
/// phase rather than resetting to 0 on each snapshot.
pub(crate) fn shadow_loading(s: LoadingSnapshot) -> LoadingState {
    LoadingState {
        started: Instant::now() - Duration::from_millis(s.elapsed_ms),
        frame: s.frame,
        workspace: shadow_warm_status(s.workspace),
        awareness: shadow_warm_status(s.awareness),
    }
}

/// Map a [`WarmStatusWire`] back to a [`WarmStatus`].
fn shadow_warm_status(w: WarmStatusWire) -> WarmStatus {
    match w {
        WarmStatusWire::Pending => WarmStatus::Pending,
        WarmStatusWire::Running => WarmStatus::Running,
        WarmStatusWire::Done(d) => WarmStatus::Done(d),
        WarmStatusWire::Skipped => WarmStatus::Skipped,
        WarmStatusWire::Failed => WarmStatus::Failed,
    }
}

/// Rebuild the two-pane session hub ([`SessionHub`]) from its projection.
///
/// The COOKING rows' live `idx` (the daemon's `sessions` index, used on Enter) is
/// not projected and not rendered, so reconstructed rows carry `0` for it; the
/// HISTORY rows' live `path` is likewise daemon-only, rebuilt as an empty path. The
/// client never acts on these — Enter is forwarded for the daemon to resolve.
///
/// The incoming `history` is ALREADY filtered by the daemon, so the shadow's
/// `history_filtered` is rebuilt as the identity over those rows (its render path
/// indexes through it the same way) and `history_selected` passes through unchanged.
/// `history_query` rides along only so the view can echo the search line; the client
/// never re-filters (the daemon owns that). `pending_kill` carries the targeted
/// session's UUID, so the confirm bar searches `cooking` for a matching `session_id`.
pub(crate) fn shadow_session_hub(h: SessionHubSnapshot) -> SessionHub {
    let history: Vec<HistoryEntry> = h
        .history
        .into_iter()
        .map(|e| HistoryEntry {
            path: std::path::PathBuf::new(), // daemon-side load target; not rendered
            name: e.name,
            last_active: std::time::UNIX_EPOCH + Duration::from_secs(e.last_active_secs),
            dir_label: String::new(), // not projected over the wire; labels shown on daemon side
            is_current_dir: false,
        })
        .collect();
    // The projected history is already filtered → identity filter over it.
    let history_filtered: Vec<usize> = (0..history.len()).collect();
    SessionHub {
        cooking: h
            .cooking
            .into_iter()
            .map(|c| CookingEntry {
                idx: 0, // daemon-side index; not rendered, resolved on the daemon
                kind: match c.kind.as_str() {
                    "new_session" => SessionKind::NewSession,
                    _ => SessionKind::Session,
                },
                name: c.name,
                working: c.working,
                is_foreground: c.is_foreground,
                // Carried from the wire so the client-side confirm bar can resolve the
                // armed target by session UUID (matching the daemon handler's identity-based
                // `pending_kill`). None for the synthetic `[+ new session]` row.
                session_id: c.session_id,
                dir_label: String::new(), // not projected over the wire
                is_current_dir: false,
                remote_host: c.remote_host,
            })
            .collect(),
        history,
        focus: if h.focus_cooking {
            HubPane::Cooking
        } else {
            HubPane::History
        },
        cooking_selected: h.cooking_selected,
        history_selected: h.history_selected,
        history_query: h.history_query,
        history_filtered,
        pending_kill: h.pending_kill,
        pending_delete: None,
    }
}

// ─── mode reconstruction (stage 3: secondary full-screen views) ──────────────

/// Rebuild the `--resume` session picker ([`PickerState`]) from its projection.
///
/// Constructed as a struct literal (NOT `PickerState::new`, which would re-run the
/// filter against a freshly-discovered local session list): the daemon's `all`
/// metadata + the `filtered_idx` it computed are carried verbatim so the SAME rows
/// render. Each row's `PathBuf` (the daemon-side load target) is rebuilt empty — the
/// client never loads it (Enter is forwarded), and the picker view doesn't render it.
pub(crate) fn shadow_picker(p: PickerSnapshot) -> PickerState {
    PickerState {
        query: p.query,
        all: p
            .all
            .into_iter()
            .map(|m| SessionMeta {
                id: m.id,
                name: m.name,
                path: std::path::PathBuf::new(), // daemon-side load target; not rendered
                modified: std::time::UNIX_EPOCH + Duration::from_secs(m.modified_secs),
                message_count: m.message_count,
                locked: m.locked,
                workdir: String::new(), // not projected over the wire
                pwd_hash: String::new(),
            })
            .collect(),
        filtered_idx: p.filtered_idx,
        selected: p.selected,
    }
}

/// Rebuild the `/effort` reasoning-effort picker ([`EffortPickerState`]) from its
/// projection (all plain data the overlay reads).
pub(crate) fn shadow_effort(e: EffortSnapshot) -> EffortPickerState {
    EffortPickerState {
        options: e.options,
        selected: e.selected,
        note: e.note,
    }
}

/// Rebuild the `/model` session model switcher ([`ModelCmdState`]) from its
/// projection (all plain data the overlay reads).
pub(crate) fn shadow_model_cmd(s: ModelCmdSnapshot) -> crate::app::mode::ModelCmdState {
    use crate::app::mode::ModelCmdSub;
    use crate::model::app_config::ModelRole;

    let sub = match s.sub.as_str() {
        "role_pick" => {
            let role = match s.role.as_deref() {
                Some("main") => ModelRole::Main,
                Some("awareness") => ModelRole::Awareness,
                Some("planner") => ModelRole::Planner,
                Some("compactor") => ModelRole::Compactor,
                Some("safeguard") => ModelRole::Safeguard,
                _ => ModelRole::Main,
            };
            ModelCmdSub::RolePick { role }
        }
        "agent_list" => ModelCmdSub::AgentList,
        "agent_pick" => ModelCmdSub::AgentPick {
            agent_name: s.agent_name.unwrap_or_default(),
            current_model: None,
        },
        _ => ModelCmdSub::Help { lines: s.lines },
    };

    crate::app::mode::ModelCmdState {
        sub,
        options: s.options,
        cursor: s.cursor,
        note: s.note,
    }
}

/// Rebuild the `/usage` dashboard nav state ([`UsageNavState`]) from its wire tokens.
/// The dashboard's DATA is seeded separately into `rest.usage_data` (it crosses on the
/// same `UsageSnapshot`), so this only restores the view/range/metric selections.
pub(crate) fn shadow_usage_nav(view: &str, range: &str, metric: &str) -> UsageNavState {
    UsageNavState {
        view: match view {
            "session" => UsageView::Session,
            _ => UsageView::Global,
        },
        range: match range {
            "week" => UsageRange::Week,
            "year" => UsageRange::Year,
            _ => UsageRange::Today,
        },
        metric: match metric {
            "tokens" => UsageMetric::Tokens,
            _ => UsageMetric::Cost,
        },
    }
}

/// Rebuild the message-rewind picker ([`RewindState`]) from its projection — the
/// newest-first entry list + the cursor.
pub(crate) fn shadow_rewind(rw: RewindSnapshot) -> RewindState {
    RewindState {
        entries: rw
            .entries
            .into_iter()
            .map(|e| RewindEntry {
                vec_index: e.vec_index,
                content: e.content,
            })
            .collect(),
        selected: rw.selected,
    }
}

/// Rebuild the `/agents` dashboard ([`AgentsState`]) from its projection.
///
/// Restores the agent list, the working drafts + sub-mode + field cursor (from wire
/// tokens), the three overlays, and a minimal `session_dir` (empty — the client never
/// saves). The KEYLESS model+provider catalogue is folded into `rest.config` by the
/// caller's `shadow_settings`-style path? No — it is reconstructed HERE into a private
/// `AppConfig` the agents view resolves the model label against, so the client renders
/// `name @ provider` exactly as the daemon would WITHOUT any API key. The reconstructed
/// state is render-only; key handling is forwarded to the daemon.
pub(crate) fn shadow_agents(a: AgentsSnapshot) -> AgentsState {
    AgentsState {
        agents: a
            .agents
            .into_iter()
            .map(|e: AgentEntry| crate::model::agent_def::AgentDef {
                name: e.name,
                description: e.description,
                conditions: e.conditions,
                source: match e.source.as_str() {
                    "global" => crate::model::agent_def::AgentSource::Global,
                    "builtin" => crate::model::agent_def::AgentSource::Builtin,
                    _ => crate::model::agent_def::AgentSource::Session,
                },
                model_uuid: e.model_uuid,
                model: e.model,
                tools: e.tools,
                prompt: e.prompt,
                file_path: None,
                ..crate::model::agent_def::AgentDef::default()
            })
            .collect(),
        list_sel: a.list_sel,
        in_detail: a.in_detail,
        mode: match a.mode.as_str() {
            "edit" => AgentSubMode::Edit,
            "create" => AgentSubMode::Create,
            "delete_confirm" => AgentSubMode::DeleteConfirm,
            _ => AgentSubMode::Browse,
        },
        field: shadow_agent_field(&a.field),
        editing: a.editing,
        create_scope: match a.create_scope.as_str() {
            "global" => AgentScope::Global,
            _ => AgentScope::Session,
        },
        draft_name: a.draft_name,
        draft_description: a.draft_description,
        draft_conditions: a.draft_conditions,
        draft_model_uuid: a.draft_model_uuid,
        draft_model_legacy: a.draft_model_legacy,
        draft_tools: a.draft_tools,
        draft_body: a.draft_body,
        // The session dir is the daemon-side save target; the client never saves, and
        // the view doesn't render it, so an empty path is fine.
        session_dir: std::path::PathBuf::new(),
        tool_picker: a.tool_picker.map(shadow_tool_picker),
        model_picker: a.model_picker.map(shadow_agent_model_picker),
        editor: a
            .editor
            .map(|(field, ed)| (shadow_agent_field(&field), shadow_text_editor(ed))),
        editor_clear_confirm: a.editor_clear_confirm,
    }
}

/// Rebuild the `/agents` tool multi-select picker ([`ToolPickerState`]).
fn shadow_tool_picker(p: ToolPickerSnapshot) -> ToolPickerState {
    ToolPickerState {
        options: p.options,
        checked: p.checked,
        cursor: p.cursor,
        filter: p.filter,
    }
}

/// Rebuild the `/agents` single-select model picker ([`ModelPickerState`]).
fn shadow_agent_model_picker(p: AgentModelPickerSnapshot) -> ModelPickerState {
    ModelPickerState {
        options: p.options,
        cursor: p.cursor,
    }
}

/// Rebuild the full-screen nano editor ([`TextEditorState`]) from its projection. The
/// render-published `wrap_w` cell is re-seeded to `usize::MAX` (its `from_text`
/// default), so before the first client frame every line is one segment — exactly the
/// editor's own safe fallback; the next draw publishes the real width.
fn shadow_text_editor(ed: TextEditorSnapshot) -> TextEditorState {
    TextEditorState {
        lines: ed.lines,
        row: ed.row,
        col: ed.col,
        scroll: ed.scroll,
        wrap_w: std::cell::Cell::new(usize::MAX),
    }
}

/// Rebuild the `/mcp` dashboard ([`McpState`]) from its projection.
///
/// Mirrors [`shadow_agents`]: the server list rides as `McpServerEntry` directly
/// (already serde pure-data) so it is moved in verbatim; the sub-mode / field /
/// transport cursors decode from their wire tokens. The LIVE per-server tool counts
/// land in `shadow_status` (the client has no MCP manager, so the view falls back to
/// this map for its status column). The reconstructed state is render-only — every
/// key is forwarded to the daemon, which owns the real config + persistence.
pub(crate) fn shadow_mcp(s: McpSnapshot) -> McpState {
    McpState {
        servers: s.servers,
        list_sel: s.list_sel,
        in_detail: s.in_detail,
        mode: shadow_mcp_submode(&s.mode),
        field: shadow_mcp_field(&s.field),
        editing: s.editing,
        draft_uuid: s.draft_uuid,
        draft_name: s.draft_name,
        draft_enabled: s.draft_enabled,
        draft_transport: shadow_mcp_transport(&s.draft_transport),
        draft_command: s.draft_command,
        draft_args: s.draft_args,
        draft_env: s.draft_env,
        draft_url: s.draft_url,
        // The projected live status — this is the client's only status source.
        shadow_status: Some(s.status),
    }
}

/// Map an `/mcp` sub-mode wire token back to an [`McpSubMode`] (unknown → Browse,
/// the read-only default — never lost).
fn shadow_mcp_submode(m: &str) -> McpSubMode {
    match m {
        "edit" => McpSubMode::Edit,
        "create" => McpSubMode::Create,
        "delete_confirm" => McpSubMode::DeleteConfirm,
        _ => McpSubMode::Browse,
    }
}

/// Map an `/mcp` field wire token back to an [`McpEditField`] (unknown → Name, the
/// editor's first field — never lost).
fn shadow_mcp_field(f: &str) -> McpEditField {
    match f {
        "enabled" => McpEditField::Enabled,
        "transport" => McpEditField::Transport,
        "command" => McpEditField::Command,
        "args" => McpEditField::Args,
        "env" => McpEditField::Env,
        "url" => McpEditField::Url,
        _ => McpEditField::Name,
    }
}

/// Map an `/mcp` transport wire token back to an [`McpTransport`] (unknown → Stdio,
/// the default transport).
fn shadow_mcp_transport(t: &str) -> McpTransport {
    match t {
        "http" => McpTransport::Http,
        _ => McpTransport::Stdio,
    }
}

/// Rebuild the `/extension` dashboard ([`ExtensionsState`]) from its projection.
///
/// Mirrors [`shadow_mcp`]: the rows are pure data moved in verbatim (per-row via
/// [`shadow_ext_row`]); the sub-mode cursor decodes from its wire token. Render-only — every
/// key is forwarded to the daemon, which owns the registry + uninstall path.
pub(crate) fn shadow_extensions(s: ExtensionsSnapshot) -> ExtensionsState {
    ExtensionsState {
        rows: s.rows.into_iter().map(shadow_ext_row).collect(),
        list_sel: s.list_sel,
        sub_mode: shadow_ext_submode(&s.mode),
        screen_sel: s.screen_sel,
        error: s.error,
    }
}

/// Rebuild ONE installed-extension row from its wire mirror (all pure data, moved verbatim).
fn shadow_ext_row(w: ExtRowWire) -> ExtRow {
    ExtRow {
        id: w.id,
        name: w.name,
        version: w.version,
        tier: w.tier,
        kind: w.kind,
        enabled: w.enabled,
        running: w.running,
        description: w.description,
        granted: w.granted,
        tools: w.tools,
        panels: w.panels,
        sub_agents: w.sub_agents,
        models: w.models,
        tui_screens: w
            .tui_screens
            .into_iter()
            .map(|t| ExtTuiScreen {
                id: t.id,
                title: t.title,
            })
            .collect(),
        workspace_dir: w.workspace_dir,
    }
}

/// Map an `/extension` sub-mode wire token back to an [`ExtSubMode`] (unknown → Browse,
/// the read-only default — never lost).
fn shadow_ext_submode(m: &str) -> ExtSubMode {
    match m {
        "detail" => ExtSubMode::Detail,
        "uninstall_confirm" => ExtSubMode::UninstallConfirm,
        _ => ExtSubMode::Browse,
    }
}

/// Rebuild an open extension screen ([`ExtScreenState`]) from its projection. The opaque
/// `Screen` value moves in verbatim; the menu cursor + loading/error flags ride as-is.
/// Render-only — keys are forwarded to the daemon, which owns the invoke + reply/push fold.
pub(crate) fn shadow_ext_screen(s: ExtScreenSnapshot) -> ExtScreenState {
    ExtScreenState {
        ext_id: s.ext_id,
        screen_id: s.screen_id,
        screen_title: s.screen_title,
        screen: s.screen,
        menu_cursor: s.menu_cursor,
        waiting: s.waiting,
        error: s.error,
    }
}

/// Rebuild ONE `/store` catalogue row from its wire mirror (all pure data, moved verbatim).
fn shadow_store_row(w: ExtStoreRowWire) -> StoreRow {
    StoreRow {
        id: w.id,
        name: w.name,
        tagline: w.tagline,
        tier: w.tier,
        kind: w.kind,
        latest_version: w.latest_version,
        author: w.author,
        installed: w.installed,
    }
}

/// Map a `/store` sub-mode wire token back to a [`StoreSubMode`] (unknown → Browse, the
/// read-only default — never lost).
fn shadow_store_submode(m: &str) -> StoreSubMode {
    match m {
        "detail" => StoreSubMode::Detail,
        "install_confirm" => StoreSubMode::InstallConfirm,
        _ => StoreSubMode::Browse,
    }
}

/// Rebuild the `/store` marketplace browser ([`ExtStoreState`]) from its projection.
///
/// Mirrors [`shadow_extensions`]: the rows are pure data moved in verbatim (per-row via
/// [`shadow_store_row`]); the sub-mode cursor decodes from its wire token. Render-only —
/// every key is forwarded to the daemon, which owns the network fetches + install path.
pub(crate) fn shadow_ext_store(s: ExtStoreSnapshot) -> ExtStoreState {
    ExtStoreState {
        sub_mode: shadow_store_submode(&s.mode),
        rows: s.rows.into_iter().map(shadow_store_row).collect(),
        list_sel: s.list_sel,
        loading: s.loading,
        error: s.error,
        detail: s.detail.map(|d| StoreDetailData {
            description: d.description,
            contributes_models: d.contributes_models,
            contributes_panels: d.contributes_panels,
            contributes_tools: d.contributes_tools,
            contributes_sub_agents: d.contributes_sub_agents,
            requires: d.requires,
            versions: d.versions,
        }),
        detail_loading: s.detail_loading,
        detail_error: s.detail_error,
        installing: s.installing,
        install_error: s.install_error,
        komarun_connected: s.komarun_connected,
    }
}

/// Rebuild the `/skill` hub ([`SkillCmdState`]) from its projection.
///
/// Mirrors [`shadow_help`]: built as a struct literal so the daemon's query +
/// filtered_idx + selected + chip are preserved exactly. Render-only — every key
/// is forwarded to the daemon.
pub(crate) fn shadow_skill_cmd(s: SkillCmdSnapshot) -> crate::app::mode::SkillCmdState {
    use crate::app::mode::skill_cmd::{SkillCmdState, SkillEntry, SkillFilterChip};
    crate::app::mode::SkillCmdState {
        query: s.query,
        chip: match s.chip.as_str() {
            "active" => SkillFilterChip::Active,
            _ => SkillFilterChip::All,
        },
        all: s
            .all
            .into_iter()
            .map(|e| SkillEntry {
                name: e.name,
                description: e.description,
                is_active: e.is_active,
            })
            .collect(),
        filtered_idx: s.filtered_idx,
        selected: s.selected,
    }
}

/// Rebuild the `/help` reference ([`HelpState`]) from its projection.
///
/// Mirrors [`shadow_picker`]: built as a struct literal (NOT `HelpState::new`, which
/// would re-aggregate the COMMANDS/KEYBINDINGS registries and discard the daemon's
/// `query` + `filtered_idx` + `selected`) so the SAME filtered rows + cursor render.
/// Each entry's `kind` decodes from its wire token. Render-only — every key is
/// forwarded to the daemon, which owns the real launch behaviour.
pub(crate) fn shadow_help(s: HelpSnapshot) -> HelpState {
    HelpState {
        query: s.query,
        all: s
            .all
            .into_iter()
            .map(|e| HelpEntry {
                kind: shadow_help_kind(&e.kind),
                key: e.key,
                desc: e.desc,
            })
            .collect(),
        filtered_idx: s.filtered_idx,
        selected: s.selected,
        current_version: s.current_version,
        update: s.update,
    }
}

/// Map a `/help` kind wire token back to a [`HelpKind`] (unknown → Command, the
/// launchable default — never lost).
fn shadow_help_kind(k: &str) -> HelpKind {
    match k {
        "keybinding" => HelpKind::Keybinding,
        _ => HelpKind::Command,
    }
}

/// Map an `/agents` field wire token back to an [`AgentEditField`] (unknown →
/// Description, the editor's default focus — never lost).
fn shadow_agent_field(f: &str) -> AgentEditField {
    match f {
        "name" => AgentEditField::Name,
        "conditions" => AgentEditField::Conditions,
        "model" => AgentEditField::Model,
        "tools" => AgentEditField::Tools,
        "prompt" => AgentEditField::Body,
        _ => AgentEditField::Description,
    }
}

/// Rebuild the `/security` control panel ([`SecurityState`]) from its projection.
///
/// The status snapshot rides verbatim (already serde-safe); the cursor is restored
/// as-is. Render-only — every key is forwarded to the daemon.
pub(crate) fn shadow_security(s: SecuritySnapshot) -> SecurityState {
    SecurityState {
        status: s.status,
        selected: s.selected,
        // The projected inactive set rides as a sorted Vec; rebuild the HashSet the
        // view + render path read from.
        inactive: s.inactive.into_iter().collect(),
        // YOLO arm flag rides verbatim so the client's panel renders the armed row.
        yolo_armed: s.yolo_armed,
        // Install-health + the pane toggle + its cursor ride verbatim so the client
        // renders the same dependency pane the daemon would.
        install_health: s.install_health,
        health_view: s.health_view,
        health_selected: s.health_selected,
        // Spinner state rides verbatim — the client renders + animates the
        // "checking dependencies…" line from the daemon's projected frame counter.
        health_fetching: s.health_fetching,
        health_frame: s.health_frame,
    }
}

/// Rebuild the `/bash` background-job panel ([`BashState`]) from its projection.
///
/// The job views + the list cursor ride verbatim (already serde-safe, pre-rendered
/// data). Render-only — the client never mutates it; every key is forwarded to the
/// daemon, which owns the real registry + kill path.
pub(crate) fn shadow_bash(s: BashSnapshot) -> BashState {
    BashState {
        jobs: s.jobs,
        selected: s.selected,
    }
}

pub(crate) fn shadow_todo(s: crate::ipc::proto::TodoSnapshot) -> crate::app::mode::TodoState {
    use crate::app::mode::todo::{TodoItem, TodoPriority, TodoStatus};
    crate::app::mode::TodoState {
        items: s
            .items
            .into_iter()
            .map(|item| TodoItem {
                content: item.content,
                status: TodoStatus::from_str(&item.status),
                priority: TodoPriority::from_str(&item.priority),
                locked: item.locked,
            })
            .collect(),
        selected: s.selected,
        pwd_hash: s.pwd_hash,
        // Daemon-only field (the plan-todos path). The client never reads or
        // writes the backing file directly — every key is forwarded to the
        // daemon, which owns `refresh_from_disk`/`reset_to_pending` — so it's
        // intentionally NOT part of `TodoSnapshot` and defaults to `None` here.
        plan_path: None,
        last_refresh: std::time::Instant::now(),
    }
}

/// Rebuild the `/remote` host manager ([`RemoteState`]) from its projection.
pub(crate) fn shadow_remote(s: RemoteSnapshot) -> crate::app::mode::RemoteState {
    use crate::app::mode::remote::{ConnectionStatus, ConnectStage, RemoteSession, RemoteState, RemoteSub};
    RemoteState {
        sub: match s.sub.as_str() {
            "fullscreen" => RemoteSub::Fullscreen,
            "connecting" => RemoteSub::Connecting,
            "password" => RemoteSub::PasswordInput,
            _ => RemoteSub::Compact,
        },
        hosts: s
            .hosts
            .into_iter()
            .map(|h| crate::remote::hosts::RemoteHost {
                id: h.id,
                name: h.name,
                user: h.user,
                host: h.host,
                port: h.port,
                key_path: h.key_path,
                last_connected: h.last_connected,
                tags: h.tags,
            })
            .collect(),
        selected: s.selected,
        query: s.query,
        filtered: s.filtered,
        detail_host: s.detail_host,
        connection_status: s.stage.map(|stage_str| ConnectionStatus {
            stage: match stage_str.as_str() {
                "authenticating" => ConnectStage::Authenticating,
                "bootstrapping" => ConnectStage::Bootstrapping,
                "connected" => ConnectStage::Connected,
                _ => ConnectStage::Resolving,
            },
            error: s.error,
        }),
        sessions: s
            .sessions
            .into_iter()
            .map(|ss| RemoteSession {
                session_id: ss.session_id,
                name: ss.name,
                working: ss.working,
                is_foreground: ss.is_foreground,
            })
            .collect(),
        session_selected: s.session_selected,
        pending_delete: s.pending_delete,
        password_buf: String::new(),
        connecting_host: None,
    }
}
