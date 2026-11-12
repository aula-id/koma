//! Ported from Orpheus `group-work.test.ts`, adapted to roles and probes.

use std::sync::Arc;

use serde_json::{json, Value};

use super::coordinator::{Caller, Coordinator, ToolOutcome};
use super::effects::Effect;
use super::finish::assert_finishable;
use super::goals::{append_goal, open_trajectory_goals};
use super::model::{Assignment, ReviewStatus, Work};
use super::queue::{complete_review_queue, enqueue_claim};
use super::store::{self, WorkStore};
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use crate::room::types::{AssignmentKind, AssignmentStatus, Outcome, Reference, WorkStatus};

struct Fx {
    co: Coordinator,
    shelf: Arc<Shelf>,
}

fn fx() -> Fx {
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let co = Coordinator::new(WorkStore::open_in_memory().expect("store"), Arc::clone(&shelf), registry);
    Fx { co, shelf }
}

fn r(key: &str, revision: u64) -> Reference {
    Reference { key: key.into(), revision }
}

fn refs(list: &[Reference]) -> Value {
    serde_json::to_value(list).expect("refs")
}

impl Fx {
    fn chief(&self, tool: &str, args: Value) -> ToolOutcome {
        self.co.call(&Caller::Chief, tool, &args)
    }

    fn lane(&self, avatar: &str, id: &str, tool: &str, args: Value) -> ToolOutcome {
        self.co.call(&Caller::Lane { avatar: avatar.into(), assignment_id: id.into() }, tool, &args)
    }

    fn work(&self) -> Work {
        self.co
            .store()
            .with_tx(|c| Ok(store::latest_work(c)?.expect("a work item")))
            .expect("work")
    }

    fn assignment(&self, i: usize) -> Assignment {
        self.work().assignments[i].clone()
    }

    /// What the reconciler does once a lane ended: submitted -> completed.
    fn settle(&self, id: &str, status: AssignmentStatus) {
        self.co.store().with_tx(|c| store::set_assignment_status(c, id, status)).expect("settle");
    }

    fn delegate_probe(&self, goal: &str, dispatch: &str) -> ToolOutcome {
        self.chief(
            "group_delegate",
            json!({ "goal_key": "g1", "goal": goal, "dispatch_key": dispatch,
                    "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "Map the login" }] }),
        )
    }

    fn put(&self, tool: &str, args: Value) -> Value {
        self.shelf.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {}", e.0))
    }

    fn publish_claim(&self) {
        self.put("put_claim", json!({ "key": "claims/a", "vulnclass": "cors", "name": "CORS", "severity": "high",
            "summary": "s", "host": "h", "poc": "curl", "expected_revision": 0 }));
        self.put("put_evidence", json!({ "key": "evidence/e", "claim_key": "claims/a", "evidence_kind": "http",
            "body": "b", "expected_revision": 0 }));
        // evidence linked itself onto the claim, so the claim is at revision 2
    }
}

fn ok(out: &ToolOutcome) -> &Value {
    assert!(out.value.get("error").is_none(), "unexpected error: {}", out.value);
    &out.value
}

fn error(out: &ToolOutcome) -> String {
    out.value["error"].as_str().unwrap_or_else(|| panic!("expected an error, got {}", out.value)).to_string()
}

fn asg(kind: AssignmentKind, status: AssignmentStatus, outcome: Option<Outcome>, reviews: Option<&str>, id: &str, refs: &[Reference]) -> Assignment {
    Assignment {
        id: id.into(), work_id: "w".into(), dispatch_key: "d".into(), avatar: "x".into(), kind,
        prompt: "p".into(), reviews_id: reviews.map(str::to_string), status, result: None, outcome,
        references: refs.to_vec(),
    }
}

// ---- goals ----

#[test]
fn keeps_the_original_goal_and_treats_only_exhausted_trajectory_goals_as_finished() {
    assert_eq!(append_goal("Scan the dashboard", "Scan the dashboard"), "Scan the dashboard");
    assert_eq!(append_goal("Scan the dashboard", "Round 2: CORS"), "Scan the dashboard\n\nAdded goal:\nRound 2: CORS");
    assert_eq!(append_goal("x", "   "), "x");
    let huge = "y".repeat(20_001);
    assert_eq!(append_goal("x", &huge), "x", "an overflowing addition is dropped");

    let graph = json!({ "nodes": [
        { "id": "g1", "kind": "goal", "label": "CORS on login", "status": "open" },
        { "id": "g2", "kind": "goal", "label": "Rate limits", "status": "Met" },
        { "id": "g3", "kind": "goal", "label": "Admin surface", "status": "Budget exhausted" },
        { "id": "g4", "kind": "goal", "label": "Injection canaries" },
        { "id": "i1", "kind": "intent", "label": "Map login", "status": "Done" }
    ] })
    .to_string();
    assert_eq!(open_trajectory_goals(&[graph]), ["CORS on login", "Injection canaries"]);
    assert!(open_trajectory_goals(&["not json".to_string(), json!({ "nodes": [] }).to_string()]).is_empty());
}

// ---- finish rule ----

#[test]
fn requires_final_verification_of_exactly_the_produced_revisions() {
    use AssignmentKind::*;
    use AssignmentStatus::*;
    let probe = asg(Probe, Completed, Some(Outcome::Passed), None, "probe", &[r("claims/task/cors", 1)]);
    let investigation = asg(Investigation, Completed, Some(Outcome::Passed), None, "inv", &[]);
    let review = asg(Verification, Completed, Some(Outcome::Passed), Some("probe"), "review", &[r("claims/task/cors", 1)]);
    assert!(assert_finishable(&[probe.clone(), investigation.clone(), review.clone()]).is_ok());
    let err = assert_finishable(&[probe.clone(), investigation.clone()]).expect_err("no verification");
    assert!(err.contains("Independent final verification"));
    let stale = asg(Verification, Completed, Some(Outcome::Passed), Some("probe"), "review", &[r("claims/other", 2)]);
    assert!(assert_finishable(&[probe.clone(), stale]).is_err());
    let newer = asg(Verification, Completed, Some(Outcome::Passed), Some("probe"), "review", &[r("claims/task/cors", 3)]);
    assert!(assert_finishable(&[probe.clone(), newer]).is_ok(), "a newer reviewed revision still covers");
    assert!(assert_finishable(&[]).is_err());
    let failed = asg(Probe, Failed, Some(Outcome::Failed), None, "bad", &[]);
    assert!(assert_finishable(&[probe, review, failed]).is_err(), "failed work blocks finish");
}

#[test]
fn findings_need_a_completed_report_before_finish() {
    use AssignmentKind::*;
    use AssignmentStatus::*;
    let probe = asg(Probe, Completed, Some(Outcome::Passed), None, "probe", &[r("claims/a", 1)]);
    let review = asg(Verification, Completed, Some(Outcome::Passed), Some("probe"), "rev", &[r("claims/a", 1), r("findings/f", 1)]);
    assert!(assert_finishable(&[probe.clone(), review.clone()]).expect_err("needs report").contains("dual pack"));
    let report = asg(Report, Completed, Some(Outcome::Passed), Some("rev"), "rep", &[r("reports/f", 1)]);
    assert!(assert_finishable(&[probe, review, report]).is_ok());
}

// ---- delegate ----

#[test]
fn only_the_chief_may_delegate_nudge_or_finish() {
    let f = fx();
    let args = json!({ "goal_key": "g1", "goal": "g", "dispatch_key": "d", "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "p" }] });
    assert!(error(&f.lane("red", "a1", "group_delegate", args)).contains("Only the coordinator may delegate"));
    assert!(error(&f.lane("red", "a1", "group_nudge", json!({ "avatar": "red" }))).contains("may nudge"));
    assert!(error(&f.lane("red", "a1", "group_finish", json!({ "report": "r" }))).contains("may finish"));
    assert!(error(&f.chief("group_task_complete", json!({ "result": "r", "outcome": "passed", "references": [] }))).contains("No active group goal"));
}

#[test]
fn delegates_a_probe_and_starts_its_lane() {
    let f = fx();
    let out = f.delegate_probe("Scan", "round-1");
    let v = ok(&out);
    assert_eq!(v["assignments"].as_array().expect("assignments").len(), 1);
    assert!(v["note"].as_str().expect("note").contains("End this turn"));
    let a = f.assignment(0);
    assert_eq!((a.avatar.as_str(), a.kind, a.status), ("red", AssignmentKind::Probe, AssignmentStatus::Queued));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::StartLane { assignment, goal } if assignment.id == a.id && goal == "Scan")));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Visible { text, .. } if text.contains("Map the login"))));
}

#[test]
fn replays_the_same_dispatch_and_rejects_a_different_payload() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "round-1"));
    let again = f.delegate_probe("Scan", "round-1");
    assert_eq!(ok(&again)["replayed"], true);
    assert!(again.effects.iter().any(|e| matches!(e, Effect::StartLane { .. })), "queued work is restarted");
    assert_eq!(f.work().assignments.len(), 1, "no duplicate assignment");
    let different = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "round-1",
        "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "Something else" }] }));
    assert!(error(&different).contains("Dispatch key already used with different assignments"));
}

#[test]
fn the_first_batch_must_be_a_probe_and_each_kind_has_one_role() {
    let f = fx();
    let call = |assignments: Value| f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "g", "dispatch_key": "d", "assignments": assignments }));
    assert!(error(&call(json!([{ "avatar": "reporter", "kind": "report", "prompt": "p" }]))).contains("Manager report follows verification"));
    assert!(error(&call(json!([{ "avatar": "reviewer", "kind": "investigation", "prompt": "p" }]))).contains("Start a probe"));
    assert!(error(&call(json!([{ "avatar": "reviewer", "kind": "probe", "prompt": "p" }]))).contains("Only Lab Red or Lab RE may take probe"));
    let with_probe = |avatar: &str, kind: &str| call(json!([{ "avatar": "red", "kind": "probe", "prompt": "p" }, { "avatar": avatar, "kind": kind, "prompt": "q" }]));
    assert!(error(&with_probe("reporter", "investigation")).contains("Only Reviewer or Lab Researcher may take investigation"));
    assert!(error(&with_probe("red", "verification")).contains("One assignment per specialist per batch"));
    assert!(error(&call(json!([{ "avatar": "clerk", "kind": "probe", "prompt": "p" }]))).contains("Clerk keeps its own loop"));
    assert!(error(&call(json!([{ "avatar": "chief", "kind": "probe", "prompt": "p" }]))).contains("Specialist membership required"));
    assert!(error(&call(json!([{ "avatar": "ghost", "kind": "probe", "prompt": "p" }]))).contains("Specialist membership required"));
    assert!(error(&call(json!([{ "avatar": "red", "kind": "probe", "prompt": "a" }, { "avatar": "red", "kind": "probe", "prompt": "b" }])))
        .contains("One assignment per specialist per batch"));
    assert!(error(&call(json!([{ "avatar": "red", "kind": "probe", "prompt": "p", "reviews_id": "x" }]))).contains("Only verification or report"));
    assert!(error(&call(json!([]))).contains("1..32"));
    assert!(error(&call(json!([{ "avatar": "red", "kind": "bogus", "prompt": "p" }]))).contains("invalid arguments"));
    assert!(f.co.store().with_tx(|c| Ok(store::latest_work(c)?)).expect("q").is_none(), "failed calls leave no work behind");
}

#[test]
fn a_running_batch_returns_pending_unless_the_new_work_is_additive_for_an_idle_role() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "round-1"));
    let wait = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "d2",
        "assignments": [{ "avatar": "reporter", "kind": "report", "prompt": "p", "reviews_id": "x" }] }));
    assert_eq!(ok(&wait)["pending"], true, "returns instead of failing so the Chief's turn can end");
    let busy = f.delegate_probe("Scan", "d3");
    assert_eq!(ok(&busy)["pending"], true, "red is already busy");
    let joined = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "d4",
        "assignments": [{ "avatar": "reviewer", "kind": "investigation", "prompt": "Look at the login flow" }] }));
    ok(&joined);
    assert_eq!(f.work().assignments.len(), 2);
}

#[test]
fn a_later_probe_goal_on_the_same_key_is_appended() {
    let f = fx();
    ok(&f.delegate_probe("Scan the dashboard", "r1"));
    let a = f.assignment(0);
    f.co.store().with_tx(|c| store::submit_assignment(c, &a.id, "done", Outcome::Passed, &[r("results/x", 1)])).expect("submit");
    f.settle(&a.id, AssignmentStatus::Completed);
    let second = f.delegate_probe("Round 2: CORS", "r2");
    assert_eq!(ok(&second)["added_goal"], true);
    assert_eq!(f.work().prompt, "Scan the dashboard\n\nAdded goal:\nRound 2: CORS");
    let again = f.delegate_probe("Round 2: CORS", "r3");
    assert!(again.value.get("added_goal").is_none(), "the same sentence is not added twice");
}

#[test]
fn another_goal_key_or_a_different_goal_for_verification_is_refused() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    let other = f.chief("group_delegate", json!({ "goal_key": "g2", "goal": "Other", "dispatch_key": "d",
        "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "p" }] }));
    assert!(error(&other).contains("Continue active goal_key 'g1'"));
    let a = f.assignment(0);
    f.settle(&a.id, AssignmentStatus::Completed);
    let changed = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "A different goal", "dispatch_key": "d9",
        "assignments": [{ "avatar": "reviewer", "kind": "verification", "prompt": "p" }] }));
    assert!(!error(&changed).contains("already names different work"), "verification continues the original goal");
}

#[test]
fn retries_replace_a_failed_assignment_of_the_same_role_and_wait_for_uncertain_lanes() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    let first = f.assignment(0);
    f.settle(&first.id, AssignmentStatus::Failed);
    let retry = |kind: &str, avatar: &str| f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r2",
        "assignments": [{ "avatar": avatar, "kind": kind, "prompt": "again", "retry_of": first.id }] }));
    assert!(error(&retry("investigation", "reviewer")).contains("Retry must replace the same failed specialist assignment"));
    ok(&retry("probe", "red"));
    let work = f.work();
    assert_eq!(work.assignments[0].status, AssignmentStatus::Superseded);
    assert_eq!(work.assignments[1].status, AssignmentStatus::Queued);

    let unsettled = {
        let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
        Coordinator::new(WorkStore::open_in_memory().expect("store"), shelf, Arc::new(RegistryHandle::embedded().expect("registry")))
            .with_lane_settled(|_| false)
    };
    unsettled.call(&Caller::Chief, "group_delegate", &json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r1",
        "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "p" }] }));
    let id = unsettled.store().with_tx(|c| Ok(store::latest_work(c)?.expect("w").assignments[0].id.clone())).expect("id");
    unsettled.store().with_tx(|c| store::set_assignment_status(c, &id, AssignmentStatus::Uncertain)).expect("uncertain");
    let blocked = unsettled.call(&Caller::Chief, "group_delegate", &json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r2",
        "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "again", "retry_of": id }] }));
    assert!(error(&blocked).contains("settled before retrying"));
}

// ---- nudge ----

#[test]
fn steers_a_running_specialist_and_rejects_an_idle_one() {
    let f = fx();
    assert!(error(&f.chief("group_nudge", json!({ "avatar": "red" }))).contains("No active group goal"));
    ok(&f.delegate_probe("Scan", "r1"));
    let out = f.chief("group_nudge", json!({ "avatar": "red", "note": "check the API too" }));
    assert_eq!(ok(&out)["nudged"], "red");
    let steer = out.effects.iter().find_map(|e| match e { Effect::Steer { avatar, text } => Some((avatar.clone(), text.clone())), _ => None });
    let (avatar, text) = steer.expect("a steering effect");
    assert_eq!(avatar, "red");
    assert!(text.contains("publish") && text.contains("check the API too"), "{text}");
    assert!(error(&f.chief("group_nudge", json!({ "avatar": "reviewer" }))).contains("No running assignment"));
}

// ---- task_complete ----

#[test]
fn a_specialist_submits_by_id_prefix_or_without_one() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    f.publish_claim();
    let id = f.assignment(0).id;
    let done = json!({ "result": "found CORS", "outcome": "passed", "references": refs(&[r("claims/a", 2), r("evidence/e", 1)]) });

    let mut wrong = done.clone();
    wrong["assignment_id"] = json!("zzzzzzzzzz");
    assert!(error(&f.lane("red", &id, "group_task_complete", wrong)).contains(&format!("assignment_id is {id}")));

    let mut short = done.clone();
    short["assignment_id"] = json!(&id[..8]);
    let out = f.lane("red", &id, "group_task_complete", short);
    ok(&out);
    assert_eq!(f.assignment(0).status, AssignmentStatus::Submitted);
    assert_eq!(f.assignment(0).result.as_deref(), Some("found CORS"));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Visible { author, text, .. } if author == "red" && text == "found CORS")));

    let again = f.lane("red", &id, "group_task_complete", done);
    assert!(ok(&again)["note"].as_str().expect("note").contains("already submitted"));
}

#[test]
fn only_the_assigned_lane_may_submit() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    let id = f.assignment(0).id;
    let args = json!({ "result": "r", "outcome": "failed", "references": [] });
    assert!(error(&f.lane("reviewer", &id, "group_task_complete", args.clone())).contains("Only the assigned specialist"));
    assert!(error(&f.lane("red", "someone-else", "group_task_complete", args.clone())).contains("Only the assigned specialist"));
    assert!(error(&f.chief("group_task_complete", args)).contains("Only the assigned specialist"));
}

#[test]
fn a_passed_probe_must_cite_its_work_and_cited_revisions_must_still_exist() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    f.publish_claim();
    let id = f.assignment(0).id;
    let call = |references: Value| f.lane("red", &id, "group_task_complete", json!({ "result": "r", "outcome": "passed", "references": references }));
    let first = error(&call(json!([])));
    assert!(first.contains("Probe must publish claim/evidence"), "{first}");
    assert!(first.contains("Fix the named gap once"), "the first failure allows one more try");
    // the claim moved to revision 2 when its evidence linked; revision 1 is stale
    let stale = error(&call(refs(&[r("claims/a", 1), r("evidence/e", 1)])));
    assert!(stale.contains("changed or missing: claims/a"), "{stale}");
    assert!(stale.contains("failure 2/2") && stale.contains("Do not call group_task_complete again"), "{stale}");
    let blocked = call(refs(&[r("claims/a", 2), r("evidence/e", 1)]));
    assert!(error(&blocked).contains("already failed 2 times"), "the limit stops further attempts");
}

#[test]
fn accepts_a_teammates_newer_asset_and_trajectory_revision() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    f.publish_claim();
    let asset = |t: &str| f.put("put_asset", json!({ "key": "assets/h", "host": "h", "asset_type": "host", "title": t }));
    asset("one");
    asset("two"); // now revision 2
    let id = f.assignment(0).id;
    let out = f.lane("red", &id, "group_task_complete", json!({ "result": "r", "outcome": "passed",
        "references": refs(&[r("claims/a", 2), r("evidence/e", 1), r("assets/h", 1)]) }));
    ok(&out);
}

#[test]
fn a_failed_attempt_leaves_no_partial_state() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    let id = f.assignment(0).id;
    let _ = f.lane("red", &id, "group_task_complete", json!({ "result": "r", "outcome": "passed", "references": [] }));
    let a = f.assignment(0);
    assert_eq!((a.status, a.result), (AssignmentStatus::Queued, None));
}

// ---- confirmation queue ----

fn probe_done(f: &Fx) -> (Assignment, Assignment) {
    ok(&f.delegate_probe("Scan", "r1"));
    f.publish_claim();
    let probe = f.assignment(0);
    ok(&f.lane("red", &probe.id, "group_task_complete", json!({ "result": "found", "outcome": "passed",
        "references": refs(&[r("claims/a", 2), r("evidence/e", 1)]) })));
    f.settle(&probe.id, AssignmentStatus::Completed);
    // The reconciler would queue the claim and dispatch a review; do both by hand.
    let probe = f.assignment(0);
    f.co.store().with_tx(|c| enqueue_claim(c, &f.shelf, &probe, "claims/a").map(|skip| assert!(skip.is_none(), "{skip:?}"))).expect("enqueue");
    let item = f.co.store().with_tx(|c| Ok(store::review_items(c, &probe.work_id)?.remove(0))).expect("item");
    let review = Assignment {
        id: "review-assignment-1".into(), work_id: probe.work_id.clone(), dispatch_key: "review-queue:1".into(),
        avatar: "reviewer".into(), kind: AssignmentKind::Verification, prompt: "verify".into(),
        reviews_id: Some(probe.id.clone()), status: AssignmentStatus::Running, result: None, outcome: None,
        references: item.references.clone(),
    };
    f.co.store().with_tx(|c| {
        store::insert_assignment(c, &review)?;
        store::update_item(c, &item.id, ReviewStatus::Assigned, Some(&review.id), None)
    }).expect("dispatch");
    (probe, review)
}

#[test]
fn queues_a_claim_snapshot_once_and_explains_why_a_claim_is_skipped() {
    let f = fx();
    ok(&f.delegate_probe("Scan", "r1"));
    let probe = f.assignment(0);
    let enqueue = |key: &str| f.co.store().with_tx(|c| enqueue_claim(c, &f.shelf, &probe, key)).expect("enqueue");
    assert!(enqueue("claims/none").expect("reason").contains("no longer exists"));
    f.put("put_claim", json!({ "key": "claims/a", "vulnclass": "x", "name": "n", "severity": "low", "summary": "s", "host": "h", "poc": "curl", "expected_revision": 0 }));
    assert!(enqueue("claims/a").expect("reason").contains("no evidence supports this claim"));
    f.put("put_evidence", json!({ "key": "evidence/e", "claim_key": "claims/a", "evidence_kind": "http", "body": "b", "expected_revision": 0 }));
    assert_eq!(enqueue("claims/a"), None);
    assert!(enqueue("claims/a").expect("reason").contains("already queued"));
    // new evidence changes the snapshot: the old queued item is superseded by a new one
    f.put("put_evidence", json!({ "key": "evidence/e2", "claim_key": "claims/a", "evidence_kind": "http", "body": "b2", "expected_revision": 0 }));
    assert_eq!(enqueue("claims/a"), None);
    let items = f.co.store().with_tx(|c| store::review_items(c, &probe.work_id)).expect("items");
    let statuses: Vec<ReviewStatus> = items.iter().map(|i| i.status).collect();
    assert_eq!(statuses, [ReviewStatus::Superseded, ReviewStatus::Queued]);
}

#[test]
fn a_review_must_publish_a_verdict_and_a_finding_for_a_confirmed_claim() {
    let f = fx();
    let (_probe, review) = probe_done(&f);
    let complete = |references: Vec<Reference>| {
        f.co.store().with_tx(|c| complete_review_queue(c, &f.shelf, &review, Outcome::Passed, &references))
    };
    let claim_refs = vec![r("claims/a", 2), r("evidence/e", 1)];
    assert!(complete(claim_refs.clone()).expect_err("no verdict").contains("Publish a verdict for claims/a revision 2"));

    f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2, "verdict": "confirmed", "note": "ok", "expected_revision": 0 }));
    let with_verdict: Vec<Reference> = claim_refs.iter().cloned().chain([r("verdicts/v", 1)]).collect();
    assert!(complete(with_verdict.clone()).expect_err("no finding").contains("requires a published finding"));

    f.put("put_finding", json!({ "key": "findings/f", "claim_key": "claims/a", "verdict_key": "verdicts/v", "vulnclass": "cors",
        "name": "CORS", "severity": "high", "summary": "s", "host": "h", "expected_revision": 0 }));
    let all: Vec<Reference> = with_verdict.into_iter().chain([r("findings/f", 1)]).collect();
    assert_eq!(complete(all).expect("complete"), false);
    let work_id = f.assignment(0).work_id;
    let status = f.co.store().with_tx(|c| Ok(store::review_items(c, &work_id)?[0].status)).expect("status");
    assert_eq!(status, ReviewStatus::Completed);
}

#[test]
fn a_returned_verdict_marks_the_item_returned_and_a_failed_review_marks_it_failed() {
    let f = fx();
    let (probe, review) = probe_done(&f);
    f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2, "verdict": "returned", "note": "need more", "expected_revision": 0 }));
    let refs_in = vec![r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1)];
    f.co.store().with_tx(|c| complete_review_queue(c, &f.shelf, &review, Outcome::Passed, &refs_in)).expect("complete");
    let items = f.co.store().with_tx(|c| store::review_items(c, &probe.work_id)).expect("items");
    assert_eq!((items[0].status, items[0].verdict_key.as_deref()), (ReviewStatus::Returned, Some("verdicts/v")));

    let g = fx();
    let (probe, review) = probe_done(&g);
    g.co.store().with_tx(|c| complete_review_queue(c, &g.shelf, &review, Outcome::Failed, &[])).expect("complete");
    let items = g.co.store().with_tx(|c| store::review_items(c, &probe.work_id)).expect("items");
    assert_eq!(items[0].status, ReviewStatus::Failed);
}

#[test]
fn evidence_that_moves_during_review_supersedes_the_snapshot_and_requeues_the_claim() {
    let f = fx();
    let (probe, review) = probe_done(&f);
    f.put("put_evidence", json!({ "key": "evidence/e3", "claim_key": "claims/a", "evidence_kind": "http", "body": "late", "expected_revision": 0 }));
    let superseded = f.co.store().with_tx(|c| complete_review_queue(c, &f.shelf, &review, Outcome::Passed, &[])).expect("complete");
    assert!(superseded);
    let work = f.co.store().with_tx(|c| Ok(store::latest_work(c)?.expect("w"))).expect("work");
    let review_row = work.assignments.iter().find(|a| a.id == review.id).expect("review");
    assert_eq!(review_row.status, AssignmentStatus::Superseded);
    let items = f.co.store().with_tx(|c| store::review_items(c, &probe.work_id)).expect("items");
    assert_eq!(items[0].status, ReviewStatus::Superseded);
    assert!(items.iter().any(|i| i.status == ReviewStatus::Queued), "the current snapshot is queued again");
}

// ---- verification, report, finish: the whole path ----

#[test]
fn runs_a_tool_scenario_from_delegate_to_finish() {
    let f = fx();
    let (probe, review) = probe_done(&f);

    // The queue owns verification dispatch while reviews are open.
    let busy = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "d-review",
        "assignments": [{ "avatar": "reviewer", "kind": "verification", "prompt": "p" }] }));
    assert_eq!(ok(&busy)["pending"], true);

    // Reviewer: verdict, finding, then complete.
    f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2, "verdict": "confirmed", "note": "ok", "expected_revision": 0 }));
    f.put("put_finding", json!({ "key": "findings/f", "claim_key": "claims/a", "verdict_key": "verdicts/v", "vulnclass": "cors",
        "name": "CORS", "severity": "high", "summary": "s", "host": "h", "expected_revision": 0 }));
    let review_refs = [r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1), r("findings/f", 1)];
    ok(&f.lane("reviewer", &review.id, "group_task_complete", json!({ "result": "confirmed", "outcome": "passed", "references": refs(&review_refs) })));
    f.settle(&review.id, AssignmentStatus::Completed);
    let _ = probe;

    // Finish is refused until the report pack exists and the goal is closed.
    let early = f.chief("group_finish", json!({ "report": "done" }));
    assert!(error(&early).contains("Verified findings require"), "{}", early.value);

    // Chief delegates the report; reporter writes the pack first, then completes.
    let delegated = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "d-report",
        "assignments": [{ "avatar": "reporter", "kind": "report", "prompt": "write it", "reviews_id": review.id }] }));
    ok(&delegated);
    let report = f.assignment(2);
    assert_eq!((report.avatar.as_str(), report.kind), ("reporter", AssignmentKind::Report));
    assert!(report.references.iter().any(|x| x.key == "findings/f"), "the report covers the verified snapshot");

    let premature = f.lane("reporter", &report.id, "group_task_complete", json!({ "result": "r", "outcome": "passed",
        "references": refs(&[r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1), r("findings/f", 1)]) }));
    assert!(error(&premature).contains("reports/"), "needs a reports/ key: {}", premature.value);
    f.put("put_report", json!({ "key": "reports/f", "audience": "technical", "finding_key": "findings/f", "title": "t",
        "markdown": format!("# CORS\n\n{}", "detail ".repeat(80)), "expected_revision": 0 }));
    f.put("put_report", json!({ "key": "reports/exec", "audience": "manager", "finding_key": "findings/f",
        "finding_keys": ["findings/f"], "title": "Executive", "markdown": "# Executive\n\nFix CORS.", "expected_revision": 0 }));
    let report_refs = [r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1), r("findings/f", 1), r("reports/f", 1), r("reports/exec", 1)];
    ok(&f.lane("reporter", &report.id, "group_task_complete", json!({ "result": "pack written", "outcome": "passed", "references": refs(&report_refs) })));
    f.settle(&report.id, AssignmentStatus::Completed);

    // An open trajectory goal still blocks the finish.
    f.put("put_trajectory", json!({ "key": "trajectory/g1", "goal_key": "g1", "nodes": [{ "id": "goal-cors", "kind": "goal", "label": "CORS on login", "status": "open" }], "edges": [] }));
    assert!(error(&f.chief("group_finish", json!({ "report": "too soon" }))).contains("CORS on login"));
    f.put("put_trajectory", json!({ "key": "trajectory/g1", "goal_key": "g1", "nodes": [{ "id": "goal-cors", "kind": "goal", "label": "CORS on login", "status": "Met" }], "edges": [] }));

    let done = f.chief("group_finish", json!({ "report": "All verified." }));
    assert_eq!(ok(&done)["ok"], true);
    assert_eq!(f.work().status, WorkStatus::Completed);
    assert_eq!(f.work().report.as_deref(), Some("All verified."));
    assert!(done.effects.iter().any(|e| matches!(e, Effect::Visible { text, .. } if text == "All verified.")));
    assert!(error(&f.delegate_probe("Scan", "r-new")).contains("is closed"), "a closed goal key cannot be reused");
}

#[test]
fn finish_waits_for_the_confirmation_queue_and_unresolved_work() {
    let f = fx();
    assert!(error(&f.chief("group_finish", json!({ "report": "r" }))).contains("No active group goal"));
    let (_probe, review) = probe_done(&f);
    let blocked = error(&f.chief("group_finish", json!({ "report": "r" })));
    assert!(blocked.contains("Confirmation queue remains unresolved"), "{blocked}");
    // with the queue resolved, the unfinished review still blocks
    f.co.store().with_tx(|c| {
        let items = store::review_items(c, &review.work_id)?;
        store::update_item(c, &items[0].id, ReviewStatus::Completed, Some(&review.id), None)
    }).expect("resolve");
    assert!(error(&f.chief("group_finish", json!({ "report": "r" }))).contains("Required work remains unresolved"));
}

#[test]
fn status_reports_the_work_queue_and_open_goals() {
    let f = fx();
    let empty = f.chief("group_status", json!({}));
    assert!(ok(&empty)["work"].is_null());
    let (_p, _r) = probe_done(&f);
    f.put("put_trajectory", json!({ "key": "trajectory/g1", "goal_key": "g1", "nodes": [{ "id": "g", "kind": "goal", "label": "Map login" }], "edges": [] }));
    let out = f.lane("reviewer", "any", "group_status", json!({}));
    let v = ok(&out);
    assert_eq!(v["work"]["key"], "g1");
    assert_eq!(v["confirmationQueue"].as_array().expect("queue").len(), 1);
    assert_eq!(v["openGoals"], json!(["Map login"]));
}

#[test]
fn unknown_tools_and_bad_arguments_come_back_as_errors() {
    let f = fx();
    assert!(error(&f.chief("group_explode", json!({}))).contains("unknown coordinator tool"));
    assert!(error(&f.chief("group_finish", json!({ "report": "r", "extra": 1 }))).contains("invalid arguments"));
    assert!(error(&f.chief("group_finish", json!({ "report": "" }))).contains("1..20000"));
}

// ---- several avatars for one role ----

fn fx_with_two_reds() -> Fx {
    let dir = std::env::temp_dir().join(format!("koma-room-avatars-{}-{}", std::process::id(), crate::room::work::tests_nonce()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("avatars.toml"), "[[avatar]]\nid = \"red-api\"\nrole = \"red\"\ndisplay = \"Lab Red (API)\"\n").expect("write");
    let registry = Arc::new(RegistryHandle::with_overlay(dir.clone()).expect("registry"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(registry.current().avatar("red-api").is_some());
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let co = Coordinator::new(WorkStore::open_in_memory().expect("store"), Arc::clone(&shelf), registry);
    Fx { co, shelf }
}

fn two_reds(f: &Fx, dispatch: &str) -> ToolOutcome {
    f.chief(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": dispatch, "assignments": [
            { "avatar": "red", "kind": "probe", "prompt": "Web surface" },
            { "avatar": "red-api", "kind": "probe", "prompt": "API surface" } ] }),
    )
}

#[test]
fn two_reds_probe_in_parallel_each_with_its_own_lane() {
    let f = fx_with_two_reds();
    let out = two_reds(&f, "r1");
    ok(&out);
    let work = f.work();
    let avatars: Vec<&str> = work.assignments.iter().map(|a| a.avatar.as_str()).collect();
    assert_eq!(avatars, ["red", "red-api"]);
    let starts: Vec<&str> = out.effects.iter().filter_map(|e| match e { Effect::StartLane { assignment, .. } => Some(assignment.avatar.as_str()), _ => None }).collect();
    assert_eq!(starts, ["red", "red-api"]);
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Visible { text, .. } if text.starts_with("Assignment for Lab Red (API):"))));
}

#[test]
fn one_assignment_per_avatar_not_per_role() {
    let f = fx_with_two_reds();
    let dup = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "d", "assignments": [
        { "avatar": "red-api", "kind": "probe", "prompt": "a" }, { "avatar": "red-api", "kind": "probe", "prompt": "b" } ] }));
    assert!(error(&dup).contains("One assignment per specialist per batch"));
    ok(&f.delegate_probe("Scan", "r1")); // red only
    // red is busy, but red-api is idle, so it can join the running batch
    let joined = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r2",
        "assignments": [{ "avatar": "red-api", "kind": "probe", "prompt": "API surface" }] }));
    ok(&joined);
    assert_eq!(f.work().assignments.len(), 2);
    let busy = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r3",
        "assignments": [{ "avatar": "red-api", "kind": "probe", "prompt": "again" }] }));
    assert_eq!(ok(&busy)["pending"], true, "red-api is now busy too");
}

#[test]
fn a_second_avatar_gets_the_role_rules_and_no_more() {
    let f = fx_with_two_reds();
    ok(&f.delegate_probe("Scan", "r1"));
    let call = |avatar: &str, kind: &str| f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "x",
        "assignments": [{ "avatar": avatar, "kind": kind, "prompt": "p" }] }));
    assert!(error(&call("red-api", "investigation")).contains("Only Reviewer or Lab Researcher may take investigation"));
    assert!(error(&call("ghost", "probe")).contains("Specialist membership required"));
    assert!(error(&call("chief", "probe")).contains("Specialist membership required"));
    assert!(error(&call("clerk", "probe")).contains("Clerk keeps its own loop"));
}

#[test]
fn lanes_and_nudges_are_per_avatar() {
    let f = fx_with_two_reds();
    ok(&two_reds(&f, "r1"));
    let (web, api) = (f.assignment(0), f.assignment(1));
    let nudge = f.chief("group_nudge", json!({ "avatar": "red-api" }));
    assert!(nudge.effects.iter().any(|e| matches!(e, Effect::Steer { avatar, .. } if avatar == "red-api")));
    assert_eq!(nudge.effects.len(), 1, "only the nudged avatar is steered");
    // one Red cannot submit the other's assignment
    let args = json!({ "result": "r", "outcome": "failed", "references": [] });
    assert!(error(&f.lane("red", &api.id, "group_task_complete", args.clone())).contains("Only the assigned specialist"));
    ok(&f.lane("red-api", &api.id, "group_task_complete", args.clone()));
    assert_eq!(f.assignment(1).status, AssignmentStatus::Submitted);
    assert_eq!(f.assignment(0).status, AssignmentStatus::Queued);
    let _ = web;
}

#[test]
fn verification_covers_the_results_of_every_red() {
    let f = fx_with_two_reds();
    ok(&two_reds(&f, "r1"));
    f.put("put", json!({ "key": "results/web", "content": "web findings", "expected_revision": 0 }));
    f.put("put", json!({ "key": "results/api", "content": "api findings", "expected_revision": 0 }));
    for (i, (avatar, key)) in [("red", "results/web"), ("red-api", "results/api")].into_iter().enumerate() {
        let id = f.assignment(i).id;
        ok(&f.lane(avatar, &id, "group_task_complete", json!({ "result": "done", "outcome": "passed", "references": refs(&[r(key, 1)]) })));
        f.settle(&id, AssignmentStatus::Completed);
    }
    let out = f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "verify",
        "assignments": [{ "avatar": "reviewer", "kind": "verification", "prompt": "check both" }] }));
    ok(&out);
    let review = f.assignment(2);
    let keys: Vec<&str> = review.references.iter().map(|x| x.key.as_str()).collect();
    assert_eq!(keys, ["results/api", "results/web"], "the snapshot spans both Reds, sorted by key");
    assert_ne!(review.avatar, f.assignment(0).avatar);
}

// ---- reconciler ----

mod reconcile_tests {
    use std::collections::HashMap;

    use super::super::reconcile::Reconciler;
    use super::super::store;
    use super::*;
    use crate::room::transition::{LaneRunState, Observation};

    fn obs(run: Option<LaneRunState>, unsettled: bool) -> Observation {
        Observation { run, unsettled }
    }

    fn observer(map: &HashMap<String, Observation>) -> impl Fn(&str) -> Observation + '_ {
        move |id: &str| map.get(id).copied().unwrap_or(obs(None, false))
    }

    fn probe_id(f: &Fx) -> String {
        f.assignment(0).id
    }

    fn work_is_active(f: &Fx) -> bool {
        f.co.store().with_tx(|c| Ok(store::active_work(c)?.is_some())).expect("active?")
    }

    #[test]
    fn a_running_batch_earns_one_check_in_per_window() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let id = probe_id(&f);
        let wid = f.work().id;
        let mut rec = Reconciler::with_window(1_000);
        let mut seen = HashMap::new();
        seen.insert(id.clone(), obs(Some(LaneRunState::Running), false));

        let first = rec.tick(f.co.store(), observer(&seen), 0).expect("tick");
        assert_eq!(first.status_changes, [(id.clone(), AssignmentStatus::Running)]);
        assert_eq!(first.checkins, [wid.clone()]);
        assert!(first.wakes.is_empty());

        // same window, unchanged state: no new check-in, no status change
        let second = rec.tick(f.co.store(), observer(&seen), 500).expect("tick");
        assert!(second.checkins.is_empty() && second.status_changes.is_empty());

        // next window: a fresh check-in
        let third = rec.tick(f.co.store(), observer(&seen), 1_500).expect("tick");
        assert_eq!(third.checkins, [wid]);
    }

    #[test]
    fn a_settled_batch_wakes_the_chief_once() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let id = probe_id(&f);
        f.co.store()
            .with_tx(|c| store::submit_assignment(c, &id, "done", Outcome::Passed, &[r("results/x", 1)]))
            .expect("submit");
        let wid = f.work().id;
        let mut rec = Reconciler::new();
        let mut seen = HashMap::new();
        seen.insert(id.clone(), obs(Some(LaneRunState::Completed), false));

        let first = rec.tick(f.co.store(), observer(&seen), 0).expect("tick");
        assert_eq!(first.status_changes, [(id.clone(), AssignmentStatus::Completed)]);
        assert_eq!(first.wakes, [wid]);
        assert!(first.checkins.is_empty());

        // unchanged settled state: no second wake
        let second = rec.tick(f.co.store(), observer(&seen), 10_000).expect("tick");
        assert!(second.wakes.is_empty());
    }

    #[test]
    fn a_wake_nonce_does_not_fire_again_after_the_store_reopens() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("koma-room-nonce-{nanos}.sqlite"));
        let _ = std::fs::remove_file(&path);
        let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
        let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
        let co = Coordinator::new(WorkStore::open(&path).expect("store"), shelf, registry);
        co.call(
            &Caller::Chief,
            "group_delegate",
            &json!({ "goal_key": "g1", "goal": "Scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "Map the login" }] }),
        );
        let work = co.store().with_tx(|c| Ok(store::latest_work(c)?.expect("work"))).expect("work");
        let id = work.assignments[0].id.clone();
        let wid = work.id.clone();
        co.store()
            .with_tx(|c| store::submit_assignment(c, &id, "done", Outcome::Passed, &[r("results/x", 1)]))
            .expect("submit");
        let mut seen = HashMap::new();
        seen.insert(id.clone(), obs(Some(LaneRunState::Completed), false));
        let first = Reconciler::new().tick(co.store(), observer(&seen), 0).expect("tick");
        assert_eq!(first.wakes, [wid]);
        drop(co);

        let store = WorkStore::open(&path).expect("reopen");
        let second = Reconciler::new().tick(&store, observer(&seen), 10_000).expect("tick");
        assert!(second.wakes.is_empty(), "a reopened room woke the Chief again");
        let _ = std::fs::remove_file(&path);
    }

    fn two_works(store: &WorkStore) -> (String, String) {
        store
            .with_tx(|c| {
                let a = store::insert_work(c, "ga", "A")?;
                let b = store::insert_work(c, "gb", "B")?;
                Ok((a.id, b.id))
            })
            .expect("works")
    }

    #[test]
    fn a_nonce_fires_once_and_belongs_to_its_work() {
        let store = WorkStore::open_in_memory().expect("store");
        let (a, _b) = two_works(&store);
        assert!(store.remember_nonce(&a, "group-wake:a:1").expect("first"));
        assert!(!store.remember_nonce(&a, "group-wake:a:1").expect("again"), "the same nonce must not fire twice");
        assert_eq!(store.nonce_count(&a).expect("count"), 1);
    }

    #[test]
    fn a_work_that_ends_takes_its_nonces_with_it_and_other_works_keep_theirs() {
        let store = WorkStore::open_in_memory().expect("store");
        let (a, b) = two_works(&store);
        for i in 0..50 {
            store.remember_nonce(&a, &format!("group-checkin:a:{i}")).expect("a");
        }
        store.remember_nonce(&b, "group-wake:b:1").expect("b");

        store.with_tx(|c| store::set_work_status(c, &a, WorkStatus::Completed, None)).expect("complete");
        assert_eq!(store.nonce_count(&a).expect("a"), 0, "a finished work leaves no nonces behind");
        assert_eq!(store.nonce_count(&b).expect("b"), 1);
        assert!(!store.remember_nonce(&b, "group-wake:b:1").expect("b again"), "the active work must still dedup");
    }

    #[test]
    fn a_cancelled_work_is_cleaned_up_too_and_an_active_one_never_loses_a_nonce() {
        let store = WorkStore::open_in_memory().expect("store");
        let (a, b) = two_works(&store);
        store.remember_nonce(&a, "group-wake:a:1").expect("a");
        store.remember_nonce(&b, "group-wake:b:1").expect("b");
        store.with_tx(|c| store::set_work_status(c, &b, WorkStatus::Active, None)).expect("still active");
        assert_eq!(store.nonce_count(&b).expect("b"), 1, "an update that keeps the work active keeps its nonces");
        store.with_tx(|c| store::set_work_status(c, &a, WorkStatus::Cancelled, None)).expect("cancel");
        assert_eq!(store.nonce_count(&a).expect("a"), 0);
    }

    #[test]
    fn a_database_from_before_nonces_were_scoped_is_upgraded_in_place() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("koma-room-legacy-nonce-{nanos}.sqlite"));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                "CREATE TABLE fired_nonces (nonce TEXT PRIMARY KEY);
                 INSERT INTO fired_nonces (nonce) VALUES ('group-wake:old:1');",
            )
            .expect("legacy table");
        }
        let store = WorkStore::open(&path).expect("upgrade");
        assert!(!store.remember_nonce("", "group-wake:old:1").expect("old"), "an old nonce still dedups");
        assert!(store.remember_nonce("w", "group-wake:new:1").expect("new"), "a scoped nonce can be written");
        drop(store);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_lane_that_cannot_be_proven_stopped_becomes_uncertain_and_wakes() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let id = probe_id(&f);
        f.co.store()
            .with_tx(|c| store::submit_assignment(c, &id, "maybe", Outcome::Passed, &[r("results/x", 1)]))
            .expect("submit");
        let wid = f.work().id;
        let mut rec = Reconciler::new();
        let mut seen = HashMap::new();
        seen.insert(id.clone(), obs(None, true)); // lane vanished, work unproven

        let out = rec.tick(f.co.store(), observer(&seen), 0).expect("tick");
        assert_eq!(out.status_changes, [(id, AssignmentStatus::Uncertain)]);
        assert_eq!(out.wakes, [wid]);
    }

    #[test]
    fn a_cancelled_lane_cancels_the_work() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let id = probe_id(&f);
        let mut rec = Reconciler::new();
        let mut seen = HashMap::new();
        seen.insert(id, obs(Some(LaneRunState::Cancelled), false));

        let out = rec.tick(f.co.store(), observer(&seen), 0).expect("tick");
        assert!(out.wakes.is_empty() && out.checkins.is_empty());
        assert!(!work_is_active(&f), "the work is cancelled, no longer active");
    }

    #[test]
    fn a_wake_fires_again_when_the_batch_state_changes() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let id = probe_id(&f);
        let wid = f.work().id;
        let mut rec = Reconciler::new();
        let mut seen = HashMap::new();

        seen.insert(id.clone(), obs(Some(LaneRunState::Running), false));
        rec.tick(f.co.store(), observer(&seen), 0).expect("tick"); // running -> check-in
        // the lane now fails: a new state, so a wake fires
        seen.insert(id.clone(), obs(Some(LaneRunState::Failed), false));
        let out = rec.tick(f.co.store(), observer(&seen), 100).expect("tick");
        assert_eq!(out.status_changes, [(id, AssignmentStatus::Failed)]);
        assert_eq!(out.wakes, [wid]);
    }
}

// ---- dispatch: reviews, feedback and the report hand-over ----

mod dispatch_tests {
    use super::super::model::ReviewItem;
    use super::super::reconcile::{ReconcileOutcome, Reconciler};
    use super::super::store;
    use super::*;
    use crate::room::transition::{LaneRunState, Observation};

    fn tick_with(f: &Fx, rec: &mut Reconciler, idle: impl Fn(&str) -> bool) -> ReconcileOutcome {
        f.co.reconcile(rec, |_| Observation { run: Some(LaneRunState::Running), unsettled: false }, idle, 0)
            .expect("reconcile")
    }

    fn tick(f: &Fx, rec: &mut Reconciler) -> ReconcileOutcome {
        tick_with(f, rec, |_| true)
    }

    fn items(f: &Fx) -> Vec<ReviewItem> {
        let work_id = f.work().id;
        f.co.store().with_tx(|c| store::review_items(c, &work_id)).expect("items")
    }

    fn starts(out: &ReconcileOutcome) -> Vec<(String, AssignmentKind)> {
        out.effects
            .iter()
            .filter_map(|e| match e {
                Effect::StartLane { assignment, .. } => Some((assignment.avatar.clone(), assignment.kind)),
                _ => None,
            })
            .collect()
    }

    /// Publish a claim and its evidence for `claim`/`evidence` keys.
    fn publish(f: &Fx, claim: &str, evidence: &str) {
        f.put("put_claim", json!({ "key": claim, "vulnclass": "cors", "name": "CORS", "severity": "high",
            "summary": "s", "host": "h", "poc": "curl", "expected_revision": 0 }));
        f.put("put_evidence", json!({ "key": evidence, "claim_key": claim, "evidence_kind": "http", "body": "b", "expected_revision": 0 }));
    }

    #[test]
    fn a_published_claim_is_queued_and_the_next_pass_hands_it_to_the_reviewer() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let probe = f.assignment(0);

        f.put("put_claim", json!({ "key": "claims/a", "vulnclass": "cors", "name": "CORS", "severity": "high",
            "summary": "s", "host": "h", "poc": "curl", "expected_revision": 0 }));
        let skipped = f.co.on_publish("red", &probe.id, "put_claim", &json!({ "key": "claims/a" }));
        assert!(skipped.expect("skip reason").contains("no evidence supports this claim"));
        f.put("put_evidence", json!({ "key": "evidence/e", "claim_key": "claims/a", "evidence_kind": "http", "body": "b", "expected_revision": 0 }));
        assert_eq!(f.co.on_publish("red", &probe.id, "put_evidence", &json!({ "claim_key": "claims/a" })), None);
        assert_eq!(items(&f)[0].status, ReviewStatus::Queued);
        assert_eq!(f.co.on_publish("red", &probe.id, "put_asset", &json!({})), None, "other tools queue nothing");

        let mut rec = Reconciler::new();
        let out = tick(&f, &mut rec);
        assert_eq!(starts(&out), [("reviewer".to_string(), AssignmentKind::Verification)]);
        let review = f.assignment(1);
        assert_eq!((review.avatar.as_str(), review.kind), ("reviewer", AssignmentKind::Verification));
        assert_eq!(review.reviews_id.as_deref(), Some(probe.id.as_str()));
        assert_eq!(review.references, [r("claims/a", 2), r("evidence/e", 1)], "the claim moved to revision 2 when its evidence linked");
        assert!(review.prompt.contains("claims/a") && review.prompt.contains("Scan"), "{}", review.prompt);
        let queue = items(&f);
        assert_eq!((queue[0].status, queue[0].review_assignment_id.as_deref()), (ReviewStatus::Assigned, Some(review.id.as_str())));
        assert!(out.wakes.is_empty(), "the batch is still running, so only a check-in is due");

        let again = tick(&f, &mut rec);
        assert!(starts(&again).is_empty(), "the reviewer is busy, so no second review");
        assert_eq!(f.work().assignments.len(), 2);
    }

    #[test]
    fn a_reviewer_that_is_not_idle_leaves_claims_queued() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let probe = f.assignment(0);
        publish(&f, "claims/a", "evidence/e");
        f.co.on_publish("red", &probe.id, "put_evidence", &json!({ "claim_key": "claims/a" }));
        let mut rec = Reconciler::new();
        let out = tick_with(&f, &mut rec, |avatar| avatar != "reviewer");
        assert!(starts(&out).is_empty());
        assert_eq!(items(&f)[0].status, ReviewStatus::Queued);
        let out = tick(&f, &mut rec);
        assert_eq!(starts(&out).len(), 1, "dispatched once the reviewer is free");
    }

    #[test]
    fn a_review_takes_at_most_eight_claims() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let probe = f.assignment(0);
        for i in 0..9 {
            publish(&f, &format!("claims/c{i}"), &format!("evidence/e{i}"));
            f.co.on_publish("red", &probe.id, "put_evidence", &json!({ "claim_key": format!("claims/c{i}") }));
        }
        let mut rec = Reconciler::new();
        tick(&f, &mut rec);
        let queue = items(&f);
        assert_eq!(queue.iter().filter(|i| i.status == ReviewStatus::Assigned).count(), 8);
        assert_eq!(queue.iter().filter(|i| i.status == ReviewStatus::Queued).count(), 1);
        assert_eq!(f.assignment(1).references.len(), 16, "eight claims and their evidence");
    }

    /// The reviewer returns the claim: complete its review with a `returned` verdict.
    fn return_claim(f: &Fx, review_id: &str) {
        f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2,
            "verdict": "returned", "note": "need a second origin", "expected_revision": 0 }));
        ok(&f.lane("reviewer", review_id, "group_task_complete", json!({ "result": "returned", "outcome": "passed",
            "references": refs(&[r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1)]) })));
        f.settle(review_id, AssignmentStatus::Completed);
    }

    #[test]
    fn a_returned_claim_steers_a_busy_red_exactly_once() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let probe = f.assignment(0);
        publish(&f, "claims/a", "evidence/e");
        f.co.on_publish("red", &probe.id, "put_evidence", &json!({ "claim_key": "claims/a" }));
        let mut rec = Reconciler::new();
        tick(&f, &mut rec);
        let review = f.assignment(1);
        return_claim(&f, &review.id);
        assert_eq!(items(&f)[0].status, ReviewStatus::Returned);

        let out = tick(&f, &mut rec);
        let steers: Vec<&Effect> = out.effects.iter().filter(|e| matches!(e, Effect::Steer { .. })).collect();
        assert_eq!(steers.len(), 1);
        match steers[0] {
            Effect::Steer { avatar, text } => {
                assert_eq!(avatar, "red");
                assert!(text.contains("claims/a") && text.contains("need a second origin"), "{text}");
            }
            _ => unreachable!(),
        }
        assert!(items(&f)[0].feedback_sent);
        let again = tick(&f, &mut rec);
        assert!(!again.effects.iter().any(|e| matches!(e, Effect::Steer { .. })), "feedback goes out once");
    }

    #[test]
    fn a_returned_claim_restarts_an_idle_red_with_the_note_and_the_original_goal() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        return_claim(&f, &review.id);
        let mut rec = Reconciler::new();
        let out = tick(&f, &mut rec);
        assert_eq!(starts(&out), [("red".to_string(), AssignmentKind::Probe)]);
        let again = f.work().assignments.last().cloned().expect("new assignment");
        assert!(again.dispatch_key.starts_with("review-return:"), "{}", again.dispatch_key);
        assert!(again.prompt.contains("need a second origin") && again.prompt.contains("Original goal: Scan"), "{}", again.prompt);
        assert!(items(&f)[0].feedback_sent);
        let none = tick(&f, &mut rec);
        assert!(starts(&none).is_empty(), "no second restart");
    }

    #[test]
    fn a_lane_that_is_not_provably_idle_defers_the_feedback() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        return_claim(&f, &review.id);
        let mut rec = Reconciler::new();
        let held = tick_with(&f, &mut rec, |_| false);
        assert!(held.effects.is_empty() && !items(&f)[0].feedback_sent);
        let out = tick(&f, &mut rec);
        assert_eq!(starts(&out).len(), 1, "sent on the next pass");
    }

    #[test]
    fn a_review_that_fails_fails_its_items() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        f.settle(&review.id, AssignmentStatus::Failed);
        let mut rec = Reconciler::new();
        tick(&f, &mut rec);
        assert_eq!(items(&f)[0].status, ReviewStatus::Failed);
    }

    /// Confirm the claim, publish the finding and complete the review.
    fn confirm_claim(f: &Fx, review_id: &str) {
        f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2,
            "verdict": "confirmed", "note": "ok", "expected_revision": 0 }));
        f.put("put_finding", json!({ "key": "findings/f", "claim_key": "claims/a", "verdict_key": "verdicts/v",
            "vulnclass": "cors", "name": "CORS", "severity": "high", "summary": "s", "host": "h", "expected_revision": 0 }));
        ok(&f.lane("reviewer", review_id, "group_task_complete", json!({ "result": "confirmed", "outcome": "passed",
            "references": refs(&[r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1), r("findings/f", 1)]) })));
        f.settle(review_id, AssignmentStatus::Completed);
    }

    #[test]
    fn findings_after_a_passed_review_are_handed_to_the_reporter_until_the_pack_is_complete() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        confirm_claim(&f, &review.id);

        let mut rec = Reconciler::new();
        let out = tick(&f, &mut rec);
        assert_eq!(starts(&out), [("reporter".to_string(), AssignmentKind::Report)]);
        let report = f.work().assignments.last().cloned().expect("report");
        assert!(report.dispatch_key.starts_with("report-queue:"), "{}", report.dispatch_key);
        assert_eq!(report.reviews_id.as_deref(), Some(review.id.as_str()));
        assert!(report.references.iter().any(|x| x.key == "findings/f"), "the snapshot includes the finding");
        assert!(report.prompt.contains("Findings to cover: findings/f") && report.prompt.contains("missing audience=technical"), "{}", report.prompt);

        let busy = tick(&f, &mut rec);
        assert!(starts(&busy).is_empty(), "one report at a time");

        // the reporter writes the pack and finishes
        f.put("put_report", json!({ "key": "reports/f", "audience": "technical", "finding_key": "findings/f", "title": "t",
            "markdown": format!("# CORS\n\n{}", "detail ".repeat(80)), "expected_revision": 0 }));
        f.put("put_report", json!({ "key": "reports/exec", "audience": "manager", "finding_key": "findings/f",
            "finding_keys": ["findings/f"], "title": "Executive", "markdown": "# Executive\n\nFix CORS.", "expected_revision": 0 }));
        f.settle(&report.id, AssignmentStatus::Completed);
        let done = tick(&f, &mut rec);
        assert!(starts(&done).is_empty(), "a complete pack needs no further hand-over");
    }

    #[test]
    fn no_findings_means_no_report_hand_over() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        f.put("put_verdict", json!({ "key": "verdicts/v", "claim_key": "claims/a", "claim_revision": 2,
            "verdict": "dismissed", "note": "not exploitable", "expected_revision": 0 }));
        ok(&f.lane("reviewer", &review.id, "group_task_complete", json!({ "result": "dismissed", "outcome": "passed",
            "references": refs(&[r("claims/a", 2), r("evidence/e", 1), r("verdicts/v", 1)]) })));
        f.settle(&review.id, AssignmentStatus::Completed);
        let mut rec = Reconciler::new();
        let out = tick(&f, &mut rec);
        assert!(starts(&out).is_empty());
    }

    #[test]
    fn the_report_waits_for_an_idle_reporter() {
        let f = fx();
        let (_probe, review) = probe_done(&f);
        confirm_claim(&f, &review.id);
        let mut rec = Reconciler::new();
        let held = tick_with(&f, &mut rec, |avatar| avatar != "reporter");
        assert!(starts(&held).is_empty());
        assert_eq!(starts(&tick(&f, &mut rec)).len(), 1);
    }

    #[test]
    fn dispatch_happens_before_the_wake_so_a_finished_probe_starts_its_review_first() {
        let f = fx();
        ok(&f.delegate_probe("Scan", "r1"));
        let probe = f.assignment(0);
        publish(&f, "claims/a", "evidence/e");
        f.co.on_publish("red", &probe.id, "put_evidence", &json!({ "claim_key": "claims/a" }));
        ok(&f.lane("red", &probe.id, "group_task_complete", json!({ "result": "found", "outcome": "passed",
            "references": refs(&[r("claims/a", 2), r("evidence/e", 1)]) })));
        // the probe's lane ended, so the reconciler sees it complete and in the same pass starts the review
        let mut rec = Reconciler::new();
        let out = f
            .co
            .reconcile(&mut rec, |_| Observation { run: Some(LaneRunState::Completed), unsettled: false }, |_| true, 0)
            .expect("reconcile");
        assert_eq!(f.assignment(0).status, AssignmentStatus::Completed);
        assert_eq!(starts(&out), [("reviewer".to_string(), AssignmentKind::Verification)]);
        assert!(out.wakes.is_empty(), "the Chief is not woken for a batch that just gained a review");
        assert_eq!(out.checkins.len(), 1);
    }
}

#[test]
fn the_extra_specialists_take_the_kinds_their_roles_allow() {
    // One goal is active at a time, so each case gets its own room.
    let delegate = |avatar: &str, kind: &str| {
        let f = fx();
        f.chief("group_delegate", json!({ "goal_key": "g1", "goal": "g", "dispatch_key": "d1", "assignments": [{ "avatar": avatar, "kind": kind, "prompt": "p" }] }))
    };
    // Lab RE takes a probe, and an execution may open a goal like a probe does.
    ok(&delegate("re", "probe"));
    ok(&delegate("builder", "execution"));
    // The researcher investigates and nothing else; the builder builds and nothing else.
    assert!(error(&delegate("researcher", "probe")).contains("Only Lab Red or Lab RE may take probe"));
    assert!(error(&delegate("builder", "probe")).contains("Only Lab Red or Lab RE may take probe"));
    assert!(error(&delegate("red", "execution")).contains("Only Lab Builder may take execution"));
}
