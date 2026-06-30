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
//! - `Ctrl+X`        → `Action::BashKillJob(id)` for the selected job, ONLY when
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

    // Ctrl+X kills the selected running job — koma's kill convention (matches the
    // sub-agent abort in chat, the session hub, etc.). No-op on a finished/killed/
    // errored job (no signal, no toast). Checked before the keycode match because
    // is_ctrl inspects modifiers.
    if is_ctrl(&key, 'x') {
        return match s.current() {
            Some(job) if job.running => Action::BashKillJob(job.id),
            _ => Action::None,
        };
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
        // Any other key closes the panel (mirrors the /task sub-agents overlay,
        // which dismisses on any non-navigation key so the next keystroke types).
        _ => Action::CloseBash,
    }
}
