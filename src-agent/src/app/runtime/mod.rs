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

mod terminal;
mod event_loop;
mod stream;
mod actions;
mod client;
mod client_shadow;
mod manage;
// `pub(crate)` so the shared `commands::internet::internet_feedback` helper is
// reachable from the controller's Ctrl+E handler (outside this module tree).
pub(crate) mod commands;
mod shortsend;

mod lifecycle;
mod mcp_daemon;
mod signals;
mod session_mgmt;

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
pub use manage::{
    any_daemon_alive, ensure_daemon_running, migrate_legacy_daemon, print_daemon_usage,
    run_daemon_subcommand,
};

// Re-export lifecycle entry points (previously free fns in this file).
pub use lifecycle::{run, run_daemon, run_daemon_selftest};

// Re-export the GLOBAL MCP daemon entry so `main` can dispatch `koma --mcp-daemon`
// (built in the `mcp_daemon` submodule). Additive: no session-daemon path uses it yet
// — the session-daemon MCP proxy in the next commit will.
pub use mcp_daemon::run_mcp_daemon;

// Re-export session management helpers at the `runtime` level so sibling
// submodules that use `crate::app::runtime::build_client` / `super::warm_session`
// / `super::reconcile_session_lock` continue to resolve correctly.
pub(crate) use session_mgmt::{
    build_client, reconcile_session_lock, spawn_awareness_recompute, warm_session,
    warm_session_background,
};

pub(super) type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

use ratatui::{backend::CrosstermBackend, Terminal};
