//! Bash mode state: the working state for the `/bash` background-job panel.
//!
//! A READ-ONLY master/detail panel (modelled on `Mode::Agents`'s two-pane look):
//! the LEFT pane lists every background bash job registered this session; the
//! RIGHT pane shows the selected job's command + status + live output tail. The
//! only mutating action is killing a running job (`k`); there is no editing, no
//! create/delete, no pickers, and no sub-modes.
//!
//! The job rows ride as [`crate::ipc::proto::BashJobView`] — a serde-safe,
//! already-rendered projection of one [`crate::app::bgbash::BashJob`] (whose live
//! `Arc`/`Mutex`/`Instant` state can't cross the wire). The panel re-reads the
//! live registry every frame + on every key (see
//! `ipc::snapshot::projection::modes::bash_job_views`), exactly like the agents
//! dashboard re-reads its registry, so the cursor always clamps to current jobs.

use crate::ipc::proto::BashJobView;

/// Working state for the `/bash` background-job panel.
///
/// Holds the (already-projected) job list + the LIST cursor. No drafts, no
/// sub-modes — read-only + kill only.
#[derive(Debug, Clone, Default)]
pub struct BashState {
    /// Snapshot of the background bash jobs (one row per job), re-read live each
    /// frame from the foreground session's registry.
    pub jobs: Vec<BashJobView>,
    /// Selected index into `jobs` (the LIST cursor).
    pub selected: usize,
}

impl BashState {
    /// Build the panel from an initial job view list, cursor at the top.
    pub fn new(jobs: Vec<BashJobView>) -> Self {
        Self { jobs, selected: 0 }
    }

    /// Move the LIST cursor up (saturating at 0).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the LIST cursor down, clamped to the last job row.
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.jobs.len() {
            self.selected += 1;
        }
    }

    /// Replace the job list (from a fresh live read) and re-clamp the cursor so
    /// it never points past the end after a job list change.
    pub fn refresh(&mut self, jobs: Vec<BashJobView>) {
        self.jobs = jobs;
        if self.selected >= self.jobs.len() {
            self.selected = self.jobs.len().saturating_sub(1);
        }
    }

    /// The currently-selected job view, if any.
    pub fn current(&self) -> Option<&BashJobView> {
        self.jobs.get(self.selected)
    }
}
