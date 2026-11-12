//! Goals, assignments and the coordinator tools.
//!
//! Ported from Orpheus `group-work.ts` and `group-review-queue.ts` (the parts
//! a tool call needs). The reconciler that keeps this in sync with running
//! lanes, and the queue's dispatch half, come with the lane runner.

mod coordinator;
mod effects;
mod finish;
mod goals;
mod model;
mod queue;
mod reconcile;
mod store;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use coordinator::{Caller, Coordinator, ToolOutcome};
#[allow(unused_imports)]
pub use effects::Effect;
#[allow(unused_imports)]
pub use finish::assert_finishable;
#[allow(unused_imports)]
pub use goals::{append_goal, open_trajectory_goals};
#[allow(unused_imports)]
pub use model::{Assignment, ReviewItem, ReviewStatus, Work};
#[allow(unused_imports)]
pub use queue::{complete_review_queue, enqueue_claim};
#[allow(unused_imports)]
pub use reconcile::{ReconcileOutcome, Reconciler, CHECKIN_WINDOW_MS};
#[allow(unused_imports)]
pub use store::WorkStore;

#[cfg(test)]
pub(crate) fn tests_nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
