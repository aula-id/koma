//! Pure state transitions for assignments and batches.
//!
//! Ported from the Orpheus reconciler (`reconcileGroupWork`). Nothing here
//! touches storage or time: callers pass in what they observed and apply the
//! result, so every rule is testable in isolation.

use sha2::{Digest, Sha256};

use super::types::{AssignmentStatus, Outcome, WorkStatus};

/// State of the lane run behind an assignment, as observed by the reconciler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneRunState {
    Running,
    WaitingInput,
    WaitingTakeover,
    Completed,
    Failed,
    Cancelled,
}

impl LaneRunState {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            LaneRunState::Completed | LaneRunState::Failed | LaneRunState::Cancelled
        )
    }
}

/// What the reconciler saw for one pending assignment.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    /// `None` when the run no longer exists.
    pub run: Option<LaneRunState>,
    /// The lane's work could not be proven stopped (a tool still running, an
    /// interrupt not acknowledged). Treated as uncertain, never as success.
    pub unsettled: bool,
}

/// Next status for an assignment given what was observed.
///
/// A specialist only *submits* a result; the reconciler turns `Submitted`
/// into `Completed`, `Uncertain` or `Failed` once the run has actually ended.
pub fn derive_assignment_status(
    current: AssignmentStatus,
    outcome: Option<Outcome>,
    has_result: bool,
    seen: Observation,
) -> AssignmentStatus {
    if !current.is_pending() {
        return current;
    }
    let ended = seen.run.map_or(true, LaneRunState::is_terminal);
    if ended {
        if seen.unsettled {
            return AssignmentStatus::Uncertain;
        }
        return match seen.run {
            Some(LaneRunState::Completed) if has_result => match outcome {
                Some(Outcome::Passed) => AssignmentStatus::Completed,
                Some(Outcome::Uncertain) => AssignmentStatus::Uncertain,
                _ => AssignmentStatus::Failed,
            },
            Some(LaneRunState::Cancelled) => AssignmentStatus::Cancelled,
            _ => AssignmentStatus::Failed,
        };
    }
    if current == AssignmentStatus::Submitted {
        return current;
    }
    match seen.run {
        Some(LaneRunState::WaitingInput) => AssignmentStatus::WaitingInput,
        Some(LaneRunState::WaitingTakeover) => AssignmentStatus::WaitingTakeover,
        _ => AssignmentStatus::Running,
    }
}

/// Where a batch of assignments stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchState {
    /// Nothing dispatched yet.
    Empty,
    /// Work is running and nothing needs attention: only a slow check-in is due.
    Healthy,
    /// Settled, or something failed / is waiting: the Chief should be woken.
    NeedsChief,
}

pub fn classify_batch(statuses: &[AssignmentStatus]) -> BatchState {
    if statuses.is_empty() {
        return BatchState::Empty;
    }
    let any_pending = statuses.iter().any(|s| s.is_pending());
    let any_attention = statuses.iter().any(|s| s.needs_attention());
    if any_pending && !any_attention {
        BatchState::Healthy
    } else {
        BatchState::NeedsChief
    }
}

/// A work item is cancelled when its room is archived or any assignment was
/// cancelled. `None` means it stays as it is.
pub fn work_status_after_sync(
    room_archived: bool,
    statuses: &[AssignmentStatus],
) -> Option<WorkStatus> {
    if room_archived || statuses.contains(&AssignmentStatus::Cancelled) {
        Some(WorkStatus::Cancelled)
    } else {
        None
    }
}

/// Idempotency key for a Chief wake. The same assignment states always give
/// the same nonce, so the Chief is never woken twice for one state.
pub fn wake_nonce(work_id: &str, assignments: &[(&str, AssignmentStatus)]) -> String {
    let listing: Vec<(&str, &str)> = assignments
        .iter()
        .map(|(id, status)| (*id, status_name(*status)))
        .collect();
    let json = serde_json::to_string(&listing).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    format!("group-wake:{work_id}:{digest:x}")
}

/// Idempotency key for a slow check-in: at most one per `window_ms` bucket.
pub fn checkin_nonce(work_id: &str, now_ms: u64, window_ms: u64) -> String {
    let bucket = now_ms / window_ms.max(1);
    format!("group-checkin:{work_id}:{bucket}")
}

fn status_name(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Queued => "queued",
        AssignmentStatus::Running => "running",
        AssignmentStatus::Submitted => "submitted",
        AssignmentStatus::WaitingInput => "waiting_input",
        AssignmentStatus::WaitingTakeover => "waiting_takeover",
        AssignmentStatus::Completed => "completed",
        AssignmentStatus::Failed => "failed",
        AssignmentStatus::Cancelled => "cancelled",
        AssignmentStatus::Uncertain => "uncertain",
        AssignmentStatus::Superseded => "superseded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AssignmentStatus as S;

    fn seen(run: Option<LaneRunState>, unsettled: bool) -> Observation {
        Observation { run, unsettled }
    }

    #[test]
    fn a_live_run_keeps_the_assignment_running() {
        let d = |cur, run| derive_assignment_status(cur, None, false, seen(Some(run), false));
        assert_eq!(d(S::Queued, LaneRunState::Running), S::Running);
        assert_eq!(d(S::Running, LaneRunState::WaitingInput), S::WaitingInput);
        assert_eq!(d(S::Running, LaneRunState::WaitingTakeover), S::WaitingTakeover);
        assert_eq!(d(S::WaitingInput, LaneRunState::Running), S::Running);
    }

    #[test]
    fn a_submitted_result_waits_for_the_run_to_end() {
        let live = seen(Some(LaneRunState::Running), false);
        assert_eq!(
            derive_assignment_status(S::Submitted, Some(Outcome::Passed), true, live),
            S::Submitted
        );
        let done = seen(Some(LaneRunState::Completed), false);
        assert_eq!(
            derive_assignment_status(S::Submitted, Some(Outcome::Passed), true, done),
            S::Completed
        );
    }

    #[test]
    fn ended_runs_resolve_by_outcome() {
        let done = seen(Some(LaneRunState::Completed), false);
        let d = |outcome, has_result| derive_assignment_status(S::Submitted, outcome, has_result, done);
        assert_eq!(d(Some(Outcome::Passed), true), S::Completed);
        assert_eq!(d(Some(Outcome::Uncertain), true), S::Uncertain);
        assert_eq!(d(Some(Outcome::Failed), true), S::Failed);
        assert_eq!(d(None, true), S::Failed);
        assert_eq!(d(Some(Outcome::Passed), false), S::Failed, "no submitted result");
    }

    #[test]
    fn cancelled_failed_and_vanished_runs() {
        let d = |run| derive_assignment_status(S::Running, None, false, seen(run, false));
        assert_eq!(d(Some(LaneRunState::Cancelled)), S::Cancelled);
        assert_eq!(d(Some(LaneRunState::Failed)), S::Failed);
        assert_eq!(d(None), S::Failed);
    }

    #[test]
    fn work_that_cannot_be_proven_stopped_is_uncertain_not_success() {
        for run in [Some(LaneRunState::Completed), Some(LaneRunState::Cancelled), Some(LaneRunState::Failed), None] {
            assert_eq!(
                derive_assignment_status(S::Submitted, Some(Outcome::Passed), true, seen(run, true)),
                S::Uncertain
            );
        }
    }

    #[test]
    fn settled_assignments_never_change() {
        for cur in [S::Completed, S::Failed, S::Cancelled, S::Uncertain, S::Superseded] {
            assert_eq!(derive_assignment_status(cur, None, false, seen(None, false)), cur);
        }
    }

    #[test]
    fn batches_classify_for_wake_versus_check_in() {
        assert_eq!(classify_batch(&[]), BatchState::Empty);
        assert_eq!(classify_batch(&[S::Running, S::Queued]), BatchState::Healthy);
        assert_eq!(classify_batch(&[S::Running, S::Completed]), BatchState::Healthy);
        assert_eq!(classify_batch(&[S::Completed, S::Completed]), BatchState::NeedsChief, "settled");
        assert_eq!(classify_batch(&[S::Running, S::Failed]), BatchState::NeedsChief);
        assert_eq!(classify_batch(&[S::Running, S::Uncertain]), BatchState::NeedsChief);
        assert_eq!(classify_batch(&[S::Running, S::WaitingInput]), BatchState::NeedsChief);
    }

    #[test]
    fn archived_rooms_and_cancelled_assignments_cancel_the_work() {
        assert_eq!(work_status_after_sync(true, &[S::Running]), Some(WorkStatus::Cancelled));
        assert_eq!(work_status_after_sync(false, &[S::Running, S::Cancelled]), Some(WorkStatus::Cancelled));
        assert_eq!(work_status_after_sync(false, &[S::Running, S::Failed]), None);
    }

    #[test]
    fn wake_nonce_is_stable_for_a_state_and_changes_with_it() {
        let a = wake_nonce("w1", &[("a1", S::Running), ("a2", S::Completed)]);
        assert_eq!(a, wake_nonce("w1", &[("a1", S::Running), ("a2", S::Completed)]));
        assert_ne!(a, wake_nonce("w1", &[("a1", S::Completed), ("a2", S::Completed)]));
        assert_ne!(a, wake_nonce("w2", &[("a1", S::Running), ("a2", S::Completed)]));
        assert!(a.starts_with("group-wake:w1:"));
    }

    #[test]
    fn check_in_nonce_changes_only_across_windows() {
        let w = 180_000;
        assert_eq!(checkin_nonce("w1", 1_000, w), checkin_nonce("w1", 179_999, w));
        assert_ne!(checkin_nonce("w1", 179_999, w), checkin_nonce("w1", 180_000, w));
        assert_eq!(checkin_nonce("w1", 5, 0), "group-checkin:w1:5", "zero window does not divide by zero");
    }
}
