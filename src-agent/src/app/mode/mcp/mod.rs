//! MCP-mode types: the sub-mode state machine and the [`McpState`] draft holder
//! for the in-app `/mcp` server management dashboard.
//!
//! Modelled on `/agents` (a LIST + DETAIL two-pane layout with a small state
//! machine), but simpler: MCP servers persist in `config.json`, so there are no
//! markdown files, no model/tool pickers, and no full-screen body editor. The
//! data layer is just `config.mcp_servers` (a `Vec<McpServerEntry>`); this module
//! holds the working drafts + navigation state, and the runtime
//! (`app::runtime::actions::mcp`) reads them back to mutate + persist the config.

mod state;
mod types;

pub use state::{transport_label, McpState};
// Re-exported so the GUI config setters (daemon `SetMcpServer` handler) can map the
// panel's single-line args/env STRING forms into the daemon's array/pair forms using
// the SAME parsers the TUI MCP editor uses — no forked parsing logic.
pub(crate) use state::{parse_args, parse_env};
pub use types::{McpEditField, McpSubMode};
