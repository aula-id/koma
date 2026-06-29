//! App – top-level module wiring.
//!
//! Exposes the sub-modules that together own the application's lifecycle:
//!
//! - [`awareness`] – project-doc summarisation for the self-awareness block
//! - [`harness`] – safety classifier ("Pass B") + deterministic workspace check
//! - [`mcp`] – MCP (Model Context Protocol) client: connect to configured servers,
//!   discover their tools, advertise + dispatch them
//! - [`mode`] – [`Mode`] enum and associated per-mode state types
//! - [`resolve`] – per-role route resolution (model + provider + endpoint + key)
//! - [`runtime`] – event loop, terminal setup/teardown, and the main `run` function that ties controller + view together
//! - [`state`] – [`AppState`] (mode + rest) and [`AppStateRest`] (shared fields used across all modes: messages, input, client, …)
//! - [`subagent`] – self-contained autonomous sub-agent runtime (LLM-tool loop in a background task)
//!
//! [`run`] is re-exported at this level so callers only need `app::run(opts)`.
//! [`run_daemon`] is likewise re-exported for the headless `koma --daemon` path.

pub mod awareness;
pub mod harness;
pub mod mcp;
pub mod mode;
pub mod resolve;
pub mod runtime;
pub mod state;
pub mod subagent;

pub use runtime::client_run;
pub use runtime::run;
pub use runtime::run_daemon;
pub use runtime::run_daemon_selftest;
pub use runtime::{
    daemon_alive, ensure_daemon_running, print_daemon_usage, run_daemon_subcommand,
};
