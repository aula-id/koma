use super::tokens::{
    agent_field_token, agent_scope_token, agent_submode_token, api_type_token, role_token,
    theme_token, usage_metric_token, usage_view_token,
};

use crate::app::mode::agents::{AgentsState, ModelPickerState, ToolPickerState};
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::settings::{ModelDraft, ModelModal, PathPicker, PickerMode, ProviderDraft};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState, Mode,
    PickerState, RewindState, SessionHub, SessionKind, SettingsState, UsageNavState,
    WarmStatus,
};
use crate::app::state::AppState;
use crate::model::store::SessionMeta;

use crate::ipc::proto::{
    AgentModelPickerSnapshot, AgentsSnapshot, CatalogueModelSnapshot, CatalogueProviderSnapshot,
    CookingEntrySnapshot, EffortSnapshot, HistoryEntrySnapshot, KeyInputSnapshot, LoadingSnapshot,
    ModeSnapshot, ModelDraftSnapshot, ModelEndpointWire, ModelModalSnapshot, PathPickerSnapshot,
    PickerSnapshot, ProviderDraftSnapshot, ProviderModalSnapshot, RewindEntrySnapshot,
    RewindSnapshot, RolePickerSnapshot, SessionHubSnapshot, SessionMetaSnapshot, SettingsSnapshot,
    TextEditorSnapshot, ToolPickerSnapshot, UsageSnapshot, WarmStatusWire,
};

pub fn mode_snapshot(state: &AppState) -> ModeSnapshot {
    match &state.mode {
        Mode::KeyInput(f) => ModeSnapshot::KeyInput(key_input_snapshot(f)),
        Mode::SessionPicker(p) => ModeSnapshot::SessionPicker(picker_snapshot(p)),
        Mode::SessionHub(h) => ModeSnapshot::SessionHub(session_hub_snapshot(h)),
        Mode::Chat => ModeSnapshot::Chat,
        Mode::Loading(s) => ModeSnapshot::Loading(loading_snapshot(s)),
        Mode::Settings(s) => ModeSnapshot::Settings(Box::new(settings_snapshot(s))),
        Mode::Agents(a) => ModeSnapshot::Agents(Box::new(agents_snapshot(a, state))),
        // The `/mcp` dashboard is a LOCAL-TUI-only mode for now: it has no daemon
        // wire snapshot (no `ModeSnapshot::Mcp` variant), so a thin client attached
        // to a host that's in `/mcp` simply sees Chat. Additive + crash-free; wiring
        // a real remote projection is out of scope for this pass.
        Mode::Mcp(_) => ModeSnapshot::Chat,
        Mode::Effort(e) => ModeSnapshot::Effort(effort_snapshot(e)),
        Mode::Usage(nav) => ModeSnapshot::Usage(Box::new(usage_snapshot(nav, state))),
        Mode::MessageRewind(rw) => ModeSnapshot::MessageRewind(rewind_snapshot(rw)),
        Mode::QuitConfirm(s) => ModeSnapshot::QuitConfirm {
            working: s.working,
            total: s.total,
        },
    }
}

pub fn key_input_snapshot(f: &KeyInputForm) -> KeyInputSnapshot {
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

pub fn loading_snapshot(s: &LoadingState) -> LoadingSnapshot {
    LoadingSnapshot {
        elapsed_ms: s.started.elapsed().as_millis() as u64,
        frame: s.frame,
        workspace: warm_status_wire(&s.workspace),
        awareness: warm_status_wire(&s.awareness),
    }
}

pub fn warm_status_wire(s: &WarmStatus) -> WarmStatusWire {
    match s {
        WarmStatus::Pending => WarmStatusWire::Pending,
        WarmStatus::Running => WarmStatusWire::Running,
        WarmStatus::Done(d) => WarmStatusWire::Done(d.clone()),
        WarmStatus::Skipped => WarmStatusWire::Skipped,
        WarmStatus::Failed => WarmStatusWire::Failed,
    }
}

pub fn session_hub_snapshot(h: &SessionHub) -> SessionHubSnapshot {
    SessionHubSnapshot {
        cooking: h.cooking.iter().map(cooking_entry_snapshot).collect(),
        history: h.history.iter().map(history_entry_snapshot).collect(),
        focus_cooking: matches!(h.focus, HubPane::Cooking),
        cooking_selected: h.cooking_selected,
        history_selected: h.history_selected,
    }
}

pub fn cooking_entry_snapshot(e: &CookingEntry) -> CookingEntrySnapshot {
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

pub fn history_entry_snapshot(e: &HistoryEntry) -> HistoryEntrySnapshot {
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

pub fn settings_snapshot(st: &SettingsState) -> SettingsSnapshot {
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

pub fn provider_draft_snapshot(p: &ProviderDraft) -> ProviderDraftSnapshot {
    ProviderDraftSnapshot {
        uuid: p.uuid.clone(),
        name: p.name.clone(),
        endpoint: p.endpoint.clone(),
        api_type: api_type_token(p.api_type).to_string(),
        api_key: p.api_key.clone(),
    }
}

pub fn model_draft_snapshot(m: &ModelDraft) -> ModelDraftSnapshot {
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

pub fn model_modal_snapshot(m: &ModelModal) -> ModelModalSnapshot {
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

pub fn path_picker_snapshot(p: &PathPicker) -> PathPickerSnapshot {
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

pub fn usage_snapshot(nav: &UsageNavState, state: &AppState) -> UsageSnapshot {
    UsageSnapshot {
        view: usage_view_token(nav.view).to_string(),
        range: nav.range.label().to_string(),
        metric: usage_metric_token(nav.metric).to_string(),
        data: crate::view::usage::collect_usage_data(nav, &state.rest),
    }
}

pub fn rewind_snapshot(rw: &RewindState) -> RewindSnapshot {
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

pub fn effort_snapshot(e: &EffortPickerState) -> EffortSnapshot {
    EffortSnapshot {
        options: e.options.clone(),
        selected: e.selected,
        note: e.note.clone(),
    }
}

pub fn picker_snapshot(p: &PickerState) -> PickerSnapshot {
    PickerSnapshot {
        query: p.query.clone(),
        all: p.all.iter().map(session_meta_snapshot).collect(),
        filtered_idx: p.filtered_idx.clone(),
        selected: p.selected,
    }
}

pub fn session_meta_snapshot(m: &SessionMeta) -> SessionMetaSnapshot {
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

pub fn agents_snapshot(a: &AgentsState, state: &AppState) -> AgentsSnapshot {
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

pub fn tool_picker_snapshot(p: &ToolPickerState) -> ToolPickerSnapshot {
    ToolPickerSnapshot {
        options: p.options.clone(),
        checked: p.checked.clone(),
        cursor: p.cursor,
        filter: p.filter.clone(),
    }
}

pub fn agent_model_picker_snapshot(p: &ModelPickerState) -> AgentModelPickerSnapshot {
    AgentModelPickerSnapshot {
        options: p.options.clone(),
        cursor: p.cursor,
    }
}

pub fn text_editor_snapshot(ed: &TextEditorState) -> TextEditorSnapshot {
    TextEditorSnapshot {
        lines: ed.lines.clone(),
        row: ed.row,
        col: ed.col,
        scroll: ed.scroll,
    }
}
