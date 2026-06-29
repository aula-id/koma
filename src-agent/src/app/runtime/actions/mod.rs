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
mod chat;
mod mcp;
// `pub(in crate::app::runtime)` so the `/quit` COMMAND handler (in the sibling
// `commands` module) can route through the same `request_quit` chokepoint as the
// quit keybind, instead of duplicating the working-aware open-or-quit logic.
pub(in crate::app::runtime) mod quit;
mod rewind;
mod session;
mod settings;

// Re-export the pwd-aware attach selector so the daemon's `Attach` handler (in the
// sibling `event_loop::daemon` module) can drive session selection through the SAME
// load/switch/create handlers the local TUI uses. Attach is not a keystroke, so it
// doesn't route through `apply_action` — it calls this directly. The `session` module
// is otherwise private to `actions`, so this single re-export is the only surface.
pub(in crate::app::runtime) use session::attach_select_for_pwd;
mod settings_creds;

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

        Action::Resend => {
            chat::handle_resend(state, client, handle)?;
        }

        Action::ApproveTool => {
            chat::handle_approve_tool(state, client, handle)?;
        }

        Action::DenyTool => {
            chat::handle_deny_tool(state)?;
        }

        Action::SaveCreds { endpoint, api_key, model } => {
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
            super::commands::apply_slash(
                crate::controller::command::Command::New(crate::controller::command::NewMode::Swap),
                state,
                client,
                handle,
            )?;
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
            state.mode = crate::app::mode::Mode::Chat;
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
            mcp::handle_create_mcp(state)?;
        }

        Action::SaveMcp => {
            mcp::handle_save_mcp(state)?;
        }

        Action::DeleteMcp => {
            mcp::handle_delete_mcp(state)?;
        }

        Action::CloseMcp => {
            mcp::handle_close_mcp(state)?;
        }

        Action::CloseHelp => {
            state.mode = crate::app::mode::Mode::Chat;
        }

        Action::HelpRun(cmd) => {
            // Close the reference first, then run the chosen command through the
            // SAME dispatcher a typed slash command uses (a mode-opening command
            // like `/mcp` will set its own mode, replacing this Chat).
            state.mode = crate::app::mode::Mode::Chat;
            // Don't re-dispatch the `/help` command itself — user is already leaving Help.
            if !matches!(cmd, crate::controller::command::Command::Help) {
                super::commands::apply_slash(cmd, state, client, handle)?;
            }
        }

        Action::FetchModelEndpoints(model_id) => {
            settings::handle_fetch_model_endpoints(model_id, state, client, handle)?;
        }

        Action::SkipLoading => {
            session::handle_skip_loading(state)?;
        }

        Action::CloseUsage => {
            state.mode = crate::app::mode::Mode::Chat;
            state.rest.status = "ready".into();
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
    }
    Ok(())
}
