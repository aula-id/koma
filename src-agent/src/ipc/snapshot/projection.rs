//! Pure render-state PROJECTION for the daemon stage-4 streaming layer.
//!
//! [`build_snapshot`] reads the live [`AppState`] and copies out a frozen
//! [`StateSnapshot`]: one [`SessionSnapshot`] per session + the foreground id + the
//! [`GlobalSnapshot`]. It is the SINGLE source of truth for "what the client should
//! render", so a client can never diverge from the daemon — it only ever renders this
//! projection.
//!
//! Keeping this PURE (a function of `&AppState`, not a method that also drives the
//! socket) is deliberate: the daemon loop owns the channels + the monotonic seq and
//! merely calls these, and a future local-TUI consumer could call the exact same
//! builder, so the wire projection can never drift from a second hand-rolled copy.

use crate::app::mode::agents::{
    AgentEditField, AgentScope, AgentSubMode, AgentsState, ModelPickerState, ToolPickerState,
};
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::settings::{ModelDraft, ModelModal, PathPicker, PickerMode, ProviderDraft};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState, Mode,
    PickerState, RewindState, SessionHub, SessionKind, SettingsState, UsageMetric, UsageNavState,
    UsageView, WarmStatus,
};
use crate::app::resolve::resolve_role;
use crate::app::state::AppState;
use crate::app::subagent::SubAgentStatus;
use crate::model::app_config::{ApiType, AppConfig, ModelRole, ThemeMode};
use crate::model::store::SessionMeta;

use crate::ipc::proto::{
    AgentModelPickerSnapshot, AgentsSnapshot, CatalogueModelSnapshot, CatalogueProviderSnapshot,
    CookingEntrySnapshot, EffortSnapshot, GlobalSnapshot, HistoryEntrySnapshot, KeyInputSnapshot,
    LoadingSnapshot, ModeSnapshot, ModelDraftSnapshot, ModelEndpointWire, ModelModalSnapshot,
    PathPickerSnapshot, PendingSubagentSnapshot, PickerSnapshot, ProviderDraftSnapshot,
    ProviderModalSnapshot, RewindEntrySnapshot, RewindSnapshot, RolePickerSnapshot,
    SessionHubSnapshot, SessionMetaSnapshot, SessionSnapshot, SettingsSnapshot, StateSnapshot,
    SubAgentSnapshot, TextEditorSnapshot, ToolPickerSnapshot, UsageSnapshot, WarmStatusWire,
};

/// Build a complete, frozen [`StateSnapshot`] from the live [`AppState`].
pub fn build_snapshot(state: &AppState) -> StateSnapshot {
    let config = &state.rest.config;
    let sessions: Vec<SessionSnapshot> = state
        .rest
        .sessions
        .iter()
        .map(|rt| session_snapshot(rt, config))
        .collect();

    let foreground_id = state
        .rest
        .sessions
        .get(state.rest.foreground)
        .map(|s| s.id.clone());

    StateSnapshot {
        foreground_id,
        sessions,
        global: global_snapshot(state),
    }
}

fn session_snapshot(rt: &crate::app::state::SessionRuntime, config: &AppConfig) -> SessionSnapshot {
    let messages = rt
        .session
        .as_ref()
        .map(|s| s.conversation.messages().to_vec())
        .unwrap_or_default();
    let committed_reasoning: Vec<Option<String>> =
        if messages.iter().any(|m| m.reasoning.is_some()) {
            messages.iter().map(|m| m.reasoning.clone()).collect()
        } else {
            Vec::new()
        };
    let name = rt
        .session
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    let resolved_model_id = rt
        .session
        .as_ref()
        .and_then(|s| resolve_role(config, &s.settings, ModelRole::Main))
        .map(|r| r.model_id)
        .or_else(|| rt.session.as_ref().map(|s| s.settings.model.clone()))
        .unwrap_or_default();

    SessionSnapshot {
        id: rt.id.clone(),
        name,
        cwd: rt
            .session
            .as_ref()
            .map(|_| rt.effective_cwd().display().to_string())
            .unwrap_or_default(),
        messages,
        committed_reasoning,
        streaming: rt.streaming.clone(),
        stream_reasoning: rt.stream_reasoning.clone(),
        tokens_in: rt.tokens_in,
        tokens_out: rt.tokens_out,
        cost: rt.cost,
        tokens_cached: rt.tokens_cached,
        waiting: rt.waiting,
        awaiting_approval: rt.awaiting_approval,
        approval_reason: rt.approval_reason.clone(),
        pending_tool_calls: rt.pending_tool_calls.clone(),
        tool_idx: rt.tool_idx,
        working: rt.is_working(),
        finished_unseen: rt.finished_unseen,
        subagents: rt.subagents.iter().map(subagent_snapshot).collect(),
        pending_subagents: rt
            .pending_subagents
            .iter()
            .map(pending_subagent_snapshot)
            .collect(),
        resolved_model_id,
    }
}

fn subagent_snapshot(sa: &crate::app::subagent::SubAgent) -> SubAgentSnapshot {
    let status = match &sa.status {
        SubAgentStatus::Running => "running".to_string(),
        SubAgentStatus::Done(_) => "done".to_string(),
        SubAgentStatus::Killed => "killed".to_string(),
        SubAgentStatus::Error(e) => format!("error: {e}"),
    };

    SubAgentSnapshot {
        id: sa.id,
        name: sa.agent_name.clone(),
        label: sa.label.clone(),
        status,
        steps: sa.transcript.len(),
        transcript: sa.transcript.clone(),
        messages: sa.messages.clone(),
    }
}

fn pending_subagent_snapshot(
    p: &crate::app::subagent::PendingSubagent,
) -> PendingSubagentSnapshot {
    PendingSubagentSnapshot {
        id: p.id,
        agent_name: p.agent_name.clone(),
        prompt: p.prompt.clone(),
    }
}

fn global_snapshot(state: &AppState) -> GlobalSnapshot {
    GlobalSnapshot {
        input: state.rest.input.clone(),
        cursor: state.rest.cursor,
        scroll: state.rest.scroll,
        follow: state.rest.follow,
        status: state.rest.status.clone(),
        work_elapsed_ms: state
            .rest
            .work_since
            .map(|since| since.elapsed().as_millis() as u64),
        theme: theme_token(&state.rest.config.theme).to_string(),
        accent: state.rest.config.accent.clone(),
        mode: mode_snapshot(state),
        toast: state.rest.toast.as_ref().map(|(msg, _until, kind)| {
            let kind = match kind {
                crate::app::state::ToastKind::Error => "error".to_string(),
                crate::app::state::ToastKind::Info => "info".to_string(),
            };
            (kind, msg.clone())
        }),
        models_cache: state.rest.models_cache.clone(),
        models_cache_endpoint: state.rest.models_cache_endpoint.clone(),
        agent_viewer: state.rest.agent_viewer,
        agent_viewer_scroll: state.rest.agent_viewer_scroll,
        agent_viewer_follow: state.rest.agent_viewer_follow,
        subagents_open: state.rest.subagents_open,
        subagent_sel: state.rest.subagent_sel,
        palette_sel: state.rest.palette_sel,
        pending_attachments: state.rest.pending_attachments.clone(),
        file_palette: file_palette_matches(state),
        agent_mode: match state.rest.agent_mode {
            crate::app::state::AgentMode::Auto => "auto",
            crate::app::state::AgentMode::Normal => "normal",
        }
        .to_string(),
    }
}

const FILE_PAL_MAX: usize = 10;

fn file_palette_matches(state: &AppState) -> Option<Vec<String>> {
    let partial = crate::controller::input::file_ref_partial(&state.rest.input)?;
    let matches = state
        .rest
        .fg()
        .dir_cache
        .read()
        .map(|c| c.search(partial, FILE_PAL_MAX))
        .unwrap_or_default();
    Some(matches)
}

fn mode_snapshot(state: &AppState) -> ModeSnapshot {
    match &state.mode {
        Mode::KeyInput(f) => ModeSnapshot::KeyInput(key_input_snapshot(f)),
        Mode::SessionPicker(p) => ModeSnapshot::SessionPicker(picker_snapshot(p)),
        Mode::SessionHub(h) => ModeSnapshot::SessionHub(session_hub_snapshot(h)),
        Mode::Chat => ModeSnapshot::Chat,
        Mode::Loading(s) => ModeSnapshot::Loading(loading_snapshot(s)),
        Mode::Settings(s) => ModeSnapshot::Settings(Box::new(settings_snapshot(s))),
        Mode::Agents(a) => ModeSnapshot::Agents(Box::new(agents_snapshot(a, state))),
        Mode::Effort(e) => ModeSnapshot::Effort(effort_snapshot(e)),
        Mode::Usage(nav) => ModeSnapshot::Usage(Box::new(usage_snapshot(nav, state))),
        Mode::MessageRewind(rw) => ModeSnapshot::MessageRewind(rewind_snapshot(rw)),
        Mode::QuitConfirm(s) => ModeSnapshot::QuitConfirm {
            working: s.working,
            total: s.total,
        },
    }
}

fn key_input_snapshot(f: &KeyInputForm) -> KeyInputSnapshot {
    KeyInputSnapshot {
        step: f.step,
        field: f.field,
        endpoint: f.endpoint.clone(),
        api_key: f.api_key.clone(),
        model: f.model.clone(),
        query: f.query.clone(),
        result_sel: f.result_sel,
        first_run: f.first_run,
        from_picker: f.from_picker,
    }
}

fn loading_snapshot(s: &LoadingState) -> LoadingSnapshot {
    LoadingSnapshot {
        elapsed_ms: s.started.elapsed().as_millis() as u64,
        frame: s.frame,
        workspace: warm_status_wire(&s.workspace),
        awareness: warm_status_wire(&s.awareness),
    }
}

fn warm_status_wire(s: &WarmStatus) -> WarmStatusWire {
    match s {
        WarmStatus::Pending => WarmStatusWire::Pending,
        WarmStatus::Running => WarmStatusWire::Running,
        WarmStatus::Done(d) => WarmStatusWire::Done(d.clone()),
        WarmStatus::Skipped => WarmStatusWire::Skipped,
        WarmStatus::Failed => WarmStatusWire::Failed,
    }
}

fn session_hub_snapshot(h: &SessionHub) -> SessionHubSnapshot {
    SessionHubSnapshot {
        cooking: h.cooking.iter().map(cooking_entry_snapshot).collect(),
        history: h.history.iter().map(history_entry_snapshot).collect(),
        focus_cooking: matches!(h.focus, HubPane::Cooking),
        cooking_selected: h.cooking_selected,
        history_selected: h.history_selected,
    }
}

fn cooking_entry_snapshot(e: &CookingEntry) -> CookingEntrySnapshot {
    CookingEntrySnapshot {
        name: e.name.clone(),
        kind: match e.kind {
            SessionKind::Session => "session",
            SessionKind::NewSession => "new_session",
        }
        .to_string(),
        working: e.working,
        is_foreground: e.is_foreground,
    }
}

fn history_entry_snapshot(e: &HistoryEntry) -> HistoryEntrySnapshot {
    let last_active_secs = e
        .last_active
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    HistoryEntrySnapshot {
        name: e.name.clone(),
        last_active_secs,
    }
}

fn settings_snapshot(st: &SettingsState) -> SettingsSnapshot {
    SettingsSnapshot {
        cat: st.cat,
        field: st.field,
        in_detail: st.in_detail,
        editing: st.editing,
        api_key: st.api_key.clone(),
        model: st.model.clone(),
        provider: st.provider.clone(),
        name: st.name.clone(),
        theme: theme_token(&st.theme).to_string(),
        accent: st.accent.clone(),
        workdir: st.workdir.clone(),
        awareness_enabled: st.awareness_enabled,
        awareness_inherit: st.awareness_inherit,
        awareness_model: st.awareness_model.clone(),
        awareness_provider: st.awareness_provider.clone(),
        classifier_enabled: st.classifier_enabled,
        classifier_model: st.classifier_model.clone(),
        classifier_provider: st.classifier_provider.clone(),
        allowed_folders: st.allowed_folders.clone(),
        short_send_enabled: st.short_send_enabled,
        sliding_cache: st.sliding_cache,
        internet_mode: st.internet_mode.as_str().to_string(),
        cwd: st.cwd.display().to_string(),
        list_editing: st.list_editing,
        list_sel: st.list_sel,
        picker: st.picker.as_ref().map(path_picker_snapshot),
        providers: st.providers.iter().map(provider_draft_snapshot).collect(),
        prov_sel: st.prov_sel,
        prov_delete_armed: st.prov_delete_armed,
        prov_modal: st.prov_modal.as_ref().map(|m| ProviderModalSnapshot {
            name: m.name.clone(),
            endpoint: m.endpoint.clone(),
            api_type: api_type_token(m.api_type).to_string(),
            api_key: m.api_key.clone(),
            field: m.field,
        }),
        models: st.models.iter().map(model_draft_snapshot).collect(),
        model_sel: st.model_sel,
        model_delete_armed: st.model_delete_armed,
        model_modal: st.model_modal.as_ref().map(model_modal_snapshot),
    }
}

fn provider_draft_snapshot(p: &ProviderDraft) -> ProviderDraftSnapshot {
    ProviderDraftSnapshot {
        uuid: p.uuid.clone(),
        name: p.name.clone(),
        endpoint: p.endpoint.clone(),
        api_type: api_type_token(p.api_type).to_string(),
        api_key: p.api_key.clone(),
    }
}

fn model_draft_snapshot(m: &ModelDraft) -> ModelDraftSnapshot {
    ModelDraftSnapshot {
        uuid: m.uuid.clone(),
        name: m.name.clone(),
        model_id: m.model_id.clone(),
        provider_idx: m.provider_idx,
        roles: m.roles.iter().map(|r| role_token(*r).to_string()).collect(),
        route: m.route.clone(),
        session_only: m.session_only,
    }
}

fn model_modal_snapshot(m: &ModelModal) -> ModelModalSnapshot {
    ModelModalSnapshot {
        editing_idx: m.editing_idx,
        uuid: m.uuid.clone(),
        name: m.name.clone(),
        provider_idx: m.provider_idx,
        model_id: m.model_id.clone(),
        field: m.field,
        roles: m.roles.iter().map(|r| role_token(*r).to_string()).collect(),
        role_picker: m.role_picker.as_ref().map(|rp| RolePickerSnapshot {
            checked: rp.checked.clone(),
            cursor: rp.cursor,
        }),
        query: m.query.clone(),
        result_sel: m.result_sel,
        route: m.route.clone(),
        route_sel: m.route_sel,
        endpoints: m.endpoints.as_ref().map(|eps| {
            eps.iter()
                .map(|ep| ModelEndpointWire {
                    name: ep.name.clone(),
                    provider_name: ep.provider_name.clone(),
                    price_prompt: ep.pricing.as_ref().and_then(|p| p.prompt.clone()),
                    price_completion: ep.pricing.as_ref().and_then(|p| p.completion.clone()),
                    uptime_last_30m: ep.uptime_last_30m,
                })
                .collect()
        }),
        endpoints_loading: m.endpoints_loading,
        endpoints_for: m.endpoints_for.clone(),
    }
}

fn path_picker_snapshot(p: &PathPicker) -> PathPickerSnapshot {
    PathPickerSnapshot {
        query: p.query.clone(),
        matches: p.matches.clone(),
        sel: p.sel,
        replace_idx: match p.mode {
            PickerMode::Add => None,
            PickerMode::Replace(i) => Some(i),
        },
    }
}

fn usage_snapshot(nav: &UsageNavState, state: &AppState) -> UsageSnapshot {
    UsageSnapshot {
        view: usage_view_token(nav.view).to_string(),
        range: nav.range.label().to_string(),
        metric: usage_metric_token(nav.metric).to_string(),
        data: crate::view::usage::collect_usage_data(nav, &state.rest),
    }
}

fn usage_view_token(v: UsageView) -> &'static str {
    match v {
        UsageView::Global => "global",
        UsageView::Session => "session",
    }
}

fn usage_metric_token(m: UsageMetric) -> &'static str {
    match m {
        UsageMetric::Cost => "cost",
        UsageMetric::Tokens => "tokens",
    }
}

fn rewind_snapshot(rw: &RewindState) -> RewindSnapshot {
    RewindSnapshot {
        entries: rw
            .entries
            .iter()
            .map(|e| RewindEntrySnapshot {
                vec_index: e.vec_index,
                content: e.content.clone(),
            })
            .collect(),
        selected: rw.selected,
    }
}

fn effort_snapshot(e: &EffortPickerState) -> EffortSnapshot {
    EffortSnapshot {
        options: e.options.clone(),
        selected: e.selected,
        note: e.note.clone(),
    }
}

fn picker_snapshot(p: &PickerState) -> PickerSnapshot {
    PickerSnapshot {
        query: p.query.clone(),
        all: p.all.iter().map(session_meta_snapshot).collect(),
        filtered_idx: p.filtered_idx.clone(),
        selected: p.selected,
    }
}

fn session_meta_snapshot(m: &SessionMeta) -> SessionMetaSnapshot {
    let modified_secs = m
        .modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    SessionMetaSnapshot {
        id: m.id.clone(),
        name: m.name.clone(),
        modified_secs,
        message_count: m.message_count,
        locked: m.locked,
    }
}

fn agents_snapshot(a: &AgentsState, state: &AppState) -> AgentsSnapshot {
    let config = &state.rest.config;

    let mut catalogue_models: Vec<CatalogueModelSnapshot> = Vec::new();
    if let Some(session) = state.rest.fg().session.as_ref() {
        for e in &session.settings.session_models {
            catalogue_models.push(CatalogueModelSnapshot {
                uuid: e.uuid.clone(),
                name: e.name.clone(),
                model_id: e.model_id.clone(),
                provider_uuid: e.provider_uuid.clone(),
            });
        }
    }
    for e in &config.models {
        catalogue_models.push(CatalogueModelSnapshot {
            uuid: e.uuid.clone(),
            name: e.name.clone(),
            model_id: e.model_id.clone(),
            provider_uuid: e.provider_uuid.clone(),
        });
    }

    let catalogue_providers: Vec<CatalogueProviderSnapshot> = config
        .providers
        .iter()
        .map(|p| CatalogueProviderSnapshot {
            uuid: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
        })
        .collect();

    AgentsSnapshot {
        agents: a
            .agents
            .iter()
            .map(|ag| crate::ipc::proto::AgentEntry {
                name: ag.name.clone(),
                description: ag.description.clone(),
                conditions: ag.conditions.clone(),
                source: match ag.source {
                    crate::model::agent_def::AgentSource::Session => "session",
                    crate::model::agent_def::AgentSource::Global => "global",
                    crate::model::agent_def::AgentSource::Builtin => "builtin",
                }
                .to_string(),
                model_uuid: ag.model_uuid.clone(),
                model: ag.model.clone(),
                tools: ag.tools.clone(),
                prompt: ag.prompt.clone(),
            })
            .collect(),
        list_sel: a.list_sel,
        in_detail: a.in_detail,
        mode: agent_submode_token(a.mode).to_string(),
        field: agent_field_token(a.field).to_string(),
        editing: a.editing,
        create_scope: agent_scope_token(a.create_scope).to_string(),
        draft_name: a.draft_name.clone(),
        draft_description: a.draft_description.clone(),
        draft_conditions: a.draft_conditions.clone(),
        draft_model_uuid: a.draft_model_uuid.clone(),
        draft_model_legacy: a.draft_model_legacy.clone(),
        draft_tools: a.draft_tools.clone(),
        draft_body: a.draft_body.clone(),
        tool_picker: a.tool_picker.as_ref().map(tool_picker_snapshot),
        model_picker: a.model_picker.as_ref().map(agent_model_picker_snapshot),
        editor: a.editor.as_ref().map(|(field, ed)| {
            (
                agent_field_token(*field).to_string(),
                text_editor_snapshot(ed),
            )
        }),
        editor_clear_confirm: a.editor_clear_confirm,
        catalogue_models,
        catalogue_providers,
    }
}

fn tool_picker_snapshot(p: &ToolPickerState) -> ToolPickerSnapshot {
    ToolPickerSnapshot {
        options: p.options.clone(),
        checked: p.checked.clone(),
        cursor: p.cursor,
        filter: p.filter.clone(),
    }
}

fn agent_model_picker_snapshot(p: &ModelPickerState) -> AgentModelPickerSnapshot {
    AgentModelPickerSnapshot {
        options: p.options.clone(),
        cursor: p.cursor,
    }
}

fn text_editor_snapshot(ed: &TextEditorState) -> TextEditorSnapshot {
    TextEditorSnapshot {
        lines: ed.lines.clone(),
        row: ed.row,
        col: ed.col,
        scroll: ed.scroll,
    }
}

fn agent_submode_token(m: AgentSubMode) -> &'static str {
    match m {
        AgentSubMode::Browse => "browse",
        AgentSubMode::Edit => "edit",
        AgentSubMode::Create => "create",
        AgentSubMode::DeleteConfirm => "delete_confirm",
    }
}

fn agent_field_token(f: AgentEditField) -> &'static str {
    match f {
        AgentEditField::Name => "name",
        AgentEditField::Description => "description",
        AgentEditField::Conditions => "conditions",
        AgentEditField::Model => "model",
        AgentEditField::Tools => "tools",
        AgentEditField::Body => "prompt",
    }
}

fn agent_scope_token(s: AgentScope) -> &'static str {
    match s {
        AgentScope::Session => "session",
        AgentScope::Global => "global",
    }
}

fn theme_token(t: &ThemeMode) -> &'static str {
    match t {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    }
}

fn api_type_token(t: ApiType) -> &'static str {
    match t {
        ApiType::OpenAiCompatible => "openai",
        ApiType::AnthropicCompatible => "anthropic",
    }
}

fn role_token(r: ModelRole) -> &'static str {
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
    }
}
