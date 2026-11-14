//! The `/security` command: ask the OpenTUI client to enter the secure room.

use anyhow::Result;

use crate::app::state::AppState;

/// `/security` asks the OpenTUI client to hand this session to the secure room.
/// It does not open a panel. `koma legacy` drains the same flag into a status
/// line and stays in the coding chat.
pub(super) fn handle_security(state: &mut AppState) -> Result<()> {
    state.rest.fg_mut().status = "/security is not part of legacy".into();
    state.rest.secure_handoff_pending = true;
    Ok(())
}
