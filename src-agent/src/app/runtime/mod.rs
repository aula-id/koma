//! Runtime: the synchronous event loop that ties the whole app together.
//!
//! Owns the terminal, the tokio runtime handle, and the `AppState`. Its job is
//! the central cycle: drain the active request's [`StreamEvent`]s -> read
//! terminal input -> turn keystrokes into `Action`s -> apply them by mutating
//! state -> redraw. This is the only place that spawns async tasks and the only
//! place that calls `view::draw`.
//!
//! Rendering is dirty-flagged (draw only after something changes) and input
//! polling is adaptive (8ms while a request streams so tokens flush at >=60fps,
//! 100ms when idle) so a quiet UI burns no CPU.
//!
//! Async bridge: one channel per request. [`start_stream_task`] opens a fresh
//! channel, stashes the receiver in `state.rest.fg().active_rx`, and spawns a task
//! holding the sender. Cancelling (interrupt / `/new` / quit) just drops the
//! receiver, so a superseded task's late events vanish with no generation
//! bookkeeping.

mod actions;
pub(crate) mod client;
mod client_shadow;
mod event_loop;
mod manage;
mod stream;
pub(crate) mod terminal;
// `pub(crate)` so the shared `commands::internet::internet_feedback` helper is
// reachable from the controller's Ctrl+E handler (outside this module tree).
pub(crate) mod commands;
mod shortsend;

mod lifecycle;
mod server;
#[cfg(feature = "linker")]
mod linker_daemon;
mod mcp_daemon;
mod oauth_daemon;
mod session_mgmt;
mod signals;
// Wave-5: persist + restore the per-session bg-bash / sub-agent records (#25).
pub(crate) mod bg_persist;
#[cfg(feature = "gui")]
pub mod gui;

// Shared stall detector (subagent engine; main-chat nudge reuses later).
pub(crate) use stream::is_stall;

// Re-export the sync-loop <-> per-client-task bridge message so the per-client
// connection task in `crate::ipc::conn` (outside this module tree) can name it.
pub(crate) use event_loop::daemon::HubInbound;

// Re-export the thin-attach-client entry so `app::client_run` reaches the
// `koma --attach` path (defined in the `client` submodule).
pub use client::client_run;

// Re-export the daemon management CLI entry + usage helper so `main` can short-circuit
// `koma daemon <verb>` before the TUI (defined in the `manage` submodule, #118).
//
// `daemon_alive` + `ensure_daemon_running` are the spawn-or-attach mechanism the
// default-launch flip (Stage 7) consumes: `daemon_alive(session_id)` is the per-session
// bind-as-oracle probe; `any_daemon_alive` is its any-session twin, which the `--local`
// guard uses to REFUSE running a second writer while ANY session-daemon is live;
// `ensure_daemon_running(session_id, …)` is the default path's "connect if up, else spawn
// a detached daemon and wait until it accepts" primitive (the thin client then attaches).
#[cfg(feature = "linker")]
pub use manage::ensure_linker_daemon_running;
pub use manage::{
    any_daemon_alive, ensure_daemon_running, migrate_legacy_daemon, print_daemon_usage,
    run_daemon_subcommand, run_doctor,
};

// Re-export the live-session discovery + cross-daemon spawn transport at the `runtime` level
// so the extension grant broker (`crate::app::ext::broker`, outside this module tree) can drive
// the extension `sessions.*` verbs (W7): `list_live_sessions` merges the registry against live
// daemons for `sessions.list`; `spawn_into_session` / `SpawnIntoReply` fire a one-shot
// `SpawnAgent` at another session-daemon's socket for the `sessions.spawn_into` cross-process
// branch. `manage` stays private; only these items are exposed.
pub(crate) use manage::{list_live_sessions, spawn_into_session, SpawnIntoReply};

// Re-export lifecycle entry points (previously free fns in this file).
pub use lifecycle::{run, run_daemon, run_daemon_selftest};
pub use server::run_server;

// Re-export the GLOBAL MCP daemon entry so `main` can dispatch `koma --mcp-daemon`
// (built in the `mcp_daemon` submodule). Additive: no session-daemon path uses it yet
// — the session-daemon MCP proxy in the next commit will.
#[cfg(feature = "linker")]
pub use linker_daemon::run_linker_daemon;
pub use mcp_daemon::run_mcp_daemon;
pub use oauth_daemon::run_oauth_daemon;

// Re-export session management helpers at the `runtime` level so sibling
// submodules that use `crate::app::runtime::build_client` / `super::warm_session`
// / `super::reconcile_session_lock` continue to resolve correctly.
pub(crate) use session_mgmt::{
    build_client, reconcile_session_lock, spawn_awareness_recompute, warm_session,
    warm_session_background,
};

// Re-export the sub-agent spawn primitive + its outcome at the `runtime` level so
// the extension grant broker (`crate::app::ext::broker`, outside this module tree)
// can drive the SAME `task`-tool spawn path. `stream` stays private; only these two
// items are exposed.
pub(crate) use stream::{spawn_or_queue, SpawnFailReason, SpawnOutcome};

// Re-export the live foreground-switch chokepoint at the `runtime` level so the extension
// grant broker (`crate::app::ext::broker`, outside this module tree) can drive the SAME
// in-daemon `sessions.switch` path the hub's `SwitchForeground` uses — it already fans out the
// `session.foreground_change` extension event (W5), so the broker must NOT re-emit. `actions`
// stays otherwise private; only this item is exposed.
pub(crate) use actions::session::handle_live_switch;

pub(super) type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

use ratatui::{backend::CrosstermBackend, Terminal};
