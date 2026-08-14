use super::tokens::{
    agent_field_token, agent_scope_token, agent_submode_token, api_type_token, ext_submode_token,
    help_kind_token, mcp_field_token, mcp_submode_token, mcp_transport_token, role_token,
    settings_page_token, store_submode_token, theme_token, usage_metric_token, usage_view_token,
};

use crate::app::mode::agents::{AgentsState, ModelPickerState, ToolPickerState};
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::ext_screen::ExtScreenState;
use crate::app::mode::extensions::{ExtRow, ExtensionsState};
use crate::app::mode::help::HelpState;
use crate::app::mode::mcp::McpState;
use crate::app::mode::security::SecurityState;
use crate::app::mode::settings::{
    ModelDraft, ModelModal, OAuthDraft, PathPicker, PickerMode, ProviderDraft,
};
use crate::app::mode::skill_cmd::{SkillCmdState, SkillFilterChip};
use crate::app::mode::store::{ExtStoreState, StoreRow};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState, Mode,
    OnboardProviderState, OnboardProviderStep, OnboardState, PickerState, RewindState, SessionHub,
    SessionKind, SettingsState, UsageNavState, WarmStatus,
};
use crate::app::state::{AppState, AppStateRest};
use crate::model::store::SessionMeta;

use crate::ipc::proto::{
    AgentModelPickerSnapshot, AgentsSnapshot, BashJobView, BashSnapshot, CatalogueModelSnapshot,
    CatalogueProviderSnapshot, CookingEntrySnapshot, EffortSnapshot, ExtRowWire, ExtScreenSnapshot,
    ExtStoreDetailWire, ExtStoreRowWire, ExtStoreSnapshot, ExtTuiScreenWire, ExtensionsSnapshot,
    HelpEntrySnapshot, HelpSnapshot, HistoryEntrySnapshot, KeyInputSnapshot, LoadingSnapshot,
    McpSnapshot, ModeSnapshot, ModelCmdSnapshot, ModelDraftSnapshot, ModelEndpointWire,
    ModelModalSnapshot, OAuthDraftSnapshot, OnboardProviderSnapshot, OnboardSnapshot,
    PathPickerSnapshot, PickerSnapshot, ProviderDraftSnapshot, ProviderModalSnapshot,
    RemoteHostSnapshot, RemoteSessionSnapshot, RemoteSnapshot, RewindEntrySnapshot, RewindSnapshot,
    RolePickerSnapshot, SecuritySnapshot, SessionHubSnapshot, SessionMetaSnapshot,
    SettingsSnapshot, SkillCmdSnapshot, SkillEntrySnapshot, TextEditorSnapshot, TodoItemSnapshot,
    TodoSnapshot, ToolPickerSnapshot, UsageSnapshot, WarmStatusWire,
};

pub fn mode_snapshot(state: &AppState) -> ModeSnapshot {
    // Source the wire mode from the FOREGROUND session's mode (C3). The wire
    // `GlobalSnapshot.mode` stays as-is; it is just sourced per-session now. In the
    // daemon the per-client foreground is swapped in before each per-client projection
    // (C2), so this projects THAT client's overlay.
    match state.mode() {
        Mode::Onboard(o) => ModeSnapshot::Onboard(Box::new(onboard_snapshot(o))),
        Mode::OnboardProvider(op) => {
            ModeSnapshot::OnboardProvider(Box::new(onboard_provider_snapshot(op)))
        }
        Mode::KeyInput(f) => ModeSnapshot::KeyInput(key_input_snapshot(f)),
        Mode::SessionPicker(p) => ModeSnapshot::SessionPicker(picker_snapshot(p)),
        Mode::SessionHub(h) => ModeSnapshot::SessionHub(session_hub_snapshot(h)),
        Mode::Chat => ModeSnapshot::Chat,
        Mode::Loading(s) => ModeSnapshot::Loading(loading_snapshot(s)),
        Mode::Settings(s) => ModeSnapshot::Settings(Box::new(settings_snapshot(s))),
        Mode::Agents(a) => ModeSnapshot::Agents(Box::new(agents_snapshot(a, state))),
        // The `/mcp` dashboard projects a full wire snapshot, exactly like `/agents`:
        // the server list + drafts + sub-mode/field/transport tokens, plus the LIVE
        // per-server tool counts (the client owns no MCP manager), so a thin client
        // rebuilds and renders the dashboard faithfully instead of a blank Chat screen.
        Mode::Mcp(m) => ModeSnapshot::Mcp(Box::new(mcp_snapshot(m, state))),
        // The `/extension` manager projects the installed-extension rows (registry + manifest
        // facts + the LIVE running flag, baked at build time) + cursors + sub-mode token, so a
        // thin client rebuilds + renders the dashboard instead of a blank Chat screen.
        Mode::Extensions(e) => ModeSnapshot::Extensions(Box::new(extensions_snapshot(e))),
        // An open extension screen projects the opaque `Screen` model verbatim + the menu
        // cursor + loading/error flags, so a thin client renders the SAME server-driven view
        // (the daemon owns the invoke + folds every reply/push).
        Mode::ExtScreen(s) => ModeSnapshot::ExtScreen(Box::new(ext_screen_snapshot(s))),
        // The `/store` marketplace browser projects the fetched catalogue rows + cursor +
        // sub-mode token + the Browse/Detail/InstallConfirm loading/error/data fields, so
        // a thin client rebuilds + renders the SAME browse/detail/confirm view instead of
        // a blank Chat screen.
        Mode::ExtStore(s) => ModeSnapshot::ExtStore(Box::new(ext_store_snapshot(s))),
        // The `/security` control panel projects a live status re-read from the
        // daemon manager (so the snapshot always reflects current daemon state after
        // start/stop, not just the state at mode-open time) plus the cursor.
        Mode::Security(s) => ModeSnapshot::Security(Box::new(security_snapshot(s, state))),
        // The `/bash` panel projects the LIVE background-job registry (read fresh from
        // the foreground session every frame, like `/agents`) + the list cursor, so a
        // thin client renders the same master/detail view of current jobs.
        Mode::Bash(b) => ModeSnapshot::Bash(Box::new(bash_snapshot(b, &state.rest))),
        Mode::Todo(t) => ModeSnapshot::Todo(Box::new(todo_snapshot(t))),
        // The `/help` reference projects a full wire snapshot, exactly like `/mcp`:
        // the query + entry list (each entry's kind as a wire token) + filtered subset
        // + cursor, so a thin client rebuilds and renders the searchable help screen
        // instead of a blank Chat screen.
        Mode::Help(h) => ModeSnapshot::Help(Box::new(help_snapshot(h))),
        Mode::Skill(s) => ModeSnapshot::Skill(Box::new(skill_cmd_snapshot(s))),
        Mode::Remote(m) => ModeSnapshot::Remote(Box::new(remote_snapshot(m))),
        Mode::Effort(e) => ModeSnapshot::Effort(effort_snapshot(e)),
        Mode::Model(m) => ModeSnapshot::Model(Box::new(model_cmd_snapshot(m))),
        Mode::Usage(nav) => ModeSnapshot::Usage(Box::new(usage_snapshot(nav, state))),
        Mode::MessageRewind(rw) => ModeSnapshot::MessageRewind(rewind_snapshot(rw)),
        Mode::QuitConfirm(s) => ModeSnapshot::QuitConfirm {
            working: s.working,
            total: s.total,
            selected: s.selected,
        },
    }
}

pub fn onboard_snapshot(o: &OnboardState) -> OnboardSnapshot {
    OnboardSnapshot { cursor: o.cursor }
}

pub fn onboard_provider_snapshot(op: &OnboardProviderState) -> OnboardProviderSnapshot {
    OnboardProviderSnapshot {
        step: match op.step {
            OnboardProviderStep::Login => "login",
            OnboardProviderStep::ModelSelect => "model_select",
        }
        .to_string(),
        oauth_flow: oauth_flow_snapshot(&op.oauth_flow),
        new_conn_uuid: op.new_conn_uuid.clone(),
        provider: op.provider.map(|p| {
            match p {
                crate::model::app_config::OAuthProvider::Codex => "codex",
                crate::model::app_config::OAuthProvider::Kilocode => "kilocode",
                crate::model::app_config::OAuthProvider::Xai => "xai",
                crate::model::app_config::OAuthProvider::ClaudeAI => "claudeai",
                crate::model::app_config::OAuthProvider::KomaRun => "komarun",
                crate::model::app_config::OAuthProvider::CommandCode => "commandcode",
                // W11: ext-backed marker (a native TUI onboard flow never selects it).
                crate::model::app_config::OAuthProvider::Extension => "extension",
            }
            .to_string()
        }),
        query: op.query.clone(),
        result_sel: op.result_sel,
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
        // Project the FILTERED history view (in filtered order) — the client renders
        // it verbatim and `history_selected` already indexes into this filtered list.
        history: h
            .history_filtered
            .iter()
            .filter_map(|&i| h.history.get(i))
            .map(history_entry_snapshot)
            .collect(),
        focus_cooking: matches!(h.focus, HubPane::Cooking),
        cooking_selected: h.cooking_selected,
        history_selected: h.history_selected,
        history_query: h.history_query.clone(),
        pending_kill: h.pending_kill.clone(),
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
        session_id: e.session_id.clone(),
        remote_host: e.remote_host.clone(),
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
        page: settings_page_token(st.page).to_string(),
        field: st.field,
        editing: st.editing,
        api_key: st.api_key.clone(),
        model: st.model.clone(),
        provider: st.provider.clone(),
        name: st.name.clone(),
        theme: theme_token(&st.theme).to_string(),
        accent: st.accent.clone(),
        palette: st.palette.clone(),
        workdir: st.workdir.clone(),
        awareness_enabled: st.awareness_enabled,
        classifier_enabled: st.classifier_enabled,
        allowed_folders: st.allowed_folders.clone(),
        short_send_enabled: st.short_send_enabled,
        sliding_cache: st.sliding_cache,
        bash_saving: st.bash_saving,
        coding_autosave: st.coding_autosave,
        internet_mode: st.internet_mode.as_str().to_string(),
        mouse_capture: st.mouse_capture.as_str().to_string(),
        cwd: st.cwd.display().to_string(),
        list_editing: st.list_editing,
        list_sel: st.list_sel,
        picker: st.picker.as_ref().map(path_picker_snapshot),
        providers: st.providers.iter().map(provider_draft_snapshot).collect(),
        oauth_drafts: st.oauth_drafts.iter().map(oauth_draft_snapshot).collect(),
        oauth_sel: st.oauth_sel,
        oauth_armed: st.oauth_armed,
        oauth_flow: oauth_flow_snapshot(&st.oauth_flow),
        prov_sel: st.prov_sel,
        prov_delete_armed: st.prov_delete_armed,
        prov_msg: st.prov_msg.clone(),
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
        model_filter: match st.model_filter {
            crate::app::mode::settings::ModelFilterMode::All => "all",
            crate::app::mode::settings::ModelFilterMode::Local => "local",
            crate::app::mode::settings::ModelFilterMode::Global => "global",
        }
        .to_string(),
        palette_sel: st.palette_sel,
        menu_sel: st.menu_sel,
        subagent_max_turns: st.subagent_max_turns.parse().unwrap_or(500),
    }
}

pub fn provider_draft_snapshot(p: &ProviderDraft) -> ProviderDraftSnapshot {
    ProviderDraftSnapshot {
        uuid: p.uuid.clone(),
        name: p.name.clone(),
        endpoint: p.endpoint.clone(),
        api_type: api_type_token(p.api_type).to_string(),
        api_key: p.api_key.clone(),
        ext_id: p.ext_id.clone(),
    }
}

pub fn oauth_draft_snapshot(o: &OAuthDraft) -> OAuthDraftSnapshot {
    OAuthDraftSnapshot {
        uuid: o.uuid.clone(),
        label: o.label.clone(),
        provider: match o.provider {
            crate::model::app_config::OAuthProvider::Codex => "codex",
            crate::model::app_config::OAuthProvider::Kilocode => "kilocode",
            crate::model::app_config::OAuthProvider::Xai => "xai",
            crate::model::app_config::OAuthProvider::ClaudeAI => "claudeai",
            crate::model::app_config::OAuthProvider::KomaRun => "komarun",
            crate::model::app_config::OAuthProvider::CommandCode => "commandcode",
            // W11: ext-backed marker.
            crate::model::app_config::OAuthProvider::Extension => "extension",
        }
        .to_string(),
        key: o.key.clone(),
        status: o.status.clone(),
    }
}

/// Project the OAuth submenu's connect-flow state into its flat wire form (see
/// [`crate::ipc::proto::OAuthFlowSnapshot`] for the field-reuse convention).
pub fn oauth_flow_snapshot(
    f: &crate::app::mode::settings::OAuthFlowState,
) -> crate::ipc::proto::OAuthFlowSnapshot {
    use crate::app::mode::settings::OAuthFlowState;
    use crate::ipc::proto::OAuthFlowSnapshot;
    match f {
        OAuthFlowState::Idle => OAuthFlowSnapshot {
            kind: "idle".to_string(),
            ..Default::default()
        },
        OAuthFlowState::Starting { provider } => OAuthFlowSnapshot {
            kind: "starting".to_string(),
            provider: provider.wire_id().to_string(),
            ..Default::default()
        },
        OAuthFlowState::Pick(cursor) => OAuthFlowSnapshot {
            kind: "pick".to_string(),
            cursor: *cursor,
            ..Default::default()
        },
        OAuthFlowState::CodexWait {
            provider,
            url,
            frame,
            copied,
        } => OAuthFlowSnapshot {
            kind: "codex_wait".to_string(),
            provider: provider.wire_id().to_string(),
            url: url.clone(),
            frame: *frame,
            copied: *copied,
            ..Default::default()
        },
        OAuthFlowState::CodexPaste {
            input, provider, ..
        } => OAuthFlowSnapshot {
            kind: "codex_paste".to_string(),
            input: input.clone(),
            provider: provider.wire_id().to_string(),
            ..Default::default()
        },
        OAuthFlowState::KiloWait {
            provider,
            user_code,
            verification_url,
            frame,
            copied,
        } => OAuthFlowSnapshot {
            kind: "kilo_wait".to_string(),
            provider: provider.wire_id().to_string(),
            url: verification_url.clone(),
            user_code: user_code.clone(),
            frame: *frame,
            copied: *copied,
            ..Default::default()
        },
        OAuthFlowState::Failed(error) => OAuthFlowSnapshot {
            kind: "failed".to_string(),
            error: error.clone(),
            ..Default::default()
        },
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
        session_only: m.session_only,
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

pub fn model_cmd_snapshot(m: &crate::app::mode::ModelCmdState) -> ModelCmdSnapshot {
    use crate::app::mode::ModelCmdSub;
    let (sub, role, agent_name, lines) = match &m.sub {
        ModelCmdSub::Help { lines } => ("help".to_string(), None, None, lines.clone()),
        ModelCmdSub::RolePick { role } => {
            let r = match role {
                crate::model::app_config::ModelRole::Main => "main",
                crate::model::app_config::ModelRole::Awareness => "awareness",
                crate::model::app_config::ModelRole::Planner => "planner",
                crate::model::app_config::ModelRole::Compactor => "compactor",
                crate::model::app_config::ModelRole::Safeguard => "safeguard",
            };
            ("role_pick".to_string(), Some(r.to_string()), None, vec![])
        }
        ModelCmdSub::AgentList => ("agent_list".to_string(), None, None, vec![]),
        ModelCmdSub::AgentPick {
            agent_name,
            current_model: _,
        } => (
            "agent_pick".to_string(),
            None,
            Some(agent_name.clone()),
            vec![],
        ),
    };
    ModelCmdSnapshot {
        sub,
        role,
        agent_name,
        options: m.options.clone(),
        cursor: m.cursor,
        note: m.note.clone(),
        lines,
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

    // Include OAuth conns alongside static providers so OAuth-backed models
    // (xAI / Codex / koma.run / …) resolve to `name @ provider` in /agents,
    // not `(unknown model)`. Projected KEYLESS (uuid + display name only).
    // Mirrors send_agents_values (requests_read.rs) and build_host_agents_values
    // (host_catalogue.rs).
    let mut catalogue_providers: Vec<CatalogueProviderSnapshot> = config
        .providers
        .iter()
        .map(|p| CatalogueProviderSnapshot {
            uuid: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
        })
        .collect();
    for c in &config.oauth_conns {
        let name = if !c.name.trim().is_empty() {
            c.name.clone()
        } else {
            c.provider.label().to_string()
        };
        catalogue_providers.push(CatalogueProviderSnapshot {
            uuid: c.uuid.clone(),
            name,
            endpoint: String::new(),
        });
    }

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
                    crate::model::agent_def::AgentSource::Extension => "extension",
                }
                .to_string(),
                model_uuid: ag.model_uuid.clone(),
                model: ag.model.clone(),
                tools: ag.tools.clone(),
                prompt: ag.prompt.clone(),
                ext_id: ag.ext_id.clone(),
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

/// Project the FOREGROUND session's background bash jobs into wire-safe views.
///
/// Reads `rest.fg().bash_jobs` LIVE and maps each [`crate::app::bgbash::BashJob`]
/// into a [`BashJobView`]: the lifecycle status rendered to a label
/// (`"running"` / `"exit {n}"` / `"killed"` / `"error: {…}"`), a `running` flag,
/// the wall-clock elapsed seconds, and a trimmed `output_tail` (the last ~40 lines,
/// then capped to ~4000 chars so a chatty job's snapshot stays bounded). Built fresh
/// every frame + on every key, exactly like the agents list, so the panel always
/// reflects current jobs.
///
/// Takes `&AppStateRest` (not `&AppState`) so the `/bash` command + the input handler
/// — which only hold `rest` — can call it directly to seed/refresh the panel.
pub fn bash_job_views(rest: &AppStateRest) -> Vec<BashJobView> {
    use crate::app::bgbash::BashJobStatus;

    rest.fg()
        .bash_jobs
        .iter()
        .map(|job| {
            let status = match job.snapshot_status() {
                BashJobStatus::Running => "running".to_string(),
                BashJobStatus::Done(code) => format!("exit {code}"),
                BashJobStatus::Killed => "killed".to_string(),
                BashJobStatus::Error(msg) => format!("error: {msg}"),
            };
            let running = job.is_running();
            let elapsed_secs = job.elapsed_secs();
            BashJobView {
                id: job.id,
                command: job.command.clone(),
                status,
                running,
                elapsed_secs,
                output_tail: tail_output(&job.output_snapshot()),
            }
        })
        .collect()
}

/// Trim a job's captured output to a bounded tail for the panel: keep the LAST ~40
/// lines, then cap the result to the last ~4000 chars (char-based, so multi-byte
/// UTF-8 is never sliced mid-codepoint). The detail pane renders this verbatim.
fn tail_output(full: &str) -> String {
    const MAX_LINES: usize = 40;
    const MAX_CHARS: usize = 4000;

    // Last ~MAX_LINES lines (preserving their order).
    let lines: Vec<&str> = full.lines().collect();
    let start = lines.len().saturating_sub(MAX_LINES);
    let mut tail = lines[start..].join("\n");

    // Then cap to the last MAX_CHARS chars so a single huge line can't blow the budget.
    let len = tail.chars().count();
    if len > MAX_CHARS {
        tail = tail.chars().skip(len - MAX_CHARS).collect();
    }
    tail
}

/// Project the `/bash` panel: the LIVE job views + the list cursor.
pub fn bash_snapshot(b: &crate::app::mode::BashState, rest: &AppStateRest) -> BashSnapshot {
    BashSnapshot {
        jobs: bash_job_views(rest),
        selected: b.selected,
    }
}

/// Project the `/todo` panel: the todo items + the list cursor.
pub fn todo_snapshot(t: &crate::app::mode::TodoState) -> TodoSnapshot {
    TodoSnapshot {
        items: t
            .items
            .iter()
            .map(|item| TodoItemSnapshot {
                content: item.content.clone(),
                status: item.status.label().to_string(),
                priority: item.priority.label().to_string(),
                locked: item.locked,
            })
            .collect(),
        selected: t.selected,
        pwd_hash: t.pwd_hash.clone(),
    }
}

pub fn mcp_snapshot(m: &McpState, state: &AppState) -> McpSnapshot {
    McpSnapshot {
        // `McpServerEntry` is serde-able pure data (no secrets), so the server list
        // rides verbatim — the lightest projection that round-trips.
        servers: m.servers.clone(),
        list_sel: m.list_sel,
        in_detail: m.in_detail,
        mode: mcp_submode_token(m.mode).to_string(),
        field: mcp_field_token(m.field).to_string(),
        editing: m.editing,
        draft_uuid: m.draft_uuid.clone(),
        draft_name: m.draft_name.clone(),
        draft_enabled: m.draft_enabled,
        draft_transport: mcp_transport_token(m.draft_transport).to_string(),
        draft_command: m.draft_command.clone(),
        draft_args: m.draft_args.clone(),
        draft_env: m.draft_env.clone(),
        draft_url: m.draft_url.clone(),
        // LIVE per-server tool counts from the daemon's MCP manager (uuid -> count).
        // The client owns no manager, so this projected map is its only status source
        // (it lands in `McpState::shadow_status` and the view falls back to it).
        status: state
            .rest
            .mcp_manager
            .as_ref()
            .map(|mgr| mgr.server_status_cached())
            .unwrap_or_default(),
    }
}

/// Project ONE installed-extension row into its wire mirror (pure data — registry facts +
/// manifest-derived counts/screens + the baked running flag, all cloned verbatim).
fn ext_row_wire(r: &ExtRow) -> ExtRowWire {
    ExtRowWire {
        id: r.id.clone(),
        name: r.name.clone(),
        version: r.version.clone(),
        tier: r.tier.clone(),
        kind: r.kind.clone(),
        enabled: r.enabled,
        running: r.running,
        description: r.description.clone(),
        granted: r.granted.clone(),
        tools: r.tools,
        panels: r.panels,
        sub_agents: r.sub_agents,
        models: r.models,
        tui_screens: r
            .tui_screens
            .iter()
            .map(|t| ExtTuiScreenWire {
                id: t.id.clone(),
                title: t.title.clone(),
            })
            .collect(),
        workspace_dir: r.workspace_dir.clone(),
    }
}

/// Project the `/extension` dashboard: the installed-extension rows + cursors + sub-mode
/// token + any in-state error.
pub fn extensions_snapshot(st: &ExtensionsState) -> ExtensionsSnapshot {
    ExtensionsSnapshot {
        rows: st.rows.iter().map(ext_row_wire).collect(),
        list_sel: st.list_sel,
        mode: ext_submode_token(st.sub_mode).to_string(),
        screen_sel: st.screen_sel,
        error: st.error.clone(),
    }
}

/// Project an open extension screen: the ids + declared title, the opaque `Screen` value
/// (carried verbatim), the menu cursor, and the loading/error flags.
pub fn ext_screen_snapshot(st: &ExtScreenState) -> ExtScreenSnapshot {
    ExtScreenSnapshot {
        ext_id: st.ext_id.clone(),
        screen_id: st.screen_id.clone(),
        screen_title: st.screen_title.clone(),
        screen: st.screen.clone(),
        menu_cursor: st.menu_cursor,
        waiting: st.waiting,
        error: st.error.clone(),
    }
}

/// Project ONE `/store` catalogue row into its wire mirror (pure data, cloned verbatim).
fn store_row_wire(r: &StoreRow) -> ExtStoreRowWire {
    ExtStoreRowWire {
        id: r.id.clone(),
        name: r.name.clone(),
        tagline: r.tagline.clone(),
        tier: r.tier.clone(),
        kind: r.kind.clone(),
        latest_version: r.latest_version.clone(),
        author: r.author.clone(),
        installed: r.installed,
    }
}

/// Project the `/store` marketplace browser: the fetched catalogue rows + cursor +
/// sub-mode token + the Browse/Detail/InstallConfirm loading/error/data fields.
pub fn ext_store_snapshot(st: &ExtStoreState) -> ExtStoreSnapshot {
    ExtStoreSnapshot {
        rows: st.rows.iter().map(store_row_wire).collect(),
        list_sel: st.list_sel,
        mode: store_submode_token(st.sub_mode).to_string(),
        loading: st.loading,
        error: st.error.clone(),
        detail: st.detail.as_ref().map(|d| ExtStoreDetailWire {
            description: d.description.clone(),
            contributes_models: d.contributes_models,
            contributes_panels: d.contributes_panels,
            contributes_tools: d.contributes_tools,
            contributes_sub_agents: d.contributes_sub_agents,
            requires: d.requires.clone(),
            versions: d.versions.clone(),
        }),
        detail_loading: st.detail_loading,
        detail_error: st.detail_error.clone(),
        installing: st.installing,
        install_error: st.install_error.clone(),
        komarun_connected: st.komarun_connected,
    }
}

pub fn help_snapshot(h: &HelpState) -> HelpSnapshot {
    HelpSnapshot {
        query: h.query.clone(),
        all: h
            .all
            .iter()
            .map(|e| HelpEntrySnapshot {
                kind: help_kind_token(e.kind).to_string(),
                key: e.key.clone(),
                desc: e.desc.clone(),
            })
            .collect(),
        filtered_idx: h.filtered_idx.clone(),
        selected: h.selected,
        current_version: h.current_version.clone(),
        update: h.update.clone(),
    }
}

pub fn skill_cmd_snapshot(s: &SkillCmdState) -> SkillCmdSnapshot {
    SkillCmdSnapshot {
        query: s.query.clone(),
        chip: match s.chip {
            SkillFilterChip::All => "all",
            SkillFilterChip::Active => "active",
        }
        .to_string(),
        all: s
            .all
            .iter()
            .map(|e| SkillEntrySnapshot {
                name: e.name.clone(),
                description: e.description.clone(),
                is_active: e.is_active,
            })
            .collect(),
        filtered_idx: s.filtered_idx.clone(),
        selected: s.selected,
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

/// Project the `/remote` host manager.
pub fn remote_snapshot(m: &crate::app::mode::RemoteState) -> RemoteSnapshot {
    use crate::app::mode::remote::{ConnectionState, RemoteIntent, RemoteView};

    let connection_state_str: Option<String> = m.connection_state.as_ref().map(|cs| match cs {
        ConnectionState::Disconnected => "disconnected".into(),
        ConnectionState::Resolving => "resolving".into(),
        ConnectionState::Authenticating => "authenticating".into(),
        ConnectionState::AuthRequired {
            host_id,
            user,
            host,
        } => {
            format!("auth_required:{host_id}:{user}:{host}")
        }
        ConnectionState::Bootstrapping => "bootstrapping".into(),
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Connected { session_id } => format!("connected:{session_id}"),
        ConnectionState::Error { message } => format!("error:{message}"),
    });

    // Legacy stage/error fields (populated from connection_state for backward compat).
    let (stage, error) = match &m.connection_state {
        Some(ConnectionState::Resolving) => (Some("resolving".into()), None),
        Some(ConnectionState::Authenticating) => (Some("authenticating".into()), None),
        Some(ConnectionState::Bootstrapping) => (Some("bootstrapping".into()), None),
        Some(ConnectionState::Connecting) => (Some("connecting".into()), None),
        Some(ConnectionState::Connected { .. }) => (Some("connected".into()), None),
        Some(ConnectionState::Error { message }) => (None, Some(message.clone())),
        _ => (None, None),
    };

    // Encode intent + view as "intent:view" for the wire.
    let intent_token = match m.intent {
        RemoteIntent::Manage => "manage",
        RemoteIntent::Resume => "resume",
        RemoteIntent::New => "new",
    };
    let view_token = match m.view {
        RemoteView::Browse => "browse",
        RemoteView::SessionHub => "session_hub",
        RemoteView::Edit => "edit",
    };

    RemoteSnapshot {
        sub: format!("{intent_token}:{view_token}"),
        hosts: m
            .hosts
            .iter()
            .map(|h| RemoteHostSnapshot {
                id: h.id.clone(),
                name: h.name.clone(),
                user: h.user.clone(),
                host: h.host.clone(),
                port: h.port,
                key_path: h.key_path.clone(),
                connected: h.last_connected.is_some(),
                last_connected: h.last_connected,
                tags: h.tags.clone(),
            })
            .collect(),
        selected: m.selected,
        query: String::new(),
        filtered: m.filtered.clone(),
        detail_host: None,
        selected_host_id: m.selected_host_id.clone(),
        connection_state: connection_state_str,
        stage,
        error,
        sessions: m
            .sessions
            .iter()
            .map(|s| RemoteSessionSnapshot {
                session_id: s.session_id.clone(),
                name: s.name.clone(),
                working: s.working,
                is_foreground: s.is_foreground,
            })
            .collect(),
        session_selected: m.session_selected,
        pending_delete: m.pending_delete.clone(),
    }
}

/// Project the `/security` control panel.
///
/// Re-reads LIVE status from the daemon manager every snapshot — the panel always
/// reflects current daemon state after start/stop/restart, not just the state when
/// the mode was opened. Falls back to `s.status` (the in-mode snapshot) when there
/// is no manager (thin client path, which has no manager of its own).
pub fn security_snapshot(s: &SecurityState, state: &AppState) -> SecuritySnapshot {
    let status = state
        .rest
        .sec_manager
        .as_ref()
        .map(|m| m.status())
        .unwrap_or_else(|| s.status.clone());
    // The inactive set is authoritative on `state.rest`; project it sorted so the
    // wire form is deterministic.
    let mut inactive: Vec<String> = state.rest.sec_inactive.iter().cloned().collect();
    inactive.sort();
    SecuritySnapshot {
        status,
        selected: s.selected,
        inactive,
        // YOLO arm flag is authoritative on `state.rest`; carry it so the client's panel
        // renders the same armed/locked status row.
        yolo_armed: state.rest.yolo_armed,
        // Install-health is carried straight from the mode state — NEVER re-fetched
        // here. `health()` is a heavy IPC round-trip and this projection runs on every
        // frame; the mode seeds it once on open and after an install.
        install_health: s.install_health.clone(),
        health_view: s.health_view,
        health_selected: s.health_selected,
        // Spinner state for the in-flight health probe — projected so the client renders
        // the same "checking dependencies…" line and ANIMATES it from `health_frame`
        // (the daemon advances the frame every tick; the client owns no probe of its own).
        health_fetching: s.health_fetching,
        health_frame: s.health_frame,
    }
}
