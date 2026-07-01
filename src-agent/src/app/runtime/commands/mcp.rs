//! The `/mcp` command: open the MCP server management dashboard.

use anyhow::Result;

use crate::app::mode::{McpState, Mode};
use crate::app::state::AppState;

/// Handle the `/mcp` command: open the MCP server manager.
///
/// Does NOT require an active session: MCP servers are GLOBAL config
/// (`config.mcp_servers`), not session-scoped, so the dashboard opens regardless
/// of session state. Opening a panel is always safe mid-stream (read-only view;
/// the turn keeps streaming), so there is no busy guard here. The dashboard
/// seeds from `state.rest.config.mcp_servers`.
pub(super) fn handle_mcp(state: &mut AppState) -> Result<()> {
    let st = McpState::from(&state.rest.config.mcp_servers);
    *state.mode_mut() = Mode::Mcp(Box::new(st));
    Ok(())
}
