//! The reconciler: a periodic pass that reads each lane's state, moves its
//! assignment to match, and wakes the Chief when a batch settles or needs
//! attention.
//!
//! This is the half of the Chief's rhythm that runs without the Chief: it ends
//! its turn after delegating, and this brings it back. Waking is idempotent —
//! each distinct batch state wakes the Chief at most once (a content nonce), and
//! a healthily running batch earns only a slow check-in.
//!
//! The automatic review-queue and reporter dispatch (so the Chief need not
//! delegate every verification by hand) is a later step; this one only syncs
//! and wakes.


use rusqlite::Connection;

use super::effects::Effect;
use super::model::Work;
use super::store::{self, WorkStore, R};
use crate::room::transition::{
    checkin_nonce, classify_batch, derive_assignment_status, wake_nonce, work_status_after_sync,
    BatchState, Observation,
};
use crate::room::types::AssignmentStatus;

/// How often a healthily running batch earns a Chief check-in.
pub const CHECKIN_WINDOW_MS: u64 = 3 * 60 * 1000;

/// What one reconciler pass decided. The actor turns `wakes` and `checkins`
/// into Chief inbox items and applies nothing else (the statuses are already
/// persisted).
#[derive(Debug, Default, PartialEq)]
pub struct ReconcileOutcome {
    /// Work ids whose batch settled or needs attention.
    pub wakes: Vec<String>,
    /// Work ids running healthily, due for a check-in.
    pub checkins: Vec<String>,
    /// Assignment ids whose status moved this pass, and to what.
    pub status_changes: Vec<(String, AssignmentStatus)>,
    /// Reviews, feedback and report hand-overs the dispatch hook produced.
    pub effects: Vec<Effect>,
}

/// Wakes and check-ins, each at most once. What already fired is recorded in
/// the store, scoped to its work, so the same state never wakes the Chief twice
/// across passes or restarts and nothing accumulates in memory.
pub struct Reconciler {
    window_ms: u64,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl Reconciler {
    pub fn new() -> Self {
        Self { window_ms: CHECKIN_WINDOW_MS }
    }

    /// A reconciler with a custom check-in window, for tests.
    pub fn with_window(window_ms: u64) -> Self {
        Self { window_ms }
    }

    /// Run one pass. `observe` reports the lane state behind each pending
    /// assignment id; `now_ms` is the wall clock for check-in bucketing.
    pub fn tick(
        &mut self,
        store: &WorkStore,
        observe: impl Fn(&str) -> Observation,
        now_ms: u64,
    ) -> R<ReconcileOutcome> {
        self.tick_with(store, observe, now_ms, |_, _| Ok(Vec::new()))
    }

    /// Like [`Reconciler::tick`], with a hook that runs after the statuses are
    /// synced and before the Chief is woken. The hook starts reviews and report
    /// hand-overs, so a probe that just finished has its review queued before the
    /// Chief is told the batch settled.
    pub fn tick_with(
        &mut self,
        store: &WorkStore,
        observe: impl Fn(&str) -> Observation,
        now_ms: u64,
        dispatch: impl Fn(&Connection, &Work) -> R<Vec<Effect>>,
    ) -> R<ReconcileOutcome> {
        store.with_tx(|conn| self.run(conn, &observe, now_ms, &dispatch))
    }

    fn run(
        &mut self,
        conn: &Connection,
        observe: &impl Fn(&str) -> Observation,
        now_ms: u64,
        dispatch: &impl Fn(&Connection, &Work) -> R<Vec<Effect>>,
    ) -> R<ReconcileOutcome> {
        let mut out = ReconcileOutcome::default();
        for work_id in store::all_active_work_ids(conn)? {
            let Some(work) = store::work_by_id(conn, &work_id)? else {
                continue;
            };
            for a in &work.assignments {
                if !a.status.is_pending() {
                    continue;
                }
                let next = derive_assignment_status(a.status, a.outcome, a.result.is_some(), observe(&a.id));
                if next != a.status {
                    store::set_assignment_status(conn, &a.id, next)?;
                    out.status_changes.push((a.id.clone(), next));
                }
            }
            // Reload the statuses the writes produced before classifying.
            let statuses: Vec<AssignmentStatus> =
                store::assignments_of(conn, &work_id)?.iter().map(|a| a.status).collect();
            if let Some(ws) = work_status_after_sync(false, &statuses) {
                store::set_work_status(conn, &work_id, ws, None)?;
                continue;
            }
            // Start reviews and hand-overs now, so the batch is classified with them.
            if let Some(fresh) = store::work_by_id(conn, &work_id)? {
                out.effects.extend(dispatch(conn, &fresh)?);
            }
            let assignments = store::assignments_of(conn, &work_id)?;
            let statuses: Vec<AssignmentStatus> = assignments.iter().map(|a| a.status).collect();
            match classify_batch(&statuses) {
                BatchState::Empty => {}
                BatchState::Healthy => {
                    let nonce = checkin_nonce(&work_id, now_ms, self.window_ms);
                    if store::insert_fired_nonce(conn, &work_id, &nonce)? {
                        out.checkins.push(work_id.clone());
                    }
                }
                BatchState::NeedsChief => {
                    let pairs: Vec<(&str, AssignmentStatus)> =
                        assignments.iter().map(|a| (a.id.as_str(), a.status)).collect();
                    if store::insert_fired_nonce(conn, &work_id, &wake_nonce(&work_id, &pairs))? {
                        out.wakes.push(work_id.clone());
                    }
                }
            }
        }
        Ok(out)
    }
}
