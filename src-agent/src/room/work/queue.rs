//! Confirmation queue: claims published by Red wait here for independent
//! review. This file holds enqueueing a claim and the completion half that runs
//! when the reviewer submits; dispatching reviews and returning feedback belongs
//! to the reconciler.

use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::model::{Assignment, ReviewItem, ReviewStatus};
use super::store::{self, new_id, R};
use crate::room::shelf::Shelf;
use crate::room::types::{AssignmentStatus, Outcome, Reference};

fn body(content: &str) -> serde_json::Map<String, Value> {
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

fn snapshot_hash(snapshot: &[Reference]) -> String {
    let json = serde_json::to_string(snapshot).unwrap_or_default();
    format!("{:x}", Sha256::digest(json.as_bytes()))
}

/// Queue the current snapshot of a claim and its evidence for review.
/// `Ok(Some(reason))` means it was skipped and why, so the publishing lane can be told.
pub fn enqueue_claim(
    conn: &Connection,
    shelf: &Shelf,
    source: &Assignment,
    claim_key: &str,
) -> R<Option<String>> {
    let shelf_err = |e: crate::room::shelf::ShelfError| e.0;
    let Some(claim) = shelf.peek(claim_key).map_err(shelf_err)? else {
        return Ok(Some("claim row no longer exists on the shelf".to_string()));
    };
    let claim_body = body(claim.content.as_deref().unwrap_or(""));
    let has_poc = claim_body.get("poc").and_then(Value::as_str).is_some_and(|p| !p.trim().is_empty());
    if claim_body.get("kind").and_then(Value::as_str) != Some("claim") || !has_poc {
        return Ok(Some("claim has no poc — put_claim with a filled poc before Reviewer can take it".to_string()));
    }
    let declared: Vec<String> = claim_body
        .get("evidence_keys")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let evidence: Vec<_> = shelf
        .scan("evidence/")
        .map_err(shelf_err)?
        .into_iter()
        .filter(|e| body(e.content.as_deref().unwrap_or("")).get("claim_key").and_then(Value::as_str) == Some(claim_key))
        .collect();
    if evidence.is_empty() {
        return Ok(Some("no evidence supports this claim — put_evidence with claim_key pointing at it first".to_string()));
    }
    if declared.iter().any(|k| !evidence.iter().any(|e| &e.key == k)) {
        return Ok(Some("a declared evidence_keys entry is missing on the shelf — put_evidence for it first".to_string()));
    }

    let mut snapshot = vec![Reference { key: claim.key.clone(), revision: claim.revision.max(0) as u64 }];
    snapshot.extend(evidence.iter().map(|e| Reference { key: e.key.clone(), revision: e.revision.max(0) as u64 }));
    snapshot.sort_by(|a, b| a.key.cmp(&b.key));
    let hash = snapshot_hash(&snapshot);

    if store::find_item(conn, &source.work_id, claim_key, &hash)?.is_some() {
        return Ok(Some(format!("already queued for review (snapshot {claim_key} revision {})", claim.revision)));
    }
    store::retire_items(
        conn,
        &source.work_id,
        claim_key,
        &[ReviewStatus::Queued, ReviewStatus::Returned, ReviewStatus::Failed],
        ReviewStatus::Superseded,
    )?;
    // A review that is already running over exactly this snapshot takes it.
    let assigned = store::assignments_of(conn, &source.work_id)?.into_iter().find(|a| {
        a.kind == crate::room::types::AssignmentKind::Verification
            && a.status.is_pending()
            && snapshot.iter().all(|s| a.references.iter().any(|o| o.key == s.key && o.revision == s.revision))
    });
    store::insert_item(
        conn,
        &ReviewItem {
            id: new_id(),
            work_id: source.work_id.clone(),
            source_assignment_id: source.id.clone(),
            claim_key: claim_key.to_string(),
            claim_revision: claim.revision.max(0) as u64,
            snapshot_hash: hash,
            references: snapshot,
            status: if assigned.is_some() { ReviewStatus::Assigned } else { ReviewStatus::Queued },
            review_assignment_id: assigned.map(|a| a.id),
            verdict_key: None,
            feedback_sent: false,
        },
    )?;
    Ok(None)
}

/// Runs when a reviewer submits a verification. Returns `true` when the
/// evidence moved during review and this snapshot was superseded (the caller
/// tells the reviewer to end its turn). Errors name what is missing.
pub fn complete_review_queue(
    conn: &Connection,
    shelf: &Shelf,
    assignment: &Assignment,
    outcome: Outcome,
    references: &[Reference],
) -> R<bool> {
    let shelf_err = |e: crate::room::shelf::ShelfError| e.0;
    let items = store::items_for_review(conn, &assignment.id, ReviewStatus::Assigned)?;
    if items.is_empty() {
        return Ok(false);
    }
    let moved = |item: &ReviewItem| -> R<bool> {
        for r in &item.references {
            match shelf.peek(&r.key).map_err(shelf_err)? {
                Some(row) if row.revision.max(0) as u64 == r.revision => {}
                _ => return Ok(true),
            }
        }
        Ok(false)
    };
    let mut stale = false;
    for item in &items {
        stale |= moved(item)?;
    }
    if stale {
        let assignments = store::assignments_of(conn, &assignment.work_id)?;
        for item in &items {
            let changed = moved(item)?;
            if changed {
                store::update_item(conn, &item.id, ReviewStatus::Superseded, item.review_assignment_id.as_deref(), None)?;
                if let Some(source) = assignments.iter().find(|a| a.id == item.source_assignment_id) {
                    enqueue_claim(conn, shelf, source, &item.claim_key)?;
                }
            } else {
                store::update_item(conn, &item.id, ReviewStatus::Queued, None, None)?;
            }
        }
        store::set_assignment_status(conn, &assignment.id, AssignmentStatus::Superseded)?;
        return Ok(true);
    }
    if outcome != Outcome::Passed {
        for item in &items {
            store::update_item(conn, &item.id, ReviewStatus::Failed, item.review_assignment_id.as_deref(), None)?;
        }
        return Ok(false);
    }

    let mut outputs = Vec::new();
    for r in references {
        if let Some(row) = shelf.peek(&r.key).map_err(shelf_err)? {
            outputs.push(row);
        }
    }
    for item in &items {
        let verdict = outputs.iter().find(|row| {
            let b = body(row.content.as_deref().unwrap_or(""));
            row.key.starts_with("verdicts/")
                && b.get("claim_key").and_then(Value::as_str) == Some(item.claim_key.as_str())
                && b.get("claim_revision").and_then(Value::as_u64) == Some(item.claim_revision)
        });
        let Some(verdict) = verdict else {
            return Err(format!(
                "Publish a verdict for {} revision {} before completing this review",
                item.claim_key, item.claim_revision
            ));
        };
        let vb = body(verdict.content.as_deref().unwrap_or(""));
        let kind = vb.get("verdict").and_then(Value::as_str).unwrap_or("");
        if !["confirmed", "returned", "dismissed", "discarded"].contains(&kind) {
            return Err(format!("Invalid verdict for {}", item.claim_key));
        }
        if kind == "confirmed" {
            let has_finding = outputs.iter().any(|row| {
                let b = body(row.content.as_deref().unwrap_or(""));
                row.key.starts_with("findings/")
                    && b.get("claim_key").and_then(Value::as_str) == Some(item.claim_key.as_str())
                    && b.get("verdict_key").and_then(Value::as_str) == Some(verdict.key.as_str())
            });
            if !has_finding {
                return Err(format!("Confirmed claim {} requires a published finding", item.claim_key));
            }
        }
        let status = if kind == "returned" { ReviewStatus::Returned } else { ReviewStatus::Completed };
        store::update_item(conn, &item.id, status, item.review_assignment_id.as_deref(), Some(&verdict.key))?;
    }
    Ok(false)
}
