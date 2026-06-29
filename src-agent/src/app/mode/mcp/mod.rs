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
pub use types::{McpEditField, McpSubMode};
