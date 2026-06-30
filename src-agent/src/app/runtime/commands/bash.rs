//! The `/bash` command: open the background-job panel.

use anyhow::Result;

use crate::app::mode::{BashState, Mode};
use crate::app::state::AppState;

/// Handle the `/bash` command: open the read-only background bash-job panel.
///
/// Mirrors `/task` with no args — it just opens the panel. The job list is read
/// LIVE from the foreground session's registry (via `bash_job_views`) so the panel
/// opens populated; it then re-reads on every key (see `handle_bash`). Blocked
/// while a request is in flight, matching the `/agents` / `/settings` busy guard.
pub(super) fn handle_bash(state: &mut AppState) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    let jobs = crate::ipc::snapshot::bash_job_views(&state.rest);
    let st = BashState::new(jobs);
    *state.mode_mut() = Mode::Bash(Box::new(st));
    Ok(())
}
