//! The Chief's tools (`group_delegate`, `group_status`, `group_nudge`,
//! `group_finish`) and the specialists' `group_task_complete`.
//!
//! Ported from Orpheus `groupWorkTool`. Each call runs in one transaction. The
//! coordinator decides and records; anything that must happen in the world
//! (start a lane, steer one, post a message) comes back as an [`Effect`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::effects::Effect;
use super::finish::{assert_finishable, required_review_references};
use super::goals::{append_goal, open_trajectory_goals};
use super::model::{Assignment, ReviewItem, ReviewStatus, Work};
use super::dispatch::{report_dispatch, review_queue};
use super::queue::{complete_review_queue, enqueue_claim};
use super::reconcile::{ReconcileOutcome, Reconciler};
use super::store::{self, new_id, WorkStore, R};
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use crate::room::transition::Observation;
use crate::room::types::{
    covers_references, same_references, validate_references, AssignmentKind, AssignmentStatus, Outcome,
    Reference, WorkStatus,
};

/// A specialist that fails `group_task_complete` this many times is told to stop calling it.
const COMPLETE_FAIL_LIMIT: u32 = 2;
const SPECIALISTS: [&str; 7] = ["red", "re", "reviewer", "researcher", "reporter", "builder", "blue"];

/// Who is calling a tool.
#[derive(Debug, Clone, PartialEq)]
pub enum Caller {
    Chief,
    /// A specialist's work lane, running one assignment.
    Lane { avatar: String, assignment_id: String },
}

/// A tool result plus what the room must now do.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub value: Value,
    pub effects: Vec<Effect>,
}

type Out = (Value, Vec<Effect>);

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignmentInput {
    avatar: String,
    kind: AssignmentKind,
    prompt: String,
    reviews_id: Option<String>,
    retry_of: Option<String>,
    references: Option<Vec<Reference>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateInput {
    goal_key: String,
    goal: String,
    dispatch_key: String,
    assignments: Vec<AssignmentInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteInput {
    assignment_id: Option<String>,
    result: String,
    outcome: Outcome,
    references: Vec<Reference>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishInput {
    report: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NudgeInput {
    avatar: String,
    note: Option<String>,
}

fn parse<T: DeserializeOwned>(args: &Value) -> R<T> {
    serde_json::from_value(args.clone()).map_err(|e| format!("invalid arguments: {e}"))
}

fn len_between(name: &str, s: &str, max: usize) -> R<()> {
    let n = s.chars().count();
    if n == 0 || n > max {
        return Err(format!("{name} must be 1..{max} chars"));
    }
    Ok(())
}

fn validate_delegate(input: &DelegateInput) -> R<()> {
    len_between("goal_key", &input.goal_key, 128)?;
    len_between("goal", &input.goal, 20_000)?;
    len_between("dispatch_key", &input.dispatch_key, 128)?;
    if input.assignments.is_empty() || input.assignments.len() > 32 {
        return Err("assignments must have 1..32 entries".to_string());
    }
    for a in &input.assignments {
        len_between("prompt", &a.prompt, 20_000)?;
        if let Some(refs) = &a.references {
            validate_references(refs)?;
        }
    }
    Ok(())
}

fn is_production_done(a: &Assignment) -> bool {
    a.kind.is_production() && a.status == AssignmentStatus::Completed && a.outcome == Some(Outcome::Passed)
}

fn is_result_ref(key: &str) -> bool {
    ["claim/", "claims/", "evidence/", "results/", "findings/"].iter().any(|p| key.starts_with(p))
}

pub struct Coordinator {
    store: WorkStore,
    shelf: Arc<Shelf>,
    registry: Arc<RegistryHandle>,
    complete_fails: Mutex<HashMap<String, u32>>,
    lane_settled: Box<dyn Fn(&str) -> bool + Send + Sync>,
}

impl Coordinator {
    pub fn new(store: WorkStore, shelf: Arc<Shelf>, registry: Arc<RegistryHandle>) -> Self {
        Self {
            store,
            shelf,
            registry,
            complete_fails: Mutex::new(HashMap::new()),
            lane_settled: Box::new(|_| true),
        }
    }

    /// Tell the coordinator whether an avatar's lane is provably idle. A retry of
    /// uncertain work waits until it is.
    pub fn with_lane_settled(mut self, f: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.lane_settled = Box::new(f);
        self
    }

    pub fn store(&self) -> &WorkStore {
        &self.store
    }

    /// One reconciler pass: sync assignments from their lanes, start reviews and
    /// report hand-overs, then decide whether to wake the Chief. `lane_idle`
    /// says whether an avatar's lane is free to take new work.
    pub fn reconcile(
        &self,
        reconciler: &mut Reconciler,
        observe: impl Fn(&str) -> Observation,
        lane_idle: impl Fn(&str) -> bool,
        now_ms: u64,
    ) -> R<ReconcileOutcome> {
        let registry = self.registry.current();
        reconciler.tick_with(&self.store, observe, now_ms, |conn, work| {
            let mut effects = review_queue(conn, &self.shelf, &registry, work, &lane_idle)?;
            effects.extend(report_dispatch(conn, &self.shelf, &registry, work, &lane_idle)?);
            Ok(effects)
        })
    }

    /// A probe lane just wrote to the shelf. If it was a claim or its evidence,
    /// queue the claim for review now instead of waiting for the lane to end.
    /// Returns why the claim was not queued, so the lane can fix it.
    pub fn on_publish(&self, avatar: &str, assignment_id: &str, tool: &str, args: &Value) -> Option<String> {
        let claim_key = match tool {
            "put_claim" => args.get("key"),
            "put_evidence" => args.get("claim_key"),
            _ => return None,
        }?
        .as_str()?
        .to_string();
        self.store
            .with_tx(|conn| {
                let Some(work) = store::active_work(conn)? else {
                    return Ok(None);
                };
                let source = work.assignments.iter().find(|a| {
                    a.id == assignment_id && a.avatar == avatar && a.kind == AssignmentKind::Probe && a.status.is_pending()
                });
                match source {
                    Some(source) => enqueue_claim(conn, &self.shelf, source, &claim_key),
                    None => Ok(None),
                }
            })
            .unwrap_or(None)
    }

    /// Run a coordinator tool. Failures come back as `{"error": "..."}` so the
    /// model can read and act on them.
    pub fn call(&self, caller: &Caller, tool: &str, args: &Value) -> ToolOutcome {
        match self.dispatch(caller, tool, args) {
            Ok((value, effects)) => ToolOutcome { value, effects },
            Err(error) => ToolOutcome { value: json!({ "error": error }), effects: Vec::new() },
        }
    }

    fn dispatch(&self, caller: &Caller, tool: &str, args: &Value) -> R<Out> {
        match tool {
            "group_status" => self.store.with_tx(|c| self.status(c)),
            "group_nudge" => {
                require_chief(caller, "Only the coordinator may nudge")?;
                let input: NudgeInput = parse(args)?;
                self.store.with_tx(|c| self.nudge(c, input))
            }
            "group_delegate" => {
                require_chief(caller, "Only the coordinator may delegate")?;
                let input: DelegateInput = parse(args)?;
                validate_delegate(&input)?;
                self.store.with_tx(|c| self.delegate(c, input))
            }
            "group_task_complete" => {
                let input: CompleteInput = parse(args)?;
                self.store.with_tx(|c| self.task_complete(c, caller, input))
            }
            "group_finish" => {
                require_chief(caller, "Only the coordinator may finish")?;
                let input: FinishInput = parse(args)?;
                len_between("report", &input.report, 20_000)?;
                self.store.with_tx(|c| self.finish(c, input))
            }
            _ => Err(format!("unknown coordinator tool '{tool}'")),
        }
    }

    /// The active engagement with its assignments, for the clerk's audit and the
    /// reconciler's observation set. `None` when nothing is active.
    pub fn active_work(&self) -> Option<super::model::Work> {
        self.store.with_tx(store::active_work).ok().flatten()
    }

    /// The active engagement's key, goal text and open trajectory goals, for the
    /// wake and check-in prompts. `None` when nothing is active.
    pub fn active_brief(&self) -> Option<(String, String, Vec<String>)> {
        let open = self.open_goals().unwrap_or_default();
        self.store
            .with_tx(|conn| Ok(store::active_work(conn)?.map(|w| (w.key, w.prompt))))
            .ok()
            .flatten()
            .map(|(key, prompt)| (key, prompt, open))
    }

    fn open_goals(&self) -> R<Vec<String>> {
        let rows = self.shelf.scan("trajectory/").map_err(|e| e.0)?;
        let contents: Vec<String> = rows.into_iter().filter_map(|r| r.content).collect();
        Ok(open_trajectory_goals(&contents))
    }

    fn status(&self, conn: &Connection) -> R<Out> {
        let active = store::active_work(conn)?;
        let queue: Vec<ReviewItem> = match &active {
            Some(w) => store::review_items(conn, &w.id)?,
            None => Vec::new(),
        };
        let work = match active {
            Some(w) => Some(w),
            None => store::latest_work(conn)?,
        };
        Ok((
            json!({ "confirmationQueue": queue, "openGoals": self.open_goals()?, "work": work }),
            Vec::new(),
        ))
    }

    fn nudge(&self, conn: &Connection, input: NudgeInput) -> R<Out> {
        let active = store::active_work(conn)?.ok_or("No active group goal")?;
        let target = active
            .assignments
            .iter()
            .find(|a| a.avatar == input.avatar && a.status.is_pending())
            .ok_or("No running assignment for that specialist")?;
        if let Some(note) = &input.note {
            if note.chars().count() > 2000 {
                return Err("note must be at most 2000 chars".to_string());
            }
        }
        let note = input.note.clone().unwrap_or_default();
        let text = self
            .registry
            .current()
            .render("nudge", &[("note", note.as_str())])
            .map_err(|e| e.to_string())?;
        Ok((
            json!({
                "ok": true,
                "nudged": target.avatar,
                "note": "Nudge delivered. Nudge other specialists if they are sitting on unpublished work, then end your turn."
            }),
            vec![Effect::Steer { avatar: input.avatar, text }],
        ))
    }

    fn delegate(&self, conn: &Connection, input: DelegateInput) -> R<Out> {
        let previous = store::work_by_key(conn, &input.goal_key)?;
        let active = store::active_work(conn)?;
        let follow_up_probe = previous.is_some()
            && input
                .assignments
                .iter()
                .all(|a| matches!(a.kind, AssignmentKind::Probe | AssignmentKind::Investigation));
        if let Some(prev) = &previous {
            let only_follow_ups = input
                .assignments
                .iter()
                .all(|a| matches!(a.kind, AssignmentKind::Verification | AssignmentKind::Report) || a.retry_of.is_some());
            if prev.prompt != input.goal && !follow_up_probe && !only_follow_ups {
                return Err(format!(
                    "Goal key '{}' already names different work. Continue its original goal or finish it before starting another goal.",
                    input.goal_key
                ));
            }
            if prev.status != WorkStatus::Active {
                return Err(format!("Goal '{}' is closed. Pick a new goal_key for follow-up work.", input.goal_key));
            }
        }
        if let Some(act) = &active {
            if act.key != input.goal_key {
                return Err(format!(
                    "Continue active goal_key '{}' for verification, reporting, and retries. Do not finish the goal before verifying it.",
                    act.key
                ));
            }
        }
        let mut work: Work = match previous {
            Some(w) => w,
            None => store::insert_work(conn, &input.goal_key, &input.goal)?,
        };

        // The same dispatch key replays; a different payload under it is an error.
        let replay: Vec<&Assignment> = work.assignments.iter().filter(|a| a.dispatch_key == input.dispatch_key).collect();
        if !replay.is_empty() {
            let same = replay.len() == input.assignments.len()
                && replay.iter().all(|a| {
                    input.assignments.iter().any(|b| {
                        b.avatar == a.avatar
                            && b.kind == a.kind
                            && b.prompt == a.prompt
                            && (b.reviews_id.is_none() || b.reviews_id == a.reviews_id)
                            && b.references.as_deref().is_none_or(|r| same_references(r, &a.references))
                    })
                });
            if !same {
                return Err("Dispatch key already used with different assignments".to_string());
            }
            let effects = replay
                .iter()
                .filter(|a| a.status == AssignmentStatus::Queued)
                .map(|a| Effect::StartLane { assignment: (*a).clone(), goal: work.prompt.clone() })
                .collect();
            return Ok((json!({ "assignments": replay, "replayed": true }), effects));
        }

        let pending: Vec<&Assignment> = work.assignments.iter().filter(|a| a.status.is_pending()).collect();

        if input.assignments.iter().any(|a| a.kind == AssignmentKind::Verification && a.retry_of.is_none()) {
            let items: Vec<ReviewItem> = store::review_items(conn, &work.id)?
                .into_iter()
                .filter(|i| i.status != ReviewStatus::Superseded)
                .collect();
            if items.iter().any(|i| i.status != ReviewStatus::Completed) {
                return Ok((
                    json!({
                        "pending": true,
                        "note": "The confirmation queue owns these reviews and returned-claim follow-ups. Do not dispatch duplicate verification. Use group_status for failed items and retry_of for a failed review; otherwise end this turn and wait for queue progress."
                    }),
                    Vec::new(),
                ));
            }
            let requested: Vec<Reference> = input
                .assignments
                .iter()
                .flat_map(|a| match &a.references {
                    Some(refs) => refs.clone(),
                    None => work
                        .assignments
                        .iter()
                        .filter(|s| is_production_done(s))
                        .flat_map(|s| required_review_references(&s.references))
                        .collect(),
                })
                .collect();
            if !items.is_empty()
                && !requested.is_empty()
                && requested
                    .iter()
                    .all(|r| items.iter().any(|i| i.references.iter().any(|o| o.key == r.key && o.revision >= r.revision)))
            {
                return Err("These claim/evidence revisions are already verified by the confirmation queue. Continue reporting or group_finish; do not review them again.".to_string());
            }
        }

        if !pending.is_empty() {
            // A running batch may take more probe/investigation work for idle avatars; anything else waits.
            // This returns instead of failing so the Chief's turn ends and it is woken when the batch settles.
            let additive = input
                .assignments
                .iter()
                .all(|a| matches!(a.kind, AssignmentKind::Probe | AssignmentKind::Investigation));
            let no_busy_duplicate = input.assignments.iter().all(|a| !pending.iter().any(|p| p.avatar == a.avatar));
            if !additive || !no_busy_duplicate {
                return Ok((
                    json!({
                        "pending": true,
                        "note": "A batch is still running. End your turn now — you are woken automatically when it settles or needs attention. To add parallel work mid-batch, dispatch only probe/investigation to idle specialists (one assignment per avatar)."
                    }),
                    Vec::new(),
                ));
            }
        }

        let mut avatars: Vec<&str> = input.assignments.iter().map(|a| a.avatar.as_str()).collect();
        avatars.sort_unstable();
        avatars.dedup();
        if avatars.len() != input.assignments.len() {
            return Err("One assignment per specialist per batch".to_string());
        }

        // Only a follow-up probe round on an existing key may add goal text.
        let mut appended = false;
        if follow_up_probe && work.prompt != input.goal {
            let next = append_goal(&work.prompt, &input.goal);
            if next != work.prompt {
                store::set_work_prompt(conn, &work.id, &next)?;
                work.prompt = next;
                appended = true;
            }
        }

        if work.assignments.is_empty() {
            if input.assignments.iter().any(|a| a.kind == AssignmentKind::Report) {
                return Err("Manager report follows verification; start a probe first".to_string());
            }
            if !input.assignments.iter().any(|a| matches!(a.kind, AssignmentKind::Probe | AssignmentKind::Execution)) {
                return Err("Start a probe or an execution; Reviewer verifies the published results afterward".to_string());
            }
        }

        let existing = work.assignments.clone();
        let registry = self.registry.current();
        let mut created = Vec::new();
        let mut effects = Vec::new();

        for mut item in input.assignments.clone() {
            if let Some(retry_of) = &item.retry_of {
                let old = existing
                    .iter()
                    .find(|a| &a.id == retry_of && matches!(a.status, AssignmentStatus::Failed | AssignmentStatus::Uncertain))
                    .filter(|a| a.avatar == item.avatar && a.kind == item.kind)
                    .ok_or("Retry must replace the same failed specialist assignment")?;
                if !(self.lane_settled)(&old.avatar) {
                    return Err("Observe uncertain lane work until it is settled before retrying".to_string());
                }
                store::set_assignment_status(conn, &old.id, AssignmentStatus::Superseded)?;
            }

            // The avatar's role decides what it may take.
            let role = registry.avatar(&item.avatar).map(|a| a.role.as_str());
            if role == Some("clerk") {
                return Err("The Clerk keeps its own loop".to_string());
            }
            if !role.is_some_and(|r| SPECIALISTS.contains(&r)) || role.and_then(|r| registry.role(r)).is_none() {
                return Err("Specialist membership required".to_string());
            }
            if !role.is_some_and(|r| item.kind.allows(r)) {
                return Err(match item.kind {
                    AssignmentKind::Probe => "Only Lab Red or Lab RE may take probe",
                    AssignmentKind::Investigation => "Only Reviewer, Lab Researcher or Lab Blue may take investigation",
                    AssignmentKind::Verification => "Only Reviewer may take verification",
                    AssignmentKind::Report => "Only Lab Reporter may take report",
                    AssignmentKind::Execution => "Only Lab Builder or Lab Blue may take execution",
                }
                .to_string());
            }

            match item.kind {
                AssignmentKind::Verification => {
                    let reviewed_id = item
                        .reviews_id
                        .clone()
                        .or_else(|| existing.iter().find(|a| is_production_done(a)).map(|a| a.id.clone()));
                    item.reviews_id = reviewed_id.clone();
                    let reviewed = existing
                        .iter()
                        .find(|a| Some(&a.id) == reviewed_id.as_ref() && is_production_done(a))
                        .filter(|a| a.avatar != item.avatar)
                        .ok_or("Verification must follow completed independent probe")?;
                    let mut snapshot = match &item.references {
                        Some(refs) => refs.clone(),
                        None => {
                            let mut by_key: Vec<Reference> = Vec::new();
                            for r in existing.iter().filter(|a| is_production_done(a)).flat_map(|a| a.references.iter()) {
                                match by_key.iter_mut().find(|k| k.key == r.key) {
                                    Some(slot) => *slot = r.clone(),
                                    None => by_key.push(r.clone()),
                                }
                            }
                            for r in &mut by_key {
                                if let Some(row) = self.shelf.peek(&r.key).map_err(|e| e.0)? {
                                    r.revision = row.revision.max(0) as u64;
                                }
                            }
                            by_key
                        }
                    };
                    snapshot.sort_by(|a, b| a.key.cmp(&b.key));
                    if snapshot.is_empty() {
                        return Err("Verification requires published claim, evidence, or result references".to_string());
                    }
                    if !snapshot.iter().any(|r| is_result_ref(&r.key)) {
                        return Err("Reviewer needs published claims, evidence, or results before verification; an asset map alone is not ready for review".to_string());
                    }
                    if !covers_references(&reviewed.references, &snapshot) {
                        return Err("Review must cover every published result key at its current revision".to_string());
                    }
                    self.shelf.verify_references(&snapshot).map_err(|e| e.0)?;
                    item.references = Some(snapshot);
                    for old in existing.iter().filter(|a| {
                        a.kind == AssignmentKind::Investigation
                            && a.avatar == item.avatar
                            && !a.status.is_pending()
                            && a.status != AssignmentStatus::Superseded
                    }) {
                        store::set_assignment_status(conn, &old.id, AssignmentStatus::Superseded)?;
                    }
                }
                AssignmentKind::Report => {
                    let verified = existing
                        .iter()
                        .find(|a| {
                            Some(&a.id) == item.reviews_id.as_ref()
                                && a.kind == AssignmentKind::Verification
                                && a.status == AssignmentStatus::Completed
                                && a.outcome == Some(Outcome::Passed)
                        })
                        .filter(|a| a.avatar != item.avatar)
                        .ok_or("Manager report must follow a completed independent verification")?;
                    let snapshot = item.references.clone().unwrap_or_else(|| verified.references.clone());
                    self.shelf.verify_references(&snapshot).map_err(|e| e.0)?;
                }
                _ => {
                    if item.reviews_id.is_some() {
                        return Err("Only verification or report can reference prior work".to_string());
                    }
                    if item.references.is_some() {
                        return Err("Only verification or report can select result revisions".to_string());
                    }
                }
            }

            let review_refs = if item.kind.may_reference() {
                match &item.references {
                    Some(refs) => refs.clone(),
                    None => existing
                        .iter()
                        .find(|a| Some(&a.id) == item.reviews_id.as_ref())
                        .map(|a| a.references.clone())
                        .unwrap_or_default(),
                }
            } else {
                Vec::new()
            };

            let assignment = Assignment {
                id: new_id(),
                work_id: work.id.clone(),
                dispatch_key: input.dispatch_key.clone(),
                avatar: item.avatar.clone(),
                kind: item.kind,
                prompt: item.prompt.clone(),
                reviews_id: item.reviews_id.clone(),
                status: AssignmentStatus::Queued,
                result: None,
                outcome: None,
                references: review_refs,
            };
            store::insert_assignment(conn, &assignment)?;
            if let Some(retry_of) = &item.retry_of {
                store::reassign_failed_items(conn, retry_of, &assignment.id)?;
            }
            let display = registry.display_of(&item.avatar);
            effects.push(Effect::Visible {
                author: "chief".to_string(),
                text: format!("Assignment for {display}: {}", item.prompt),
                nonce: format!("group-dispatch:{}:{}:{}", work.id, input.dispatch_key, item.avatar),
                work_id: work.id.clone(),
            });
            effects.push(Effect::StartLane { assignment: assignment.clone(), goal: work.prompt.clone() });
            created.push(assignment);
        }

        let note = if appended {
            "Added goal appended on this goal_key. The original goal stays. Assignments queued in parallel. End this turn. You will be woken when the batch settles or needs attention."
        } else {
            "Assignments queued in parallel. End this turn. You will be woken when the batch settles or needs attention."
        };
        let mut value = json!({ "assignments": created, "note": note });
        if appended {
            value["added_goal"] = json!(true);
        }
        Ok((value, effects))
    }

    fn fail_complete(&self, assignment_id: &str, message: &str) -> String {
        let mut fails = self.complete_fails.lock().unwrap_or_else(|e| e.into_inner());
        let next = fails.get(assignment_id).copied().unwrap_or(0) + 1;
        fails.insert(assignment_id.to_string(), next);
        if next >= COMPLETE_FAIL_LIMIT {
            format!("{message} This was failure {next}/{COMPLETE_FAIL_LIMIT}. Do not call group_task_complete again. Shelf publishes are still allowed — finish those, then end your turn.")
        } else {
            format!("{message} Fix the named gap once, then call group_task_complete a final time. Do not alternate get and complete.")
        }
    }

    fn task_complete(&self, conn: &Connection, caller: &Caller, input: CompleteInput) -> R<Out> {
        let active = store::active_work(conn)?.ok_or("No active group goal")?;
        let Caller::Lane { avatar, assignment_id } = caller else {
            return Err("Only the assigned specialist run can submit a result".to_string());
        };
        validate_references(&input.references)?;
        len_between("result", &input.result, 20_000)?;
        let Some(own) = active.assignments.iter().find(|a| &a.id == assignment_id && &a.avatar == avatar) else {
            return Err("Only the assigned specialist run can submit a result".to_string());
        };
        let cited = input.assignment_id.as_deref().map(str::trim).unwrap_or("");
        let cited_ok = cited.is_empty() || cited == own.id || (cited.len() >= 8 && own.id.starts_with(cited));
        if !cited_ok || !own.status.is_pending() {
            return Err(format!(
                "This run's assignment_id is {}. Pass that exact string, a prefix of at least 8 characters, or omit assignment_id.",
                own.id
            ));
        }
        if own.status == AssignmentStatus::Submitted {
            return Ok((json!({ "ok": true, "note": "Result already submitted. End your turn." }), Vec::new()));
        }
        let prior = self.complete_fails.lock().unwrap_or_else(|e| e.into_inner()).get(&own.id).copied().unwrap_or(0);
        if prior >= COMPLETE_FAIL_LIMIT {
            return Ok((
                json!({ "error": format!("group_task_complete already failed {prior} times for this assignment. Do not call it again. Shelf publishes (put_claim, put_evidence, put_verdict, put_finding, put_result, put_trajectory, put_report) are still allowed — finish those, then end your turn.") }),
                Vec::new(),
            ));
        }
        let fail = |msg: &str| Err(self.fail_complete(&own.id, msg));

        let passed = input.outcome == Outcome::Passed;
        if passed && own.kind == AssignmentKind::Probe && input.references.is_empty() {
            return fail("Probe must publish claim/evidence shelf references.");
        }
        if passed && own.kind == AssignmentKind::Report && !input.references.iter().any(|r| r.key.starts_with("reports/")) {
            return fail("Report must publish reports/ shelf references (technical per finding + manager executive).");
        }
        if passed && own.kind == AssignmentKind::Report {
            if let Err(e) = self.shelf.assert_reporter_pack_complete() {
                return fail(&e.0);
            }
        }
        if own.kind == AssignmentKind::Verification {
            match complete_review_queue(conn, &self.shelf, own, input.outcome, &input.references) {
                Ok(true) => {
                    self.complete_fails.lock().unwrap_or_else(|e| e.into_inner()).remove(&own.id);
                    return Ok((
                        json!({ "ok": true, "note": "Evidence changed during review. This snapshot was superseded and current evidence is queued. End your turn." }),
                        Vec::new(),
                    ));
                }
                Ok(false) => {}
                Err(e) => return fail(&e),
            }
        }
        if let Err(e) = self.shelf.verify_references(&input.references) {
            return fail(&e.0);
        }
        if own.kind == AssignmentKind::Verification {
            let reviewed = active.assignments.iter().find(|a| Some(&a.id) == own.reviews_id.as_ref());
            if reviewed.is_none() || !covers_references(&own.references, &input.references) {
                return fail("Verification must cover the reviewed result revisions and may include its verdicts and findings.");
            }
        }
        if own.kind == AssignmentKind::Report {
            if !active.assignments.iter().any(|a| Some(&a.id) == own.reviews_id.as_ref()) {
                return fail("Report must follow the verification assignment it reviews.");
            }
            if !covers_references(&own.references, &input.references) {
                return fail("Report must cover the verified shelf revisions.");
            }
        }

        self.complete_fails.lock().unwrap_or_else(|e| e.into_inner()).remove(&own.id);
        store::submit_assignment(conn, &own.id, &input.result, input.outcome, &input.references)?;
        Ok((
            json!({ "ok": true, "note": "Result recorded; completion waits for this run to end. End your turn." }),
            vec![Effect::Visible {
                author: avatar.clone(),
                text: input.result,
                nonce: format!("group-result:{}", own.id),
                work_id: own.work_id.clone(),
            }],
        ))
    }

    fn finish(&self, conn: &Connection, input: FinishInput) -> R<Out> {
        let active = store::active_work(conn)?.ok_or("No active group goal")?;
        let blocking = store::review_items(conn, &active.id)?.iter().any(|i| i.status.blocks_finish());
        if blocking {
            return Err("Confirmation queue remains unresolved; wait for Reviewer or address returned/failed items before group_finish".to_string());
        }
        assert_finishable(&active.assignments)?;
        self.shelf.assert_reporter_pack_complete().map_err(|e| e.0)?;
        let open = self.open_goals()?;
        if !open.is_empty() {
            return Err(format!(
                "Open trajectory goals remain: {}. Dispatch another probe round, or mark each met, done, stopped, or budget exhausted. group_finish waits until every goal is exhausted.",
                open.join("; ")
            ));
        }
        // The newest revision of everything a passed verification covered must still exist.
        let mut reviewed: Vec<Reference> = Vec::new();
        for review in active.assignments.iter().filter(|a| {
            a.kind == AssignmentKind::Verification && a.status == AssignmentStatus::Completed && a.outcome == Some(Outcome::Passed)
        }) {
            for r in &review.references {
                match reviewed.iter_mut().find(|k| k.key == r.key) {
                    Some(slot) if slot.revision < r.revision => *slot = r.clone(),
                    Some(_) => {}
                    None => reviewed.push(r.clone()),
                }
            }
        }
        self.shelf.verify_references(&reviewed).map_err(|e| e.0)?;

        store::set_work_status(conn, &active.id, WorkStatus::Completed, Some(&input.report))?;
        Ok((
            json!({ "ok": true, "note": "Verified goal completed. End this turn." }),
            vec![Effect::Visible {
                author: "chief".to_string(),
                text: input.report,
                nonce: format!("group-finish:{}", active.id),
                work_id: active.id.clone(),
            }],
        ))
    }
}

fn require_chief(caller: &Caller, message: &str) -> R<()> {
    match caller {
        Caller::Chief => Ok(()),
        Caller::Lane { .. } => Err(message.to_string()),
    }
}
