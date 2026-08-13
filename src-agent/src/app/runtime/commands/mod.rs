//! Slash command dispatcher: apply a parsed slash command to app state.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::service::openrouter::OpenRouterClient;

mod bash;
mod cd;
mod clear;
mod todo;
// `pub(crate)` so the plan-approval compaction rail (deferred/idle drain) can call
// `handle_compact` once the post-approval turn settles — the same entry point
// `/compact` uses, with `preserve_n = 0`.
pub(crate) mod compact;
// `pub(crate)` so the GUI `GetEffortOptions`/`SetEffort` daemon-request handlers
// (event_loop::daemon::hub::requests) can reuse `effort_menu` — the SAME
// per-model menu derivation (incl. the cold-cache fetch-arm side effect) the
// TUI's `/effort` uses, so both surfaces agree byte-for-byte.
pub(crate) mod effort;
// `pub(crate)` so the GUI model quick-picker's synthetic "advertised free" row can
// reuse `set_session_koma_free` — the SAME find-or-create-and-pin core `/free` runs —
// from the daemon `SetSessionMain` handler.
pub(crate) mod free;
// `pub(crate)` so the shared `internet_feedback` helper is reachable from the
// Ctrl+E handler (controller) and the settings-save action, which flip the same
// mode and must show the identical status line.
pub(crate) mod internet;
mod mcp;
pub(crate) mod model_cmd;
// `pub(crate)` so `build_extensions_state` (the `/extension` row builder) is reachable from
// `actions::extensions` (post-uninstall rebuild + ExtScreen pop-back) and the ext-screen
// close drain, which all derive the installed-extension rows the SAME way.
pub(crate) mod extensions;
mod misc;
pub(crate) mod new_session;
pub(crate) mod skill_cmd;
// `pub(crate)` so the shared `kick_off_health_probe` helper is reachable from BOTH the
// `/security` command (panel open) and the input-path self-heal (controller), which must
// start the non-blocking health probe with identical semantics.
pub(crate) mod security;
// `pub(crate)` so `store_row_from_item`/`store_detail_from_wire` (the `/store` row/detail
// mappers) are reachable from `event_loop::global::drains::drain_store`, which folds a
// landed catalogue/detail fetch into `Mode::ExtStore` the SAME way this module's own
// `handle_store` seeds it.
mod attach;
pub(crate) mod store;
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
        Command::Compact => compact::handle_compact(state, client, handle, None)?,
        Command::Clear => clear::handle_clear(state)?,
        Command::New(mode) => new_session::handle_new(state, client, handle, mode)?,
        Command::Mode(arg) => misc::handle_mode(state, arg)?,
        Command::Effort => effort::handle_effort(state, client)?,
        Command::Free => free::handle_free(state)?,
        Command::Rename(name) => new_session::handle_rename(state, name)?,
        Command::Settings => misc::handle_settings(state)?,
        Command::Agents => misc::handle_agents(state)?,
        Command::Mcp => mcp::handle_mcp(state)?,
        Command::Extensions => extensions::handle_extensions(state)?,
        Command::Store => store::handle_store(state, handle)?,
        Command::Security => security::handle_security(state)?,
        Command::Remote(args) => {
            if args.is_empty() {
                // Open the remote host manager.
                let hosts = crate::remote::hosts::load_hosts();
                state.set_mode(crate::app::mode::Mode::Remote(Box::new(
                    crate::app::mode::RemoteState::new(hosts.hosts),
                )));
            } else {
                // Ad-hoc: /remote user@host — set the break-out signal directly.
                state.rest.connect_remote_target = Some(args.to_string());
            }
        }
        Command::Resume => new_session::handle_resume(state)?,
        Command::Select => misc::handle_select(state)?,
        Command::Help => misc::handle_help(state)?,
        Command::Usage => misc::handle_usage(state)?,
        Command::Quit => misc::handle_quit(state)?,
        Command::Task(args) => task::handle_task(args, state, client, handle)?,
        Command::Model(args) => model_cmd::handle_model(args, state)?,
        Command::Bash => bash::handle_bash(state)?,
        Command::Skill(args) => skill_cmd::handle_skill(args, state)?,
        Command::Todo => todo::handle_todo(state)?,
        Command::Cd(path) => cd::handle_cd(path, state, client, handle)?,
        Command::AddDir(path) => cd::handle_adddir(path, state)?,
        Command::Internet(target) => internet::handle_internet(target, state)?,
        Command::Attach(arg) => attach::handle_attach(&arg, state)?,
        Command::Unknown(s) => {
            state.rest.fg_mut().status = format!("unknown command: /{s}");
        }
    }
    Ok(())
}
