//! Action handlers for the `/todo` task-panel: CloseTodo.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Handle `Action::CloseTodo`: return to Chat.
pub(super) fn handle_close_todo(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}
