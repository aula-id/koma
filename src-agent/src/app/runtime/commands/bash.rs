//! The `/bash` command: open the background-job panel.

use anyhow::Result;

use crate::app::mode::{BashState, Mode};
use crate::app::state::AppState;

/// Handle the `/bash` command: open the read-only background bash-job panel.
///
/// Mirrors `/task` with no args — it just opens the panel. The job list is read
/// LIVE from the foreground session's registry (via `bash_job_views`) so the panel
/// opens populated; it then re-reads on every key (see `handle_bash`). Opening
/// a panel is always safe mid-stream (read-only view; the turn keeps streaming),
/// so there is no busy guard here.
pub(super) fn handle_bash(state: &mut AppState) -> Result<()> {
    let jobs = crate::ipc::snapshot::bash_job_views(&state.rest);
    let st = BashState::new(jobs);
    *state.mode_mut() = Mode::Bash(Box::new(st));
    Ok(())
}
