//! Slash command dispatcher: apply a parsed slash command to app state.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::service::openrouter::OpenRouterClient;

mod cd;
mod compact;
mod effort;
// `pub(crate)` so the shared `internet_feedback` helper is reachable from the
// Ctrl+E handler (controller) and the settings-save action, which flip the same
// mode and must show the identical status line.
pub(crate) mod internet;
mod mcp;
mod misc;
pub(crate) mod new_session;
mod security;
mod task;

/// Apply a parsed slash command. Like [`apply_action`], it mutates state and
/// may spawn/abort the request task.
pub(super) fn apply_slash(
    cmd: Command,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    match cmd {
        Command::Compact => compact::handle_compact(state, client, handle)?,
        Command::New(mode) => new_session::handle_new(state, client, handle, mode)?,
        Command::Mode => misc::handle_mode(state)?,
        Command::Effort => effort::handle_effort(state, client)?,
        Command::Rename(name) => new_session::handle_rename(state, name)?,
        Command::Settings => misc::handle_settings(state)?,
        Command::Agents => misc::handle_agents(state)?,
        Command::Mcp => mcp::handle_mcp(state)?,
        Command::Security => security::handle_security(state)?,
        Command::Resume => new_session::handle_resume(state)?,
        Command::Select => misc::handle_select(state)?,
        Command::Help => misc::handle_help(state)?,
        Command::Usage => misc::handle_usage(state)?,
        Command::Quit => misc::handle_quit(state)?,
        Command::Task(args) => task::handle_task(args, state, client, handle)?,
        Command::Cd(path) => cd::handle_cd(path, state, client, handle)?,
        Command::AddDir(path) => cd::handle_adddir(path, state)?,
        Command::Internet(target) => internet::handle_internet(target, state)?,
        Command::Unknown(s) => {
            state.rest.status = format!("unknown command: /{s}");
        }
    }
    Ok(())
}
