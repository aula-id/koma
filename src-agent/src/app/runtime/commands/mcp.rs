//! The `/mcp` command: open the MCP server management dashboard.

use anyhow::Result;

use crate::app::mode::{McpState, Mode};
use crate::app::state::AppState;

/// Handle the `/mcp` command: open the MCP server manager.
///
/// Blocked while a request is in flight (busy guard), but — unlike `/agents` —
/// it does NOT require an active session: MCP servers are GLOBAL config
/// (`config.mcp_servers`), not session-scoped, so the dashboard opens regardless
/// of session state. The dashboard seeds from `state.rest.config.mcp_servers`.
pub(super) fn handle_mcp(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.fg_mut().status = "busy — wait for response".into();
        return Ok(());
    }
    let st = McpState::from(&state.rest.config.mcp_servers);
    *state.mode_mut() = Mode::Mcp(Box::new(st));
    Ok(())
}
