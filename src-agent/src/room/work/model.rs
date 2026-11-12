//! Rows the coordinator keeps in `room.sqlite`: goals, assignments and
//! confirmation-queue items.

use serde::{Deserialize, Serialize};

use crate::room::types::{AssignmentKind, AssignmentStatus, Outcome, Reference, WorkStatus};

/// One task given to one specialist avatar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: String,
    pub work_id: String,
    pub dispatch_key: String,
    /// The avatar doing the work. Its role is looked up in the registry.
    pub avatar: String,
    pub kind: AssignmentKind,
    pub prompt: String,
    pub reviews_id: Option<String>,
    pub status: AssignmentStatus,
    pub result: Option<String>,
    pub outcome: Option<Outcome>,
    pub references: Vec<Reference>,
}

/// One goal (engagement) and its assignments, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Work {
    pub id: String,
    pub key: String,
    pub prompt: String,
    pub status: WorkStatus,
    pub report: Option<String>,
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Queued,
    Assigned,
    /// The reviewer sent the claim back; the originating Red is steered.
    Returned,
    Failed,
    /// Reviewed to a verdict other than `returned`.
    Completed,
    Superseded,
}

impl ReviewStatus {
    /// Still blocks `group_finish`.
    pub fn blocks_finish(self) -> bool {
        matches!(
            self,
            ReviewStatus::Queued | ReviewStatus::Assigned | ReviewStatus::Returned | ReviewStatus::Failed
        )
    }
}

/// A claim snapshot waiting for, or going through, independent review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    pub work_id: String,
    pub source_assignment_id: String,
    pub claim_key: String,
    pub claim_revision: u64,
    pub snapshot_hash: String,
    pub references: Vec<Reference>,
    pub status: ReviewStatus,
    pub review_assignment_id: Option<String>,
    pub verdict_key: Option<String>,
    pub feedback_sent: bool,
}
