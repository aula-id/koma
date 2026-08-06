//! App – top-level module wiring.
//!
//! Exposes the sub-modules that together own the application's lifecycle:
//!
//! - [`awareness`] – project-doc summarisation for the self-awareness block
//! - [`bgbash`] – background-bash registry: run a shell command detached, poll it
//!   with `bash_output`, stop it with `bash_kill`
//! - [`ext`] – extension host: install + verify signed extension packages and
//!   spawn/supervise them, talking the duplex extension protocol over a per-
//!   extension unix socket
//! - [`harness`] – safety classifier ("Pass B") + deterministic workspace check
//! - [`mcp`] – MCP (Model Context Protocol) client: connect to configured servers,
//!   discover their tools, advertise + dispatch them
//! - [`mode`] – [`Mode`] enum and associated per-mode state types
//! - [`resolve`] – per-role route resolution (model + provider + endpoint + key)
//! - [`runtime`] – event loop, terminal setup/teardown, and the main `run` function that ties controller + view together
//! - [`sec`] – security daemon client: spawn the Python `koma_sec_daemon`, discover its tools, advertise + dispatch them over newline-delimited JSON
//! - [`state`] – [`AppState`] (mode + rest) and [`AppStateRest`] (shared fields used across all modes: messages, input, client, …)
//! - [`subagent`] – self-contained autonomous sub-agent runtime (LLM-tool loop in a background task)
//! - [`version`] – self-update awareness: compiled-in version + non-blocking check against the public version endpoint
//!
//! [`run`] is re-exported at this level so callers only need `app::run(opts)`.
//! [`run_daemon`] is likewise re-exported for the headless `koma --daemon` path.

pub mod awareness;
pub mod bgbash;
pub mod cascade;
pub mod ext;
pub mod harness;
pub mod mcp;
pub mod mode;
pub mod resolve;
pub mod runtime;
pub mod sec;
pub mod state;
pub mod subagent;
pub mod update;
pub mod version;

pub use ext::{print_ext_usage, run_ext_install_dev};
pub use runtime::client_run;
pub use runtime::ensure_linker_daemon_running;
#[cfg(feature = "gui")]
pub use runtime::gui::run_gui;
pub use runtime::run;
pub use runtime::run_daemon;
pub use runtime::run_daemon_selftest;
pub use runtime::run_linker_daemon;
pub use runtime::run_mcp_daemon;
pub use runtime::run_oauth_daemon;
pub use runtime::{
    any_daemon_alive, ensure_daemon_running, migrate_legacy_daemon, print_daemon_usage,
    run_daemon_subcommand, run_doctor,
};
pub use update::run_update;
