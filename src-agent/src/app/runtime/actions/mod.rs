//! Action dispatcher: apply a decoded keystroke action to app state.
//!
//! The module is split into focused submodules by concern:
//! - [`chat`]     — Submit, Interrupt, Resend, ApproveTool, DenyTool
//! - [`settings`] — SaveCreds, SaveSettings, SaveEffort, EffortCancel, FetchModelEndpoints
//! - [`session`]  — CancelKeyInput, CancelKeyInputToPicker, CancelPickerToChat, PickerSelect, LiveSwitch, HubOpenHistory, CloseSessionHub, SkipLoading
//! - [`agents`]   — CreateAgent, SaveAgent, DeleteAgent, CloseAgents

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::controller::input::Action;
use crate::service::openrouter::OpenRouterClient;

mod agents;
mod background;
mod bash;
mod chat;
mod todo;
// `pub(in crate::app::runtime)` so the daemon store hub (`event_loop::daemon::hub::
// requests_ext`, within `runtime`) can call `uninstall_extension_core` — the shared,
// hub-independent uninstall nuke the TUI `/extension` path also drives.
// `pub(in crate::app::runtime)` so the daemon store hub (`event_loop::daemon::hub::
// requests_ext`, within `runtime`) can call `install_extension_core` — the shared,
// hub-independent install tail the TUI `/store` path also drives.
pub(in crate::app::runtime) mod ext_install;
pub(in crate::app::runtime) mod ext_uninstall;
mod extensions;
mod mcp;
pub(crate) mod model_cmd;
mod oauth;
mod onboard;
mod plan_decision;
// `pub(in crate::app::runtime)` so the `/quit` COMMAND handler (in the sibling
// `commands` module) can route through the same `request_quit` chokepoint as the
// quit keybind, instead of duplicating the working-aware open-or-quit logic.
pub(in crate::app::runtime) mod quit;
mod rewind;
mod security;
// `pub(in crate::app::runtime)` so `runtime` can re-export `session::handle_live_switch` for
// the extension grant broker's `sessions.switch` (W7); the module's own items stay `pub`.
mod config_reload;
pub(in crate::app::runtime) mod session;
mod settings;
mod store;

// Re-export the pwd-explicit fresh-session creator. Daemon-per-session no longer creates
// a session on Attach (the daemon owns its one session from startup — see
// `lifecycle::install_daemon_session`, which mirrors this creator's construction), so
// this currently has NO in-tree caller; it is retained for the LATER `/new`-spawns-a-
// daemon commit and to keep the create logic in one place. `#[allow(unused_imports)]`
// keeps the dormant re-export warning-free at this commit.
#[allow(unused_imports)]
pub(in crate::app::runtime) use session::create_session_for_pwd;
// Re-export the mode-independent MCP save+reload so the daemon's GUI config setters
// (`SetMcpServer`/`DeleteMcpServer`/`EnableMcpServer`) can persist + live-reconnect the
// MCP manager without a `Mode::Mcp` in scope.
pub(crate) use config_reload::{apply_global_catalogue_reload, save_config_and_broadcast};
pub(in crate::app::runtime) use mcp::save_and_reload_mcp;
mod settings_creds;

/// Apply mouse-capture mode to the terminal. Resolves `Auto` (always true),
/// then sends the appropriate crossterm escape sequence.
/// Called at session init and on settings save.
pub(in crate::app::runtime) fn apply_mouse_capture(mode: crate::model::settings::MouseCapture) {
    use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use ratatui::crossterm::execute;
    let enabled = mode.resolved();
    let _ = if enabled {
        execute!(std::io::stdout(), EnableMouseCapture)
    } else {
        execute!(std::io::stdout(), DisableMouseCapture)
    };
}

/// Apply one `Action` (the decoded result of a keystroke) by mutating state and,
/// where needed, spawning/aborting the request task.
///
/// `pub(in crate::app::runtime)` (not just `pub(super)`) so the headless daemon
/// loop (`event_loop::daemon`) can drive the SAME action handlers the local TUI
/// uses: a daemon client's `SubmitInput` / `SendKey` / `ApproveTool` / `NewSession`
/// / `SwitchForeground` request is translated to the corresponding `Action` and
/// funnelled through here, so the daemon never forks the turn/submit/approval logic.
pub(in crate::app::runtime) fn apply_action(
    action: Action,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    match action {
        Action::None => {}

        Action::Quit => {
            // Quit chokepoint: quit immediately if nothing is working, else open
            // the kill-all / detach / cancel confirm overlay.
            quit::request_quit(state);
        }

        Action::QuitKillAll => {
            quit::handle_quit_kill_all(state);
        }

        Action::QuitDetach => {
            quit::handle_quit_detach(state);
        }

        Action::QuitCancel => {
            quit::handle_quit_cancel(state);
        }

        Action::Submit(text) => {
            chat::handle_submit(text, state, client, handle)?;
        }

        Action::Shell(cmd) => {
            chat::handle_shell(cmd, state)?;
        }

        Action::Slash(cmd) => {
            super::commands::apply_slash(cmd, state, client, handle)?;
        }

        Action::Interrupt => {
            chat::handle_interrupt(state)?;
        }

        Action::InterruptRewind => {
            chat::handle_interrupt_rewind(state)?;
        }

        Action::ClearComposer => {
            state.rest.fg_mut().clear_composer();
        }

        Action::Resend => {
            chat::handle_resend(state, client, handle)?;
        }

        Action::ApproveTool => {
            chat::handle_approve_tool(state, client, handle)?;
        }

        Action::DenyTool => {
            chat::handle_deny_tool(state)?;
        }

        Action::ApprovePlan => {
            if plan_decision::is_pending_mission_ready(state) {
                plan_decision::handle_approve_mission(state, client, handle)?;
            } else {
                plan_decision::handle_approve_plan(state, client, handle)?;
            }
        }

        Action::ApprovePlanCompact => {
            if plan_decision::is_pending_mission_ready(state) {
                plan_decision::handle_approve_mission_compact(state, client, handle)?;
            } else {
                plan_decision::handle_approve_plan_compact(state, client, handle)?;
            }
        }

        Action::DenyPlan => {
            if plan_decision::is_pending_mission_ready(state) {
                plan_decision::handle_deny_mission(state, client, handle)?;
            } else {
                plan_decision::handle_deny_plan(state, client, handle)?;
            }
        }

        Action::SetupKomaFree => {
            onboard::handle_setup_koma_free(state, client, handle)?;
        }

        Action::OnboardProvider => {
            onboard::handle_onboard_provider(state)?;
        }

        Action::OnboardCustom => {
            onboard::handle_onboard_custom(state)?;
        }

        Action::OnboardProviderSaveModel(model_id) => {
            onboard::handle_onboard_provider_save_model(model_id, state, client, handle)?;
        }

        Action::OnboardProviderBack => {
            onboard::handle_onboard_provider_back(state)?;
        }

        Action::SaveCreds {
            endpoint,
            api_key,
            model,
        } => {
            settings_creds::handle_save_creds(endpoint, api_key, model, state, client, handle)?;
        }

        Action::CancelKeyInput => {
            session::handle_cancel_key_input(state, client)?;
        }

        Action::CancelKeyInputToPicker => {
            session::handle_cancel_key_input_to_picker(state, client)?;
        }

        Action::CancelPickerToChat => {
            session::handle_cancel_picker_to_chat(state)?;
        }

        Action::PickerSelect => {
            session::handle_picker_select(state, client, handle)?;
        }

        Action::PickerNewSession => {
            // The session-PICKER's `[+ new session]` row must create an in-process session
            // RIGHT HERE (same daemon), so call `apply_new_session_local` directly — NOT the
            // `/new` slash command, which (daemon-per-session) merely sets `new_pending` and
            // would make the client tear this daemon down + spawn a whole new one. `kill =
            // false` (Swap): keep any previous foreground cooking.
            super::commands::new_session::apply_new_session_local(state, client, handle, false)?;
        }

        Action::LiveSwitch(idx) => {
            session::handle_live_switch(idx, state, client)?;
        }

        Action::HubOpenHistory(idx) => {
            session::handle_hub_open_history(idx, state, client, handle)?;
        }

        Action::HubKillConfirm => {
            session::handle_hub_kill_confirm(state, client, handle)?;
        }

        Action::HubDeleteConfirm => {
            session::handle_hub_delete_confirm(state)?;
        }

        Action::CloseSessionHub => {
            session::handle_close_session_hub(state)?;
        }

        Action::SaveSettings => {
            settings::handle_save_settings(state)?;
        }

        Action::SaveEffort(choice) => {
            settings::handle_save_effort(choice, state)?;
        }

        Action::EffortCancel => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }

        Action::ModelRoleSwap { role, model_uuid } => {
            model_cmd::handle_model_role_swap(role, model_uuid, state)?;
        }

        Action::ModelAgentSwap {
            agent_name,
            model_uuid,
        } => {
            model_cmd::handle_model_agent_swap(agent_name, model_uuid, state)?;
        }

        Action::ModelCancel => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }

        Action::ModelBackToAgentList => {
            model_cmd::handle_model_back_to_agent_list(state);
        }

        Action::ModelOpenAgentPick { agent_name } => {
            model_cmd::handle_model_open_agent_pick(agent_name, state);
        }

        Action::CreateAgent => {
            agents::handle_create_agent(state)?;
        }

        Action::SaveAgent => {
            agents::handle_save_agent(state)?;
        }

        Action::DeleteAgent => {
            agents::handle_delete_agent(state)?;
        }

        Action::CloseAgents => {
            agents::handle_close_agents(state)?;
        }

        Action::CreateMcp => {
            mcp::handle_create_mcp(state, handle)?;
        }

        Action::SaveMcp => {
            mcp::handle_save_mcp(state, handle)?;
        }

        Action::DeleteMcp => {
            mcp::handle_delete_mcp(state, handle)?;
        }

        Action::CloseMcp => {
            mcp::handle_close_mcp(state)?;
        }

        Action::CloseExtensions => {
            extensions::handle_close_extensions(state)?;
        }

        Action::UninstallExtension => {
            extensions::handle_uninstall_extension(state, handle)?;
        }

        Action::ExtScreenOpen => {
            extensions::handle_ext_screen_open(state, handle)?;
        }

        Action::ExtScreenSelect => {
            extensions::handle_ext_screen_select(state, handle)?;
        }

        Action::ExtScreenClose => {
            extensions::handle_ext_screen_close(state, handle)?;
        }

        Action::CloseStore => {
            store::handle_close_store(state)?;
        }

        Action::StoreRetryBrowse => {
            store::handle_store_retry_browse(state, handle)?;
        }

        Action::StoreOpenDetail => {
            store::handle_store_open_detail(state, handle)?;
        }

        Action::StoreInstallConfirm => {
            store::handle_store_install_confirm(state, handle)?;
        }

        Action::CloseSecurity => {
            security::handle_close_security(state)?;
        }

        // Daemon start/stop/toggle no longer have their own Actions — the Daemon checkbox
        // (handled inside `SecurityToggleTool`) starts/stops the daemon directly, calling
        // `handle_security_start` / `handle_security_stop`. Only restart keeps a key (`r`).
        Action::SecurityRestart => {
            security::handle_security_restart(state)?;
        }

        Action::SecurityToggleTool => {
            security::handle_security_toggle_tool(state)?;
        }

        Action::SecurityToggleDomain => {
            security::handle_security_toggle_domain(state)?;
        }

        Action::SecurityInstall(key) => {
            security::handle_security_install(key, state)?;
        }

        Action::CloseBash => {
            bash::handle_close_bash(state)?;
        }

        Action::CloseTodo => {
            todo::handle_close_todo(state)?;
        }

        Action::BashKillJob(id) => {
            bash::handle_bash_kill(id, state)?;
        }

        Action::CloseSkill => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }

        Action::SkillToggle(name) => {
            let sess_idx = state.rest.foreground;
            let is_active = state.rest.sessions[sess_idx]
                .active_skills
                .contains_key(&name);
            if is_active {
                let msg = super::commands::skill_cmd::deactivate_skill(state, sess_idx, &name);
                state.rest.fg_mut().status = msg;
            } else {
                match super::commands::skill_cmd::activate_skill(state, sess_idx, &name) {
                    Ok(msg) => state.rest.fg_mut().status = msg,
                    Err(e) => state.rest.fg_mut().status = format!("error: {e}"),
                }
            }
            // Refresh the hub state's is_active flags
            if let crate::app::mode::Mode::Skill(s) = state.mode_mut() {
                s.set_active(&name, !is_active);
            }
        }

        Action::CloseHelp => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }

        Action::HelpRun(cmd) => {
            // Close the reference first, then run the chosen command through the
            // SAME dispatcher a typed slash command uses (a mode-opening command
            // like `/mcp` will set its own mode, replacing this Chat).
            *state.mode_mut() = crate::app::mode::Mode::Chat;
            // Don't re-dispatch the `/help` command itself — user is already leaving Help.
            if !matches!(cmd, crate::controller::command::Command::Help) {
                super::commands::apply_slash(cmd, state, client, handle)?;
            }
        }

        Action::FetchModelEndpoints(model_id) => {
            settings::handle_fetch_model_endpoints(model_id, state, client, handle)?;
        }

        Action::OAuthStart(provider) => {
            oauth::handle_oauth_start(provider, state, handle)?;
        }

        Action::OAuthCancel => {
            oauth::handle_oauth_cancel(state)?;
        }

        Action::OAuthPaste { provider, token } => {
            oauth::handle_oauth_paste(provider, token, state, handle)?;
        }

        Action::OAuthDelete(uuid) => {
            oauth::handle_oauth_delete(uuid, state, handle)?;
        }

        Action::OAuthCopyUrl => {
            oauth::handle_oauth_copy_url(state)?;
        }

        Action::OAuthOpenUrl => {
            oauth::handle_oauth_open_url(state)?;
        }

        Action::SkipLoading => {
            session::handle_skip_loading(state)?;
        }

        Action::CloseUsage => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
            state.rest.fg_mut().status = "ready".into();
        }

        Action::CancelSteers => {
            let n = state.rest.fg().pending_steer.len();
            state.rest.fg_mut().pending_steer.clear();
            if n > 0 {
                state.rest.fg_mut().status = "steering queue cleared".into();
            }
        }

        Action::BackgroundSubagent(id) => {
            background::handle_background_subagent(id, state)?;
        }

        Action::BackgroundAllSubagents => {
            background::handle_background_all_subagents(state)?;
        }

        Action::OpenRewind => {
            rewind::handle_open_rewind(state)?;
        }

        Action::RewindCancel => {
            rewind::handle_rewind_cancel(state)?;
        }

        Action::RewindToMessage(idx) => {
            rewind::handle_rewind_to_message(idx, state)?;
        }

        Action::CloseRemote => {
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }

        Action::RemoteAddHost
        | Action::RemoteConnect(_)
        | Action::RemoteEditHost(_)
        | Action::RemoteDeleteHost(_)
        | Action::RemotePasswordSubmit(_) => {
            // Phase 3 TODO: implement actual connection/edit/delete logic.
            // For now, close the remote view and show status.
            *state.mode_mut() = crate::app::mode::Mode::Chat;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "prompt_contract_test.rs"]
mod prompt_contract_tests;
