//! The queue's dispatch half: turning queued claims into reviews, sending a
//! returned claim back to the Red that made it, and handing finished findings
//! to the Reporter. Together with `queue.rs` this is the whole confirmation
//! loop, so the Chief does not delegate every verification by hand.
//!
//! Ported from Orpheus `reconcileReviewQueue` and `reconcileReportDispatch`.
//! Every function here is called inside the reconciler's transaction and
//! returns [`Effect`]s for the actor to apply once it commits.

use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::effects::Effect;
use super::model::{Assignment, ReviewItem, ReviewStatus, Work};
use super::queue::enqueue_claim;
use super::store::{self, new_id, R};
use crate::room::oracle;
use crate::room::registry::Registry;
use crate::room::shelf::Shelf;
use crate::room::types::{AssignmentKind, AssignmentStatus, Outcome, Reference};

/// Claims reviewed per verification, and the most references one review may cover.
const MAX_BATCH_ITEMS: usize = 8;
const MAX_REVIEW_REFS: usize = 32;

fn hash(value: &impl serde::Serialize) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_string(value).unwrap_or_default().as_bytes()))
}

/// The first avatar of `role` with nothing pending in this work and an idle lane.
fn idle_avatar(registry: &Registry, assignments: &[Assignment], role: &str, lane_idle: &dyn Fn(&str) -> bool) -> Option<String> {
    registry
        .avatars_of(role)
        .into_iter()
        .map(|a| a.id.as_str())
        .find(|id| !assignments.iter().any(|a| a.avatar == *id && a.status.is_pending()) && lane_idle(id))
        .map(str::to_string)
}

/// Create a queued assignment and the effects that start it.
#[allow(clippy::too_many_arguments)]
fn assign(
    conn: &Connection,
    registry: &Registry,
    work: &Work,
    assignments: &mut Vec<Assignment>,
    avatar: &str,
    kind: AssignmentKind,
    prompt: String,
    dispatch_key: String,
    references: Vec<Reference>,
    reviews_id: Option<String>,
) -> R<Vec<Effect>> {
    let assignment = Assignment {
        id: new_id(),
        work_id: work.id.clone(),
        dispatch_key: dispatch_key.clone(),
        avatar: avatar.to_string(),
        kind,
        prompt: prompt.clone(),
        reviews_id,
        status: AssignmentStatus::Queued,
        result: None,
        outcome: None,
        references,
    };
    store::insert_assignment(conn, &assignment)?;
    assignments.push(assignment.clone());
    Ok(vec![
        Effect::Visible {
            author: "chief".to_string(),
            text: format!("Assignment for {}: {prompt}", registry.display_of(avatar)),
            nonce: format!("group-dispatch:{}:{}:{}", work.id, dispatch_key, avatar),
            work_id: work.id.clone(),
        },
        Effect::StartLane { assignment, goal: work.prompt.clone() },
    ])
}

fn verdict_note(shelf: &Shelf, verdict_key: Option<&str>) -> String {
    let fallback = "More evidence is required".to_string();
    let Some(key) = verdict_key else { return fallback };
    let Ok(Some(row)) = shelf.peek(key) else { return fallback };
    match serde_json::from_str::<Value>(row.content.as_deref().unwrap_or("")) {
        Ok(body) => body.get("note").and_then(Value::as_str).map(str::to_string).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

/// Queue published claims, retire reviews that died, return feedback, and hand
/// the next batch of queued claims to an idle reviewer.
pub fn review_queue(
    conn: &Connection,
    shelf: &Shelf,
    registry: &Registry,
    work: &Work,
    lane_idle: &dyn Fn(&str) -> bool,
) -> R<Vec<Effect>> {
    let mut effects = Vec::new();
    let mut assignments = store::assignments_of(conn, &work.id)?;

    // 1. Probes that submitted claims put them in the queue (rescans only re-queue).
    for source in assignments.clone().iter().filter(|a| a.kind == AssignmentKind::Probe && a.status != AssignmentStatus::Superseded) {
        for r in source.references.iter().filter(|r| r.key.starts_with("claims/")) {
            enqueue_claim(conn, shelf, source, &r.key)?;
        }
    }

    // 2. A review that failed, was cancelled or was replaced takes its items with it.
    for item in store::review_items(conn, &work.id)?.iter().filter(|i| i.status == ReviewStatus::Assigned) {
        let review = assignments.iter().find(|a| Some(&a.id) == item.review_assignment_id.as_ref());
        if let Some(review) = review {
            if matches!(
                review.status,
                AssignmentStatus::Failed | AssignmentStatus::Uncertain | AssignmentStatus::Cancelled | AssignmentStatus::Superseded
            ) {
                let next = if review.status == AssignmentStatus::Superseded { ReviewStatus::Superseded } else { ReviewStatus::Failed };
                store::update_item(conn, &item.id, next, item.review_assignment_id.as_deref(), None)?;
            }
        }
    }

    // 2b. A claim whose cited code a program could not find goes straight back to its Red, unreviewed.
    for item in store::review_items(conn, &work.id)?.iter().filter(|i| i.status == ReviewStatus::Queued) {
        if oracle::stored(shelf, &item.claim_key, item.claim_revision).is_some_and(|s| s.verdict == Some(oracle::Verdict::Refuted)) {
            store::update_item(conn, &item.id, ReviewStatus::Returned, None, None)?;
        }
    }

    // 3. A returned claim goes back to the Red that made it, once.
    for item in store::review_items(conn, &work.id)?.iter().filter(|i| i.status == ReviewStatus::Returned && !i.feedback_sent) {
        let Some(source) = assignments.iter().find(|a| a.id == item.source_assignment_id).cloned() else {
            continue;
        };
        let revision = item.claim_revision.to_string();
        let note = match (item.verdict_key.as_deref(), oracle::stored(shelf, &item.claim_key, item.claim_revision)) {
            (None, Some(found)) if found.verdict == Some(oracle::Verdict::Refuted) => oracle::feedback(&found.lines),
            _ => verdict_note(shelf, item.verdict_key.as_deref()),
        };
        let text = registry
            .render("returned", &[("claim", &item.claim_key), ("revision", &revision), ("note", &note)])
            .map_err(|e| e.to_string())?;
        let busy = assignments.iter().any(|a| a.avatar == source.avatar && a.status.is_pending());
        if busy {
            effects.push(Effect::Steer { avatar: source.avatar.clone(), text });
        } else if lane_idle(&source.avatar) {
            let prompt = format!("{text}\n\nOriginal goal: {}", work.prompt);
            effects.extend(assign(
                conn,
                registry,
                work,
                &mut assignments,
                &source.avatar,
                AssignmentKind::Probe,
                prompt,
                format!("review-return:{}", item.id),
                Vec::new(),
                None,
            )?);
        } else {
            continue; // the lane is not provably idle; try again next pass
        }
        store::mark_feedback_sent(conn, &item.id)?;
    }

    // 4. Hand the next batch of queued claims to an idle reviewer.
    let Some(reviewer) = idle_avatar(registry, &assignments, "reviewer", lane_idle) else {
        return Ok(effects);
    };
    // A claim whose proof check has not answered yet waits; the others go ahead.
    let queued: Vec<ReviewItem> = store::review_items(conn, &work.id)?
        .into_iter()
        .filter(|i| i.status == ReviewStatus::Queued)
        .filter(|i| !oracle::stored(shelf, &i.claim_key, i.claim_revision).is_some_and(|s| s.pending))
        .collect();
    let mut batch: Vec<&ReviewItem> = Vec::new();
    let mut snapshot: Vec<Reference> = Vec::new();
    for item in queued.iter().take(MAX_BATCH_ITEMS) {
        let mut combined = snapshot.clone();
        for r in &item.references {
            match combined.iter_mut().find(|k| k.key == r.key) {
                Some(slot) => *slot = r.clone(),
                None => combined.push(r.clone()),
            }
        }
        if combined.len() > MAX_REVIEW_REFS {
            break;
        }
        batch.push(item);
        snapshot = combined;
    }
    if batch.is_empty() {
        return Ok(effects);
    }
    snapshot.sort_by(|a, b| a.key.cmp(&b.key));
    for old in assignments.iter().filter(|a| {
        a.kind == AssignmentKind::Investigation && a.avatar == reviewer && !a.status.is_pending() && a.status != AssignmentStatus::Superseded
    }) {
        store::set_assignment_status(conn, &old.id, AssignmentStatus::Superseded)?;
    }
    let listing = json!(batch
        .iter()
        .map(|i| {
            let checked = oracle::stored(shelf, &i.claim_key, i.claim_revision).and_then(|s| s.verdict).map_or("unproven", oracle::Verdict::as_str);
            json!({ "claim": i.claim_key, "revision": i.claim_revision, "code_check": checked, "references": i.references })
        })
        .collect::<Vec<_>>())
    .to_string();
    let prompt = registry
        .render("verify", &[("goal", &work.prompt), ("items", &listing)])
        .map_err(|e| e.to_string())?;
    let ids: Vec<&str> = batch.iter().map(|i| i.id.as_str()).collect();
    let before = assignments.len();
    effects.extend(assign(
        conn,
        registry,
        work,
        &mut assignments,
        &reviewer,
        AssignmentKind::Verification,
        prompt,
        format!("review-queue:{}", hash(&ids)),
        snapshot,
        Some(batch[0].source_assignment_id.clone()),
    )?);
    let review_id = assignments[before].id.clone();
    for item in &batch {
        store::update_item(conn, &item.id, ReviewStatus::Assigned, Some(&review_id), None)?;
    }
    Ok(effects)
}

/// When findings exist after a passed verification and the Reporter is idle,
/// hand over the report pack: a detailed technical report per finding, then a
/// manager executive summary. Repeats only while the pack has gaps.
pub fn report_dispatch(
    conn: &Connection,
    shelf: &Shelf,
    registry: &Registry,
    work: &Work,
    lane_idle: &dyn Fn(&str) -> bool,
) -> R<Vec<Effect>> {
    let mut assignments = store::assignments_of(conn, &work.id)?;
    let verifications: Vec<&Assignment> = assignments
        .iter()
        .filter(|a| a.kind == AssignmentKind::Verification && a.status == AssignmentStatus::Completed && a.outcome == Some(Outcome::Passed))
        .collect();
    if verifications.is_empty() {
        return Ok(Vec::new());
    }
    if assignments.iter().any(|a| a.kind == AssignmentKind::Report && a.status.is_pending()) {
        return Ok(Vec::new());
    }
    let Some(reporter) = idle_avatar(registry, &assignments, "reporter", lane_idle) else {
        return Ok(Vec::new());
    };
    let findings = shelf.scan("findings/").map_err(|e| e.0)?;
    if findings.is_empty() {
        return Ok(Vec::new());
    }
    // Only a gap in the pack triggers a hand-over; a complete pack ends here.
    let Err(gaps) = shelf.assert_reporter_pack_complete() else {
        return Ok(Vec::new());
    };

    let reported: Vec<&str> = assignments
        .iter()
        .filter(|a| a.kind == AssignmentKind::Report && a.status == AssignmentStatus::Completed && a.outcome == Some(Outcome::Passed))
        .filter_map(|a| a.reviews_id.as_deref())
        .collect();
    let verified = verifications
        .iter()
        .rev()
        .find(|a| !reported.contains(&a.id.as_str()))
        .or_else(|| verifications.last())
        .copied()
        .cloned()
        .ok_or("no verification to report on")?;

    let mut snapshot: Vec<Reference> = verified.references.clone();
    for f in &findings {
        let r = Reference { key: f.key.clone(), revision: f.revision.max(0) as u64 };
        match snapshot.iter_mut().find(|k| k.key == r.key) {
            Some(slot) => *slot = r,
            None => snapshot.push(r),
        }
    }
    snapshot.sort_by(|a, b| a.key.cmp(&b.key));
    let listing = findings.iter().map(|f| f.key.as_str()).collect::<Vec<_>>().join(", ");
    let prompt = registry
        .render("report", &[("goal", &work.prompt), ("gaps", &gaps.0), ("findings", &listing)])
        .map_err(|e| e.to_string())?;
    let key = format!("report-queue:{}", hash(&(&snapshot, &gaps.0)));
    assign(conn, registry, work, &mut assignments, &reporter, AssignmentKind::Report, prompt, key, snapshot, Some(verified.id))
}
