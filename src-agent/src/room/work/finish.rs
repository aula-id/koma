//! The rule for when an engagement may be closed.

use super::model::Assignment;
use crate::room::types::{AssignmentKind, AssignmentStatus, Outcome, Reference};

fn is_result_key(key: &str) -> bool {
    ["claims/", "claim/", "evidence/", "results/"].iter().any(|p| key.starts_with(p))
}

/// What verification must cover for a production result: its claims, evidence
/// and results. A result that cites nothing of those must be covered in full.
pub fn required_review_references(refs: &[Reference]) -> Vec<Reference> {
    let results: Vec<Reference> = refs.iter().filter(|r| is_result_key(&r.key)).cloned().collect();
    if results.is_empty() {
        refs.to_vec()
    } else {
        results
    }
}

/// `Err` carries the reason the Chief cannot finish yet.
pub fn assert_finishable(assignments: &[Assignment]) -> Result<(), String> {
    let live: Vec<&Assignment> = assignments
        .iter()
        .filter(|a| {
            a.status != AssignmentStatus::Superseded
                && !(a.kind == AssignmentKind::Investigation && !a.status.is_pending())
        })
        .collect();
    if live.is_empty()
        || live
            .iter()
            .any(|a| a.status != AssignmentStatus::Completed || a.outcome != Some(Outcome::Passed))
    {
        return Err("Required work remains unresolved".to_string());
    }

    let productions: Vec<&&Assignment> = live.iter().filter(|a| a.kind.is_production()).collect();
    let all_verified = !productions.is_empty()
        && productions.iter().all(|p| {
            required_review_references(&p.references).iter().all(|need| {
                live.iter().any(|v| {
                    v.kind == AssignmentKind::Verification
                        && v.reviews_id.is_some()
                        && v.references.iter().any(|have| have.key == need.key && have.revision >= need.revision)
                })
            })
        });
    if !all_verified {
        return Err("Independent final verification required for every probe result".to_string());
    }

    let finding_published = live.iter().any(|a| {
        a.kind == AssignmentKind::Verification
            && a.references.iter().any(|r| r.key.starts_with("findings/") || r.key.starts_with("finding/"))
    });
    if finding_published {
        let has_report = live.iter().any(|a| {
            a.kind == AssignmentKind::Report
                && a.status == AssignmentStatus::Completed
                && a.outcome == Some(Outcome::Passed)
                && a.references.iter().any(|r| r.key.starts_with("reports/"))
        });
        if !has_report {
            return Err("Verified findings require a completed Lab Reporter dual pack under reports/ (technical per finding + manager executive) before finish".to_string());
        }
    }
    Ok(())
}
