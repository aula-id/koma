//! Per-mode shadow reconstruction: one `shadow_*` fn per interactive mode.

use std::time::{Duration, Instant};

use crate::app::mode::agents::{
    AgentEditField, AgentScope, AgentSubMode, AgentsState, ModelPickerState, ToolPickerState,
};
use crate::app::mode::help::{HelpEntry, HelpKind, HelpState};
use crate::app::mode::mcp::{McpEditField, McpState, McpSubMode};
use crate::app::mode::security::SecurityState;
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::settings::{
    ModelDraft, ModelModal, PathPicker, PickerMode, ProviderDraft, ProviderModal, RolePickerState,
    SettingsState,
};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState,
    PickerState, RewindEntry, RewindState, SessionHub, SessionKind, UsageMetric, UsageNavState,
    UsageRange, UsageView, WarmStatus,
};
use crate::dto::openrouter::{ModelEndpoint, ModelPricing};
use crate::ipc::proto::{
    AgentEntry, AgentModelPickerSnapshot, AgentsSnapshot, EffortSnapshot, HelpSnapshot,
    KeyInputSnapshot, LoadingSnapshot, McpSnapshot, ModelModalSnapshot, PathPickerSnapshot,
    PickerSnapshot, RewindSnapshot, SecuritySnapshot, SessionHubSnapshot, SettingsSnapshot,
    TextEditorSnapshot, ToolPickerSnapshot, WarmStatusWire,
};
use crate::model::app_config::{ApiType, McpTransport, ModelRole, ThemeMode};
use crate::model::settings::{InternetMode, Settings};
use crate::model::store::SessionMeta;

// ─── mode reconstruction (stage 2: core interactive modes) ───────────────────
//
// Each rebuilds a REAL mode-state value from its wire projection so the unmodified
// `view::draw` renders it. The client never mutates these (input is forwarded to
// the daemon); they only need to be faithful enough to DRAW. None hold a channel /
// `Instant`-clock that must keep ticking except `Loading::started`, which is
// re-anchored from the projected elapsed-ms so its footer counter matches.

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
/// never re-filters (the daemon owns that). `pending_kill` indexes the cooking list,
/// which is order-identical here, so the confirm bar resolves `cooking[pending_kill]`.
pub(crate) fn shadow_session_hub(h: SessionHubSnapshot) -> SessionHub {
    let history: Vec<HistoryEntry> = h
        .history
        .into_iter()
        .map(|e| HistoryEntry {
            path: std::path::PathBuf::new(), // daemon-side load target; not rendered
            name: e.name,
            last_active: std::time::UNIX_EPOCH + Duration::from_secs(e.last_active_secs),
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
    }
}

/// Rebuild the `/settings` dashboard ([`SettingsState`]) from its projection — the
/// largest reconstruction. Every draft + list + modal + picker is restored so the
/// settings view (and its pure helper methods, which recompute from these same
/// fields) renders exactly as the daemon's would.
pub(crate) fn shadow_settings(s: SettingsSnapshot) -> SettingsState {
    SettingsState {
        cat: s.cat,
        field: s.field,
        in_detail: s.in_detail,
        editing: s.editing,
        api_key: s.api_key,
        model: s.model,
        provider: s.provider,
        name: s.name,
        theme: shadow_theme(&s.theme),
        accent: s.accent,
        workdir: s.workdir,
        awareness_enabled: s.awareness_enabled,
        awareness_inherit: s.awareness_inherit,
        awareness_model: s.awareness_model,
        awareness_provider: s.awareness_provider,
        classifier_enabled: s.classifier_enabled,
        classifier_model: s.classifier_model,
        classifier_provider: s.classifier_provider,
        allowed_folders: s.allowed_folders,
        short_send_enabled: s.short_send_enabled,
        sliding_cache: s.sliding_cache,
        internet_mode: shadow_internet_mode(&s.internet_mode),
        cwd: std::path::PathBuf::from(s.cwd),
        list_editing: s.list_editing,
        list_sel: s.list_sel,
        picker: s.picker.map(shadow_path_picker),
        providers: s
            .providers
            .into_iter()
            .map(|p| ProviderDraft {
                uuid: p.uuid,
                name: p.name,
                endpoint: p.endpoint,
                api_type: shadow_api_type(&p.api_type),
                api_key: p.api_key,
            })
            .collect(),
        prov_sel: s.prov_sel,
        prov_delete_armed: s.prov_delete_armed,
        prov_modal: s.prov_modal.map(|m| ProviderModal {
            name: m.name,
            endpoint: m.endpoint,
            api_type: shadow_api_type(&m.api_type),
            api_key: m.api_key,
            field: m.field,
        }),
        models: s
            .models
            .into_iter()
            .map(|m| ModelDraft {
                uuid: m.uuid,
                name: m.name,
                model_id: m.model_id,
                provider_idx: m.provider_idx,
                roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
                route: m.route,
                session_only: m.session_only,
            })
            .collect(),
        model_sel: s.model_sel,
        model_delete_armed: s.model_delete_armed,
        model_modal: s.model_modal.map(shadow_model_modal),
    }
}

/// Rebuild the add/edit-model modal ([`ModelModal`]) from its projection. The
/// endpoints are reconstructed from the serde mirror back into [`ModelEndpoint`]
/// (a `Default`-padded copy carrying just the rendered fields).
fn shadow_model_modal(m: ModelModalSnapshot) -> ModelModal {
    ModelModal {
        editing_idx: m.editing_idx,
        uuid: m.uuid,
        name: m.name,
        provider_idx: m.provider_idx,
        model_id: m.model_id,
        field: m.field,
        roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
        role_picker: m.role_picker.map(|rp| RolePickerState {
            checked: rp.checked,
            cursor: rp.cursor,
        }),
        query: m.query,
        result_sel: m.result_sel,
        route: m.route,
        route_sel: m.route_sel,
        endpoints: m.endpoints.map(|eps| {
            eps.into_iter()
                .map(|ep| ModelEndpoint {
                    name: ep.name,
                    provider_name: ep.provider_name,
                    pricing: Some(ModelPricing {
                        prompt: ep.price_prompt,
                        completion: ep.price_completion,
                    }),
                    context_length: None,
                    quantization: None,
                    max_completion_tokens: None,
                    uptime_last_30m: ep.uptime_last_30m,
                    status: None,
                })
                .collect()
        }),
        endpoints_loading: m.endpoints_loading,
        endpoints_for: m.endpoints_for,
    }
}

/// Rebuild the FS directory picker overlay ([`PathPicker`]) from its projection.
///
/// The matches are the daemon's already-computed `read_dir` results, used VERBATIM
/// (the client never walks its own filesystem — its cwd is unrelated to the
/// daemon's session). Constructed as a struct literal rather than via
/// `PathPicker::new`, which would re-run `list_dirs` against the local FS.
pub(crate) fn shadow_path_picker(p: PathPickerSnapshot) -> PathPicker {
    PathPicker {
        query: p.query,
        matches: p.matches,
        sel: p.sel,
        mode: match p.replace_idx {
            None => PickerMode::Add,
            Some(i) => PickerMode::Replace(i),
        },
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
        agents: a.agents.into_iter().map(|e: AgentEntry| {
            crate::model::agent_def::AgentDef {
                name: e.name,
                description: e.description,
                conditions: e.conditions,
                source: match e.source.as_str() {
                    "global"  => crate::model::agent_def::AgentSource::Global,
                    "builtin" => crate::model::agent_def::AgentSource::Builtin,
                    _         => crate::model::agent_def::AgentSource::Session,
                },
                model_uuid: e.model_uuid,
                model: e.model,
                tools: e.tools,
                prompt: e.prompt,
                file_path: None,
                ..crate::model::agent_def::AgentDef::default()
            }
        }).collect(),
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

/// Map a theme wire token back to a [`ThemeMode`] (unknown → Dark).
pub(crate) fn shadow_theme(t: &str) -> ThemeMode {
    match t {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// Map an internet-mode wire token back to an [`InternetMode`] (unknown → Simple).
pub(crate) fn shadow_internet_mode(t: &str) -> InternetMode {
    match t {
        "full" => InternetMode::Full,
        _ => InternetMode::Simple,
    }
}

/// Map an api-type wire token back to an [`ApiType`] (unknown → OpenAiCompatible).
pub(crate) fn shadow_api_type(t: &str) -> ApiType {
    match t {
        "anthropic" => ApiType::AnthropicCompatible,
        _ => ApiType::OpenAiCompatible,
    }
}

/// Map a role wire token back to a [`ModelRole`] (unknown → Main, never lost).
fn shadow_role(r: &str) -> ModelRole {
    match r {
        "awareness" => ModelRole::Awareness,
        "safeguard" => ModelRole::Safeguard,
        "compactor" => ModelRole::Compactor,
        _ => ModelRole::Main,
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
        // Install-health + the pane toggle + its cursor ride verbatim so the client
        // renders the same dependency pane the daemon would.
        install_health: s.install_health,
        health_view: s.health_view,
        health_selected: s.health_selected,
    }
}
