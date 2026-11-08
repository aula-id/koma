//! Direct full-screen controls. Selection never replays arrow keys; commits use
//! the same runtime actions as the legacy client.
use super::core::DaemonHub;
use crate::app::runtime::actions::apply_action;
use crate::app::{mode::*, state::AppState};
use crate::controller::input::{handle_key, Action};
use crate::ipc::proto::{DaemonEvent, MenuAction};
use crate::service::openrouter::OpenRouterClient;
use anyhow::{bail, ensure, Result};
use std::sync::Arc;

pub(super) struct MenuCall {
    pub request_id: String,
    pub mode: String,
    pub action: MenuAction,
}

impl DaemonHub {
    pub(super) fn menu(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        call: MenuCall,
    ) {
        let MenuCall {
            request_id,
            mode,
            action,
        } = call;
        let result = menu_action(state, client, handle, &mode, action);
        let exit = result.is_ok() && state.rest.should_quit;
        if exit {
            self.exit_delivery = self.clients.get(idx).map(|client| client.id);
        }
        self.send_to(
            idx,
            DaemonEvent::MenuResult {
                request_id,
                exit,
                error: result.err().map(|e| e.to_string()),
            },
        );
        self.resync_or_list_sessions(idx, state);
    }
}

fn menu_action(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    expected: &str,
    action: MenuAction,
) -> Result<()> {
    let snapshot = crate::ipc::snapshot::mode_snapshot(state);
    let value = serde_json::to_value(snapshot)?;
    let current = value
        .as_object()
        .and_then(|m| m.keys().next())
        .map(String::as_str)
        .or_else(|| value.as_str())
        .unwrap_or("");
    ensure!(
        current == expected,
        "This menu changed. Refresh and try again."
    );
    match action {
        MenuAction::Select { group, index, id } => {
            if group == "model_providers" {
                // Validate before taking the mode so errors cannot lose the form.
                let Mode::Settings(s) = state.mode() else {
                    bail!("Model form closed");
                };
                ensure!(s.model_provider_cursor.is_some(), "Provider chooser closed");
                let choices = s.mm_provider_choices();
                let p = choices
                    .iter()
                    .find(|p| p.index == index)
                    .ok_or_else(|| anyhow::anyhow!("Provider unavailable"))?;
                identity(id.as_deref(), &p.id)?;
                let mut mode = state.take_mode();
                if let Mode::Settings(s) = &mut mode {
                    crate::app::runtime::choose_model_provider(s, &mut state.rest, index);
                }
                state.set_mode(mode);
            } else {
                select(state, &group, index, id.as_deref())?;
            }
            Ok(())
        }
        MenuAction::Replace { field, value } => replace(state, &field, value),
        MenuAction::Save => {
            if let Mode::Settings(_) = state.mode() {
                if let Mode::Settings(s) = state.mode_mut() {
                    if let Some(e) = s.search_editor.as_mut() {
                        e.confirmed = true;
                    } else {
                        bail!("Settings apply when activated or confirmed")
                    }
                }
                let action = handle_key(
                    state,
                    ratatui::crossterm::event::KeyEvent::new(
                        ratatui::crossterm::event::KeyCode::Enter,
                        ratatui::crossterm::event::KeyModifiers::NONE,
                    ),
                );
                apply_action(action, state, client, handle)?;
                Ok(())
            } else {
                match state.mode_mut() {
                    Mode::Mcp(s) => s.editing = false,
                    Mode::Agents(s) => s.editing = false,
                    Mode::Remote(s) => s.editing_field = false,
                    _ => {}
                }
                let key = crate::ipc::proto::KeyWire {
                    code: crate::ipc::proto::KeyCodeWire::Char('s'),
                    mods: 0,
                };
                let action = handle_key(state, key.to_key_event());
                apply_action(action, state, client, handle)?;
                ensure!(
                    !matches!(state.mode(), Mode::Mcp(s) if matches!(s.mode, McpSubMode::Create | McpSubMode::Edit))
                        && !matches!(state.mode(), Mode::Agents(s) if matches!(s.mode, AgentSubMode::Create | AgentSubMode::Edit)),
                    "{}",
                    state.rest.fg().status
                );
                if let Mode::Remote(s) = state.mode() {
                    ensure!(
                        s.view != remote::RemoteView::Edit,
                        "{}",
                        s.editor
                            .as_ref()
                            .and_then(|e| e.error.as_deref())
                            .unwrap_or("Host validation failed")
                    );
                }
                Ok(())
            }
        }
        MenuAction::Discard if matches!(state.mode(), Mode::Settings(_)) => {
            let Mode::Settings(s) = state.mode_mut() else {
                let message = "Settings closed before the form could be cancelled.".to_string();
                state.rest.fg_mut().set_toast(message.clone());
                bail!("{message}");
            };
            if s.search_editor.is_some() {
                s.search_editor = None;
            } else if s.prov_modal.is_some() {
                s.close_provider_modal();
                s.page = settings::SettingsPage::Providers;
            } else if s.model_modal.is_some() {
                s.close_model_modal();
                s.page = settings::SettingsPage::Models;
            } else {
                bail!("This menu has no form to cancel");
            }
            Ok(())
        }
        MenuAction::Discard | MenuAction::KeepEditing => {
            bail!("This menu has no pending screen-level save")
        }
        MenuAction::Key(key) => {
            use ratatui::crossterm::event::KeyCode;
            let event = key.to_key_event();
            if let Mode::Settings(s) = state.mode_mut() {
                let top = s.page == settings::SettingsPage::Quick
                    && !s.editing
                    && !s.list_editing
                    && s.picker.is_none()
                    && s.search_editor.is_none();
                if top && matches!(event.code, KeyCode::Tab | KeyCode::BackTab) {
                    s.quick_focus = match s.quick_focus {
                        settings::QuickFocus::Appearance => settings::QuickFocus::General,
                        settings::QuickFocus::General => settings::QuickFocus::Appearance,
                    };
                    return Ok(());
                }
            }
            // Tab changes fields/groups, never walks a browse list.
            if matches!(event.code, KeyCode::Tab | KeyCode::BackTab) {
                let forward = event.code == KeyCode::Tab;
                match state.mode_mut() {
                    Mode::Settings(s)
                        if matches!(
                            s.page,
                            settings::SettingsPage::Providers
                                | settings::SettingsPage::OAuth
                                | settings::SettingsPage::Models
                        ) && s.picker.is_none()
                            && s.search_editor.is_none()
                            && matches!(s.oauth_flow, settings::OAuthFlowState::Idle) =>
                    {
                        use settings::SettingsPage::*;
                        let pages = [Quick, Providers, OAuth, Models];
                        let at = pages.iter().position(|p| *p == s.page).unwrap_or(0);
                        s.page = pages[(at + if forward { 1 } else { 3 }) % 4];
                        return Ok(());
                    }
                    Mode::Mcp(s) if matches!(s.mode, McpSubMode::Edit | McpSubMode::Create) => {
                        s.editing = false;
                        let fields = s.fields();
                        let at = fields.iter().position(|f| *f == s.field).unwrap_or(0);
                        s.field = fields
                            [(at + if forward { 1 } else { fields.len() - 1 }) % fields.len()];
                        return Ok(());
                    }
                    Mode::Agents(s)
                        if matches!(s.mode, AgentSubMode::Edit | AgentSubMode::Create)
                            && s.editor.is_none()
                            && s.tool_picker.is_none()
                            && s.model_picker.is_none() =>
                    {
                        s.editing = false;
                        let fields = s.fields();
                        let at = fields.iter().position(|f| *f == s.field).unwrap_or(0);
                        s.field = fields
                            [(at + if forward { 1 } else { fields.len() - 1 }) % fields.len()];
                        return Ok(());
                    }
                    Mode::Settings(s) if s.prov_modal.is_some() => {
                        s.provider_editing = false;
                        if let Some(m) = s.prov_modal.as_mut() {
                            m.field = (m.field + if forward { 1 } else { 4 }) % 5;
                        }
                        return Ok(());
                    }
                    Mode::Settings(s) if s.model_modal.is_some() => {
                        s.model_editing = false;
                        s.model_provider_cursor = None;
                        let len = s.model_modal_fields().len();
                        if let Some(m) = s.model_modal.as_mut() {
                            m.field = (m.field + if forward { 1 } else { len - 1 }) % len;
                        }
                        return Ok(());
                    }
                    Mode::Remote(s) if s.editor.is_some() => {
                        s.editing_field = false;
                        if let Some(e) = s.editor.as_mut() {
                            e.focused = if forward {
                                e.focused.next()
                            } else {
                                e.focused.prev()
                            };
                        }
                        return Ok(());
                    }
                    Mode::Agents(_)
                    | Mode::Mcp(_)
                    | Mode::Extensions(_)
                    | Mode::ExtStore(_)
                    | Mode::ExtScreen(_)
                    | Mode::Security(_)
                    | Mode::Help(_)
                    | Mode::Effort(_) => return Ok(()),
                    _ => {}
                }
            }
            if let Mode::Settings(s) = state.mode_mut() {
                if s.page == settings::SettingsPage::Models
                    && matches!(event.code, KeyCode::Up | KeyCode::Down)
                {
                    let len = 5 + s
                        .models
                        .iter()
                        .filter(|m| s.model_filter.matches(m.session_only))
                        .count();
                    s.model_sel = if event.code == KeyCode::Up {
                        s.model_sel.saturating_sub(1)
                    } else {
                        (s.model_sel + 1).min(len - 1)
                    };
                    return Ok(());
                }
            }
            let provider_save = matches!(state.mode(), Mode::Settings(s) if s.prov_modal.as_ref().is_some_and(|m| m.field==3))
                && event.code == KeyCode::Enter;
            let action = handle_key(state, event);
            apply_action(action, state, client, handle)?;
            if matches!(state.mode(), Mode::Settings(s) if !s.editing && s.dirty()) {
                crate::app::runtime::actions::commit_settings(state)?;
            }
            if provider_save {
                if let Mode::Settings(s) = state.mode() {
                    ensure!(
                        s.prov_modal.is_none(),
                        "{}",
                        s.prov_msg
                            .as_deref()
                            .unwrap_or("Provider validation failed")
                    );
                }
            }
            Ok(())
        }
    }
}

fn bounded(index: usize, len: usize) -> Result<usize> {
    ensure!(index < len, "The selected item is no longer available");
    Ok(index)
}
fn identity(id: Option<&str>, actual: &str) -> Result<()> {
    ensure!(
        id.is_none_or(|id| id == actual),
        "The selected item changed; select it again"
    );
    Ok(())
}
fn select(state: &mut AppState, group: &str, index: usize, id: Option<&str>) -> Result<()> {
    match state.mode_mut() {
        Mode::Settings(s) => {
            use settings::{OAuthFlowState, QuickFocus, SettingsPage};
            match group {
                "search_back" => {
                    let e = s
                        .search_editor
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Search form closed"))?;
                    e.confirmed = false;
                    e.editing = false;
                }
                "search_cancel" => {
                    s.search_editor = None;
                }
                "appearance" => {
                    s.finish_edit(false);
                    s.quick_focus = QuickFocus::Appearance;
                    s.palette_sel = bounded(index, crate::view::theme::PALETTES.len())?;
                }
                "general" => {
                    s.finish_edit(false);
                    s.quick_focus = QuickFocus::General;
                    s.field = bounded(index, settings::GENERAL_FIELDS.len())?;
                    s.editing = false;
                }
                "model_results" => {
                    let m = s
                        .model_modal
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Model form closed"))?;
                    m.result_sel = index;
                }
                "routes" => {
                    let m = s
                        .model_modal
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Model form closed"))?;
                    m.route_sel = bounded(index, m.endpoints.as_ref().map_or(0, Vec::len) + 1)?;
                }
                "page" => {
                    s.finish_edit(false);
                    s.page = *[
                        SettingsPage::Quick,
                        SettingsPage::Providers,
                        SettingsPage::OAuth,
                        SettingsPage::Models,
                    ]
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Unknown settings page"))?;
                }
                _ if s.search_editor.is_some() => {
                    let e = s
                        .search_editor
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Search form closed"))?;
                    let provider = *crate::model::web_search::SearchProvider::ALL
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("Unknown search provider"))?;
                    if e.provider != provider {
                        e.key.0.clear();
                        e.error = None;
                    }
                    e.provider = provider;
                    e.confirmed = true;
                    e.editing = provider != crate::model::web_search::SearchProvider::BuiltIn;
                }
                _ if s.picker.is_some() => {
                    let p = s
                        .picker
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Directory list closed"))?;
                    p.sel = bounded(index, p.matches.len())?;
                }
                _ if s.list_editing => {
                    s.list_sel = bounded(
                        index,
                        if s.field == 4 {
                            s.allowed_folders.len()
                        } else {
                            s.workdir.len()
                        },
                    )?;
                }
                _ if s.mm_role_picker_open() => {
                    let p = s
                        .model_modal
                        .as_mut()
                        .and_then(|modal| modal.role_picker.as_mut())
                        .ok_or_else(|| anyhow::anyhow!("Role picker closed"))?;
                    p.cursor = bounded(index, p.checked.len())?;
                }
                _ if s.prov_modal.is_some() => {
                    let field = bounded(index, 5)?;
                    if let Some(modal) = s.prov_modal.as_mut() {
                        modal.field = field;
                    }
                    s.provider_editing = index < 3;
                }
                _ if s.model_modal.is_some() => {
                    let len = s.model_modal_fields().len();
                    let field = bounded(index, len)?;
                    if let Some(modal) = s.model_modal.as_mut() {
                        modal.field = field;
                    }
                    s.model_editing = matches!(
                        s.mm_current_field(),
                        Some(settings::ModelField::Name | settings::ModelField::Model)
                    );
                    s.model_provider_cursor = if s.mm_current_field()
                        == Some(settings::ModelField::Provider)
                    {
                        let provider_idx = s.model_modal.as_ref().map(|modal| modal.provider_idx);
                        Some(
                            s.mm_provider_choices()
                                .iter()
                                .position(|choice| Some(choice.index) == provider_idx)
                                .unwrap_or(0),
                        )
                    } else {
                        None
                    };
                }
                _ if matches!(s.oauth_flow, OAuthFlowState::Pick(_)) => {
                    s.oauth_flow = OAuthFlowState::Pick(bounded(index, 8)?);
                }
                _ => match s.page {
                    SettingsPage::Quick | SettingsPage::Appearance | SettingsPage::General => {
                        if s.page == SettingsPage::General || s.quick_focus == QuickFocus::General {
                            s.field = bounded(index, settings::GENERAL_FIELDS.len())?;
                        } else {
                            s.palette_sel = bounded(index, crate::view::theme::PALETTES.len())?;
                        }
                    }
                    SettingsPage::Providers => {
                        if let Some(p) = s.providers.get(index) {
                            identity(id, &p.uuid)?;
                        }
                        s.prov_sel = bounded(index, s.providers.len() + 1)?;
                    }
                    SettingsPage::OAuth => {
                        if let Some(p) = s.oauth_drafts.get(index) {
                            identity(id, &p.uuid)?;
                        }
                        s.oauth_sel = bounded(index, s.oauth_drafts.len() + 1)?;
                    }
                    SettingsPage::Models => {
                        let visible: Vec<_> = s
                            .models
                            .iter()
                            .filter(|m| s.model_filter.matches(m.session_only))
                            .collect();
                        if let Some(m) = index.checked_sub(5).and_then(|n| visible.get(n)) {
                            identity(id, &m.uuid)?;
                        }
                        s.model_sel = bounded(index, 5 + visible.len())?;
                    }
                    SettingsPage::Menu => {
                        s.menu_sel = bounded(index, 5)?;
                    }
                    _ => bail!("No selectable list in this settings page"),
                },
            }
        }
        Mode::Agents(s) => {
            if group == "scope" {
                ensure!(
                    s.mode == AgentSubMode::Create,
                    "Scope is only editable while creating an agent"
                );
                s.create_scope = if bounded(index, 2)? == 0 {
                    AgentScope::Session
                } else {
                    AgentScope::Global
                };
            } else if let Some(p) = s.tool_picker.as_mut() {
                p.cursor = bounded(index, p.filtered_indices().len())?;
            } else if let Some(p) = s.model_picker.as_mut() {
                p.cursor = bounded(index, p.options.len())?;
            } else if matches!(s.mode, AgentSubMode::Edit | AgentSubMode::Create) {
                s.field = *s
                    .fields()
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Unknown agent field"))?;
                s.editing = false;
            } else {
                let at = bounded(index, s.agents.len())?;
                let name = s
                    .agents
                    .get(at)
                    .map(|agent| agent.name.clone())
                    .ok_or_else(|| anyhow::anyhow!("The selected item is no longer available"))?;
                identity(id, &name)?;
                s.list_sel = index;
            }
        }
        Mode::Mcp(s) => {
            if matches!(s.mode, McpSubMode::Edit | McpSubMode::Create) {
                s.field = *s
                    .fields()
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Unknown MCP field"))?;
                s.editing = false;
            } else {
                let p = &s.servers[bounded(index, s.servers.len())?];
                identity(id, &p.uuid)?;
                s.list_sel = index;
            }
        }
        Mode::Extensions(s) => {
            if group == "screens" {
                let len = s.rows.get(s.list_sel).map_or(0, |r| r.tui_screens.len());
                s.screen_sel = bounded(index, len)?;
            } else {
                identity(id, &s.rows[bounded(index, s.rows.len())?].id)?;
                s.list_sel = index;
            }
        }
        Mode::ExtStore(s) => {
            identity(id, &s.rows[bounded(index, s.rows.len())?].id)?;
            s.list_sel = index;
        }
        Mode::ExtScreen(s) => {
            let entries = s.menu_entries();
            let entry = &entries[bounded(index, entries.len())?];
            identity(id, &entry.0)?;
            s.menu_cursor = index;
        }
        Mode::Model(s) => {
            let option = s
                .options
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("Model option unavailable"))?;
            identity(id, option.0.as_deref().unwrap_or("__inherit__"))?;
            s.cursor = s
                .filtered_indices()
                .iter()
                .position(|i| *i == index)
                .ok_or_else(|| anyhow::anyhow!("Search results changed; choose again"))?;
        }
        Mode::Help(s) => {
            s.selected = bounded(index, s.filtered_idx.len())?;
        }
        Mode::Effort(s) => {
            s.selected = bounded(index, s.options.len())?;
        }
        Mode::Notifications(s) => {
            if let Some(e) = s.filtered().get(index) {
                identity(id, &e.id)?;
            }
            s.cursor = bounded(index, s.filtered().len())?;
        }
        Mode::Remote(s) => {
            if let Some(e) = s.editor.as_mut() {
                use remote::HostEditField::*;
                e.focused = *[Name, User, Host, Port, KeyPath]
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Unknown host field"))?;
                s.editing_field = false;
            } else if s.view == remote::RemoteView::SessionHub {
                s.session_selected = bounded(index, s.sessions.len())?;
            } else {
                let p = &s.hosts[s.filtered[bounded(index, s.filtered.len())?]];
                identity(id, &p.id)?;
                s.selected = index;
            }
        }
        Mode::Security(s) => match s.pane {
            crate::app::mode::SecPane::Refused => {
                s.refused_selected = index;
            }
            crate::app::mode::SecPane::Images => {
                s.images_selected = index;
            }
            crate::app::mode::SecPane::Tools => {
                s.selected = index;
            }
        },
        Mode::SessionPicker(s) => {
            s.selected = bounded(index, s.filtered_idx.len())?;
        }
        Mode::SessionHub(s) => {
            if group == "cooking" {
                s.focus = HubPane::Cooking;
                s.cooking_selected = bounded(index, s.cooking.len())?;
            } else {
                s.focus = HubPane::History;
                s.history_selected = bounded(index, s.history_filtered.len())?;
            }
        }
        Mode::Onboard(s) => {
            s.cursor = bounded(index, 3)?;
        }
        Mode::OnboardProvider(s) => {
            if let settings::OAuthFlowState::Pick(cursor) = &mut s.oauth_flow {
                *cursor = bounded(index, 8)?;
            } else {
                s.result_sel = index;
            }
        }
        Mode::KeyInput(s) => {
            if s.step == 0 {
                s.field = bounded(index, 2)?;
                s.editing = true;
            } else {
                s.result_sel = index;
            }
        }
        _ => bail!("Direct selection is unavailable in this menu"),
    }
    Ok(())
}

fn replace(state: &mut AppState, field: &str, value: String) -> Result<()> {
    if field == "editor" {
        let editor = match state.mode_mut() {
            Mode::Agents(s) => s.editor.as_mut().map(|(_, e)| e),
            Mode::Attachments(s) => s.editor.as_mut().map(|(_, e)| e),
            _ => None,
        }
        .ok_or_else(|| anyhow::anyhow!("The text editor is closed"))?;
        editor.lines = value.split('\n').map(String::from).collect();
        editor.row = editor.row.min(editor.lines.len().saturating_sub(1));
        editor.col = editor.col.min(editor.lines[editor.row].chars().count());
        return Ok(());
    }
    match state.mode_mut() {
        Mode::Settings(s) => {
            if let settings::OAuthFlowState::CodexPaste { input, .. } = &mut s.oauth_flow {
                ensure!(field == "oauth_token", "Unknown OAuth field");
                *input = value;
            } else if let Some(e) = s.search_editor.as_mut() {
                ensure!(field == "search_key", "Unknown search field");
                e.key = crate::model::web_search::SearchKey(value);
            } else if let Some(p) = s.picker.as_mut() {
                p.query = value;
                p.recompute(&s.cwd);
            } else if let Some(m) = s.prov_modal.as_mut() {
                match field {
                    "Name" | "name" => m.name = value,
                    "Endpoint" | "endpoint" => m.endpoint = value,
                    "API key" | "api_key" => m.api_key = value,
                    _ => bail!("Unknown provider field"),
                }
            } else if let Some(m) = s.model_modal.as_mut() {
                match field {
                    "Name" | "name" => m.name = value,
                    "Model" | "query" => {
                        m.query = value;
                        m.result_sel = 0;
                    }
                    _ => bail!("Unknown model field"),
                }
            } else {
                match field {
                    "Session name" | "name" => s.name = value,
                    "Max turns" => s.subagent_max_turns = value,
                    "Max out tokens" => s.max_output_tokens = value,
                    _ => bail!("Unknown general field"),
                }
            }
        }
        Mode::Mcp(s) => match field {
            "name" => s.draft_name = value,
            "command" => s.draft_command = value,
            "args" => s.draft_args = value,
            "env" => s.draft_env = value,
            "url" => s.draft_url = value,
            _ => bail!("Unknown MCP field"),
        },
        Mode::Agents(s) => match field {
            "name" => s.draft_name = value,
            "description" => s.draft_description = value,
            "conditions" => s.draft_conditions = value,
            "prompt" => s.draft_body = value,
            _ => bail!("Unknown agent field"),
        },
        Mode::Remote(s) => {
            let e = s
                .editor
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Host form is closed"))?;
            match field {
                "name" => e.name = value,
                "user" => e.user = value,
                "host" => e.host = value,
                "port" => e.port = value,
                "key_path" => e.key_path = value,
                _ => bail!("Unknown host field"),
            }
        }
        Mode::OnboardProvider(s) => {
            if let settings::OAuthFlowState::CodexPaste { input, .. } = &mut s.oauth_flow {
                *input = value;
            } else {
                s.query = value;
                s.result_sel = 0;
            }
        }
        Mode::Security(s) => match field {
            "pane" => {
                s.set_pane(
                    crate::app::mode::SecPane::parse(&value)
                        .unwrap_or(crate::app::mode::SecPane::Tools),
                );
            }
            _ => bail!("Unknown security field"),
        },
        Mode::KeyInput(s) => match field {
            "endpoint" => s.endpoint = value,
            "api_key" => s.api_key = value,
            "query" => {
                s.query = value;
                s.result_sel = 0;
            }
            _ => bail!("Unknown credential field"),
        },
        Mode::SessionPicker(s) => {
            ensure!(field == "query", "Unknown session search field");
            s.query = value;
            s.refilter();
        }
        Mode::SessionHub(s) => {
            ensure!(field == "history_query", "Unknown history search field");
            s.history_query = value;
            s.refilter_history();
        }
        Mode::Model(s) => {
            ensure!(
                field == "query" && s.searchable(),
                "This model picker has no search field"
            );
            s.set_query(value);
        }
        Mode::Help(s) => {
            s.query = value;
            s.refilter();
        }
        Mode::Notifications(s) => {
            s.query = value;
            s.cursor = 0;
        }
        _ => bail!("This menu has no editable field"),
    }
    if matches!(field, "Model" | "query") {
        let connection = match state.mode() {
            Mode::Settings(s) => s
                .mm_provider_conn()
                .map(|(ep, key)| (ep, key, s.mm_provider_oauth_uuid())),
            Mode::KeyInput(s) => Some((s.endpoint.clone(), s.api_key.clone(), String::new())),
            _ => None,
        };
        if let Some((endpoint, key, oauth)) = connection {
            state.rest.request_catalogue(&endpoint, &key, &oauth);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::proto::{KeyCodeWire, KeyWire};
    fn state() -> AppState {
        let config = crate::model::app_config::AppConfig::default();
        let session = crate::model::session::Session::new(
            "menu-test".into(),
            std::env::temp_dir().join("koma-menu-unit"),
            "test".into(),
            crate::model::settings::Settings {
                name: "Original".into(),
                ..Default::default()
            },
            crate::model::conversation::Conversation::new(""),
        );
        let settings = SettingsState::from(&session, &config);
        let mut state = AppState::new(Mode::Settings(Box::new(settings)));
        state.rest.config = config;
        state.rest.fg_mut().session = Some(session);
        state
    }
    fn key(code: KeyCodeWire) -> MenuAction {
        MenuAction::Key(KeyWire { code, mods: 0 })
    }
    #[test]
    fn direct_selection_and_tab_do_not_apply_the_highlight() {
        let mut state = state();
        let mut client = None;
        let rt = tokio::runtime::Runtime::new().unwrap();
        select(&mut state, "general", 5, None).unwrap();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Tab),
        )
        .unwrap();
        let Mode::Settings(s) = state.mode() else {
            panic!()
        };
        assert_eq!(s.quick_focus, settings::QuickFocus::Appearance);
        assert_eq!(
            s.field, 5,
            "Tab switches groups rather than moving the active list"
        );
        let saved = state.rest.config.palette.clone();
        select(&mut state, "appearance", 2, None).unwrap();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Down),
        )
        .unwrap();
        assert_eq!(state.rest.config.palette, saved);
        assert!(matches!(state.mode(),Mode::Settings(s) if !s.dirty()));
    }
    #[test]
    fn cancelling_native_text_restores_the_committed_value() {
        let mut state = state();
        let mut client = None;
        let rt = tokio::runtime::Runtime::new().unwrap();
        select(&mut state, "general", 0, None).unwrap();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Enter),
        )
        .unwrap();
        replace(&mut state, "name", "cancel this draft".into()).unwrap();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Esc),
        )
        .unwrap();
        assert!(
            matches!(state.mode(),Mode::Settings(s) if s.name=="Original" && !s.editing && !s.dirty())
        );
        assert_eq!(state.rest.fg().session.as_ref().unwrap().name, "Original");
    }
    #[test]
    fn wrong_mode_and_stale_ids_do_not_destroy_the_draft() {
        let mut state = state();
        let mut client = None;
        let rt = tokio::runtime::Runtime::new().unwrap();
        replace(&mut state, "name", "Keep this draft".into()).unwrap();
        assert!(menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Mcp",
            MenuAction::Save
        )
        .is_err());
        assert!(matches!(state.mode(),Mode::Settings(s) if s.name=="Keep this draft"));
        assert!(identity(Some("old"), "replacement").is_err());
    }
    #[test]
    fn native_text_replacement_does_not_trigger_mnemonics() {
        let mut state = state();
        select(&mut state, "general", 0, None).unwrap();
        replace(&mut state, "name", "a g s 2 😀".into()).unwrap();
        let Mode::Settings(s) = state.mode() else {
            panic!()
        };
        assert_eq!(s.name, "a g s 2 😀");
        assert_eq!(s.quick_focus, settings::QuickFocus::General);
    }
    #[test]
    fn editing_provider_preserves_identity_and_blank_secret() {
        let mut state = state();
        let Mode::Settings(s) = state.mode_mut() else {
            panic!()
        };
        s.providers.push(settings::ProviderDraft {
            uuid: "provider".into(),
            name: "Original".into(),
            endpoint: "https://example.invalid".into(),
            api_type: Default::default(),
            api_key: "secret".into(),
            ext_id: None,
        });
        s.prov_sel = s.providers.len() - 1;
        s.open_provider_modal_edit();
        let m = s.prov_modal.as_mut().unwrap();
        m.name = "Renamed".into();
        m.api_key.clear();
        assert!(s.save_provider_modal().is_none());
        assert!(s.model_modal.is_none());
        let p = &s.providers[s.prov_sel];
        assert_eq!(p.uuid, "provider");
        assert_eq!(p.api_key, "secret");
        assert_eq!(p.name, "Renamed");
    }
    #[test]
    fn adding_a_provider_opens_a_model_for_that_provider() {
        let mut state = state();
        let Mode::Settings(s) = state.mode_mut() else {
            panic!()
        };
        s.page = settings::SettingsPage::ProviderForm;
        s.open_provider_modal();
        let modal = s.prov_modal.as_mut().unwrap();
        modal.name = "Local".into();
        modal.endpoint = "http://127.0.0.1:9/v1".into();
        let idx = s.save_provider_modal().unwrap();
        s.jump_to_new_model(idx);
        assert_eq!(s.page, settings::SettingsPage::ModelForm);
        let model = s.model_modal.as_ref().unwrap();
        assert_eq!(model.provider_idx, idx);
        assert_eq!(model.field, 2);
        assert_eq!(s.providers[idx].name, "Local");
    }
    #[test]
    fn provider_chooser_uses_canonical_indices_and_cancels_without_mutation() {
        let mut state = state();
        let mut client = None;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let Mode::Settings(s) = state.mode_mut() else {
            panic!()
        };
        let provider = |id: &str, api_type| settings::ProviderDraft {
            uuid: id.into(),
            name: id.into(),
            endpoint: format!("http://127.0.0.1:9/{id}"),
            api_type,
            api_key: "fixture".into(),
            ext_id: None,
        };
        s.providers = vec![
            provider("first", Default::default()),
            provider("free", crate::model::app_config::ApiType::KomaFree),
            provider("last", Default::default()),
        ];
        s.oauth_drafts = vec![settings::OAuthDraft {
            uuid: "oauth".into(),
            label: "OAuth connection".into(),
            provider: crate::model::app_config::OAuthProvider::Codex,
            key: String::new(),
            status: "active".into(),
        }];
        s.page = settings::SettingsPage::ModelForm;
        s.open_model_modal_add(false);
        let choices = s.mm_provider_choices();
        assert_eq!(
            choices.iter().map(|p| p.index).collect::<Vec<_>>(),
            vec![0, 2, 3]
        );
        s.model_modal.as_mut().unwrap().field = 1;
        s.clean_fingerprint = s.fingerprint();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Enter),
        )
        .unwrap();
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Down),
        )
        .unwrap();
        assert!(
            matches!(state.mode(),Mode::Settings(s) if s.model_provider_cursor==Some(1) && s.model_modal.as_ref().unwrap().provider_idx==0)
        );
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Esc),
        )
        .unwrap();
        assert!(
            matches!(state.mode(),Mode::Settings(s) if s.model_provider_cursor.is_none() && s.model_modal.as_ref().unwrap().provider_idx==0)
        );
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            key(KeyCodeWire::Enter),
        )
        .unwrap();
        let Mode::Settings(s) = state.mode_mut() else {
            panic!()
        };
        let m = s.model_modal.as_mut().unwrap();
        m.query = "old query".into();
        m.route = Some("old upstream".into());
        m.endpoints_for = Some("old-model".into());
        m.endpoints_loading = true;
        let choose = |id: &str| MenuAction::Select {
            group: "model_providers".into(),
            index: 2,
            id: Some(id.into()),
        };
        assert!(menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            choose("stale")
        )
        .is_err());
        assert!(
            matches!(state.mode(),Mode::Settings(s) if s.model_modal.as_ref().unwrap().query=="old query")
        );
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            choose("last"),
        )
        .unwrap();
        let Mode::Settings(s) = state.mode() else {
            panic!()
        };
        let m = s.model_modal.as_ref().unwrap();
        assert_eq!(m.provider_idx, 2);
        assert!(m.query.is_empty());
        assert!(m.route.is_none());
        assert!(m.endpoints_for.is_none());
        assert!(!m.endpoints_loading);
    }
    #[test]
    fn search_pointer_switch_clears_unsaved_key_and_cancel_closes_form() {
        let mut state = state();
        let mut client = None;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let Mode::Settings(s) = state.mode_mut() else {
            panic!()
        };
        s.search_editor = Some(crate::model::web_search::SearchEditor::default());
        select(&mut state, "active", 2, None).unwrap();
        replace(&mut state, "search_key", "draft secret".into()).unwrap();
        select(&mut state, "search_back", 0, None).unwrap();
        assert!(
            matches!(state.mode(),Mode::Settings(s) if !s.search_editor.as_ref().unwrap().editing)
        );
        select(&mut state, "active", 1, None).unwrap();
        assert!(
            matches!(state.mode(),Mode::Settings(s) if s.search_editor.as_ref().unwrap().key.0.is_empty() && s.search_editor.as_ref().unwrap().editing)
        );
        menu_action(
            &mut state,
            &mut client,
            rt.handle(),
            "Settings",
            MenuAction::Discard,
        )
        .unwrap();
        assert!(matches!(state.mode(),Mode::Settings(s) if s.search_editor.is_none()));
    }

    #[test]
    fn model_pointer_selection_uses_canonical_identity_with_filtered_results() {
        let mut state = state();
        state.set_mode(Mode::Model(Box::new(ModelCmdState {
            sub: ModelCmdSub::RolePick {
                role: crate::model::app_config::ModelRole::Planner,
            },
            query: "beta".into(),
            from_roles: false,
            options: vec![
                (None, "inherit".into()),
                (Some("alpha".into()), "Alpha".into()),
                (Some("beta".into()), "Beta".into()),
            ],
            cursor: 0,
            note: String::new(),
        })));
        select(&mut state, "models", 2, Some("beta")).unwrap();
        assert!(
            matches!(state.mode(),Mode::Model(m) if m.selected_uuid().as_deref()==Some("beta") && m.cursor==0)
        );
        assert!(select(&mut state, "models", 2, Some("stale")).is_err());
        replace(&mut state, "query", "alpha".into()).unwrap();
        assert!(select(&mut state, "models", 2, Some("beta")).is_err());
        assert!(
            matches!(state.mode(),Mode::Model(m) if m.selected_uuid().as_deref()==Some("alpha"))
        );
    }
}
