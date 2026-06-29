//! Action handlers for the `/bash` background-job panel: CloseBash, BashKillJob.
//!
//! A read-only + kill panel — the only state-changing action is killing a running
//! job. After a kill the panel stays open; its next key refreshes the job list
//! from the live registry (see `controller::input::handle_bash`), so the killed
//! job re-renders with its `killed` status on the very next frame.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Handle `Action::CloseBash`: return to Chat.
pub(super) fn handle_close_bash(state: &mut AppState) -> Result<()> {
    state.mode = Mode::Chat;
    state.rest.status = "ready".into();
    Ok(())
}

/// Handle `Action::BashKillJob(id)`: terminate the running background bash job with
/// the given id in the FOREGROUND session.
///
/// Resolves the job out of the live `rt.bash_jobs` registry by id (never by Vec
/// position — the model addresses jobs as `bash-<id>`) and signals it via
/// [`crate::app::bgbash::kill_bash_job`] (SIGTERM + flip status to `Killed`). The
/// panel is LEFT OPEN: the input handler refreshes the list on the next key, so the
/// row updates to `killed` then. A no-op when the id is absent (already gone).
pub(super) fn handle_bash_kill(id: usize, state: &mut AppState) -> Result<()> {
    let job = state.rest.fg().bash_jobs.iter().find(|j| j.id == id);
    if let Some(job) = job {
        crate::app::bgbash::kill_bash_job(job);
        state.rest.status = format!("killed bash-{id}");
        state.rest.set_toast_info(format!("killed bash-{id}"));
    }
    Ok(())
}
