//! Key handler for the `/bash` background-job panel (`Mode::Bash`).
//!
//! A read-only master/detail panel — no sub-modes and no editing, so the
//! dispatch is simple: navigate the job list, view the selected job's output,
//! kill a running job, close.
//!
//! Key map:
//! - `Esc`           → `Action::CloseBash` (return to Chat)
//! - `Ctrl+C`        → `Action::Quit`
//! - `Up`            → move the LIST cursor up
//! - `Down`          → move the LIST cursor down
//! - `k`/`K`         → `Action::BashKillJob(id)` for the selected job, ONLY when
//!   it is still running (a no-op on a finished/killed/errored job)

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::BashState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key press inside the `/bash` background-job panel.
///
/// Re-reads the LIVE job registry from the foreground session BEFORE handling
/// navigation so the cursor clamps to the current jobs (a background job may have
/// finished — or a new one started — since the last frame), exactly like the
/// agents/security panels re-read their live state on each key.
pub fn handle_bash(s: &mut BashState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // Refresh from the live registry first so the cursor + the kill target resolve
    // against current jobs. `bash_job_views` reads the foreground session's
    // `bash_jobs` and projects each into a wire-safe view (the same projection the
    // snapshot uses), so the panel and the daemon never diverge.
    s.refresh(crate::ipc::snapshot::bash_job_views(rest));

    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::CloseBash,
        KeyCode::Up => {
            s.move_up();
            Action::None
        }
        KeyCode::Down => {
            s.move_down();
            Action::None
        }
        // Kill the selected job — only meaningful while it is still running. On a
        // finished/killed/errored job this is a no-op (no signal, no toast).
        KeyCode::Char('k') | KeyCode::Char('K') => match s.current() {
            Some(job) if job.running => Action::BashKillJob(job.id),
            _ => Action::None,
        },
        _ => Action::None,
    }
}
