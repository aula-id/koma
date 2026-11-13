use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::scope::{in_scope, normalize_host, parse_scopes, Scope};
use super::*;
use crate::room::lane::{Digest, InboxItem, LaneEvent};
use crate::room::registry::RegistryHandle;
use crate::room::types::{AssignmentKind, AssignmentStatus};
use crate::room::work::Assignment;

fn shelf() -> Shelf {
    Shelf::open_in_memory().expect("shelf")
}

fn put(s: &Shelf, tool: &str, args: Value) {
    s.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {}", e.0));
}

fn scope(host: &str, allow: &[&str], deny: &[&str]) -> Scope {
    Scope {
        key: "scope/x".into(),
        allow: std::iter::once(host).chain(allow.iter().copied()).map(normalize_host).collect(),
        deny: deny.iter().map(|d| normalize_host(d)).collect(),
    }
}

fn asg(avatar: &str, kind: AssignmentKind, status: AssignmentStatus, prompt: &str) -> Assignment {
    Assignment {
        id: format!("{avatar}-{prompt}-{status:?}"), work_id: "w".into(), dispatch_key: "d".into(),
        avatar: avatar.into(), kind, prompt: prompt.into(), reviews_id: None, status, result: None,
        outcome: None, references: Vec::new(),
    }
}

fn running_probe(avatar: &str) -> Assignment {
    asg(avatar, AssignmentKind::Probe, AssignmentStatus::Running, "scan")
}

fn claim(s: &Shelf, key: &str, host: &str) {
    put(s, "put_claim", json!({ "key": key, "vulnclass": "cors", "name": "n", "severity": "low",
        "summary": "s", "host": host, "poc": "curl", "expected_revision": 0 }));
}

struct Env {
    shelf: Shelf,
    digest: Digest,
    assignments: Vec<Assignment>,
    paused: BTreeSet<String>,
    now_ms: u64,
}

impl Env {
    fn new() -> Self {
        Self { shelf: shelf(), digest: Digest::new(), assignments: vec![running_probe("red")], paused: BTreeSet::new(), now_ms: 1_000_000 }
    }
    fn input(&self) -> AuditInput<'_> {
        AuditInput {
            shelf: &self.shelf, digest: &self.digest, assignments: &self.assignments, paused: &self.paused,
            now_ms: self.now_ms, config: AuditConfig::default(),
        }
    }
    fn findings(&self) -> Vec<Finding> {
        audit(&self.input()).expect("audit")
    }
    fn scope_record(&self, host: &str) {
        put(&self.shelf, "put_scope", json!({ "key": "scope/gate", "host": host, "expected_revision": 0 }));
    }
}

// ---- scope matching ----

#[test]
fn normalizes_urls_ports_and_case_to_a_bare_host() {
    for (raw, want) in [
        ("https://App.Example.com:8443/login?x=1", "app.example.com"),
        ("app.example.com:8080", "app.example.com"),
        ("user:pw@app.example.com", "app.example.com"),
        ("APP.example.com.", "app.example.com"),
        ("  app.example.com  ", "app.example.com"),
        ("10.0.0.0/24", "10.0.0.0/24"),
        ("*.example.com", "*.example.com"),
        ("http://10.1.2.3/admin", "10.1.2.3"),
    ] {
        assert_eq!(normalize_host(raw), want, "{raw}");
    }
}

#[test]
fn only_exact_hosts_wildcards_and_cidr_ranges_are_in_scope() {
    let scopes = [scope("app.example.com", &["*.api.example.com", "10.0.0.0/24"], &[])];
    for (host, want) in [
        ("app.example.com", true),
        ("https://app.example.com:8443/x", true),
        ("admin.app.example.com", false), // allowing a host does not allow its subdomains
        ("example.com", false),
        ("v1.api.example.com", true),
        ("deep.v1.api.example.com", true),
        ("api.example.com", false), // the wildcard does not cover its own apex
        ("evilapi.example.com", false), // lookalike without the dot boundary
        ("app.example.com.evil.net", false),
        ("10.0.0.7", true),
        ("10.0.1.7", false),
        ("", false),
    ] {
        assert_eq!(in_scope(host, &scopes), want, "{host}");
    }
}

#[test]
fn deny_always_wins_and_no_records_means_nothing_is_in_scope() {
    let scopes = [scope("app.example.com", &["*.example.com"], &["staging.example.com", "10.0.0.5/32"])];
    assert!(in_scope("shop.example.com", &scopes));
    assert!(!in_scope("staging.example.com", &scopes), "denied despite the wildcard");
    assert!(!in_scope("anything.example.com", &[]), "no scope record");
    let cidr = [scope("10.0.0.0/24", &[], &["10.0.0.5/32"])];
    assert!(in_scope("10.0.0.4", &cidr) && !in_scope("10.0.0.5", &cidr));
}

#[test]
fn a_scope_records_host_is_an_allow_entry_and_other_rows_are_skipped() {
    let s = shelf();
    put(&s, "put_scope", json!({ "key": "scope/gate", "host": "App.Example.com", "allow": ["*.api.example.com"], "deny": ["x.example.com"], "expected_revision": 0 }));
    put(&s, "put", json!({ "key": "notes/scope-ish", "content": "{\"kind\":\"note\"}", "expected_revision": 0 }));
    let mut entries = s.scan("scope/").expect("scan");
    entries.extend(s.scan("notes/").expect("scan"));
    let scopes = parse_scopes(&entries);
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].allow, ["*.api.example.com", "app.example.com"]);
    assert_eq!(scopes[0].deny, ["x.example.com"]);
}

// ---- audit ----

#[test]
fn claims_without_any_scope_record_get_one_warning() {
    let env = Env::new();
    assert!(env.findings().is_empty(), "nothing published, nothing to check");
    claim(&env.shelf, "claims/a", "app.example.com");
    let found = env.findings();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].kind, found[0].severity), (FindingKind::NoScopeRecord, Severity::Warn));
}

#[test]
fn an_out_of_scope_claim_or_asset_is_a_stop_finding_that_names_the_lane_that_published_it() {
    let mut env = Env::new();
    env.scope_record("app.example.com");
    claim(&env.shelf, "claims/ok", "app.example.com");
    claim(&env.shelf, "claims/bad", "https://other.net/admin");
    put(&env.shelf, "put_asset", json!({ "key": "assets/h", "host": "10.9.9.9", "asset_type": "host", "title": "t" }));
    env.digest.observe("red", &LaneEvent::Published { key: "claims/bad".into(), revision: 1 });

    let found: Vec<_> = env.findings().into_iter().filter(|f| f.kind == FindingKind::OutOfScope).collect();
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|f| f.severity == Severity::Stop));
    let bad = found.iter().find(|f| f.key == "out-of-scope:claims/bad").expect("claim finding");
    assert_eq!(bad.avatar.as_deref(), Some("red"));
    assert!(bad.note.contains("other.net") && bad.note.contains("claims/bad"), "{}", bad.note);
    assert_eq!(bad.refs, ["claims/bad"]);
    let asset = found.iter().find(|f| f.key == "out-of-scope:assets/h").expect("asset finding");
    assert_eq!(asset.avatar, None, "no lane is known to have published it");
}

#[test]
fn a_silent_running_lane_is_stalled_unless_paused_recent_or_finished() {
    let mut env = Env::new();
    env.digest.observe_at("red", &LaneEvent::Step(0), env.now_ms - 6 * 60_000);
    let found = env.findings();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].kind, found[0].key.as_str()), (FindingKind::Stalled, "stalled:red"));
    assert!(found[0].note.contains("6 minutes"), "{}", found[0].note);

    env.paused.insert("red".into());
    assert!(env.findings().is_empty(), "a paused lane is quiet on purpose");
    env.paused.clear();

    env.digest.observe_at("red", &LaneEvent::Step(1), env.now_ms - 60_000);
    assert!(env.findings().is_empty(), "recent activity");

    env.digest.observe_at("red", &LaneEvent::Step(2), env.now_ms - 9 * 60_000);
    env.assignments = vec![asg("red", AssignmentKind::Probe, AssignmentStatus::Completed, "scan")];
    assert!(env.findings().is_empty(), "no pending assignment, so nothing is stalled");
}

#[test]
fn a_lane_deep_into_its_work_with_nothing_published_is_flagged() {
    let mut env = Env::new();
    for step in 0..6 {
        env.digest.observe("red", &LaneEvent::Step(step));
    }
    let found = env.findings();
    assert_eq!((found.len(), found[0].kind), (1, FindingKind::Unpublished));
    assert!(found[0].note.contains("6 steps"), "{}", found[0].note);

    env.digest.observe("red", &LaneEvent::Published { key: "assets/h".into(), revision: 1 });
    assert!(env.findings().iter().all(|f| f.kind != FindingKind::Unpublished), "it shared something");
}

#[test]
fn few_steps_do_not_count_as_hoarding() {
    let mut env = Env::new();
    for step in 0..5 {
        env.digest.observe("red", &LaneEvent::Step(step));
    }
    assert!(env.findings().is_empty());
}

#[test]
fn the_same_assignment_given_three_times_is_a_loop_but_superseded_ones_do_not_count() {
    let mut env = Env::new();
    env.assignments = vec![
        asg("red", AssignmentKind::Probe, AssignmentStatus::Failed, "scan the login"),
        asg("red", AssignmentKind::Probe, AssignmentStatus::Failed, "scan the login"),
        asg("red", AssignmentKind::Probe, AssignmentStatus::Running, "scan the login"),
        asg("red", AssignmentKind::Probe, AssignmentStatus::Completed, "a different task"),
    ];
    let found = env.findings();
    assert_eq!((found.len(), found[0].kind), (1, FindingKind::RepeatedWork));
    assert!(found[0].note.contains("3 times"), "{}", found[0].note);

    env.assignments[0].status = AssignmentStatus::Superseded;
    assert!(env.findings().is_empty(), "a retried assignment is not a loop");
}

#[test]
fn findings_come_most_severe_first_in_a_stable_order() {
    let mut env = Env::new();
    env.scope_record("app.example.com");
    claim(&env.shelf, "claims/bad", "other.net");
    for step in 0..6 {
        env.digest.observe("red", &LaneEvent::Step(step));
    }
    let order: Vec<Severity> = env.findings().iter().map(|f| f.severity).collect();
    assert_eq!(order.first(), Some(&Severity::Stop));
    assert!(order.iter().skip(1).all(|severity| *severity == Severity::Warn), "{order:?}");
}

#[test]
fn a_claim_with_no_evidence_is_flagged_until_evidence_is_attached() {
    let env = Env::new();
    env.scope_record("app.example.com");
    claim(&env.shelf, "claims/a", "app.example.com");
    assert!(env.findings().iter().any(|f| f.kind == FindingKind::MissingEvidence && f.key == "no-evidence:claims/a"));

    put(&env.shelf, "put_evidence", json!({ "key": "evidence/real", "claim_key": "claims/a", "evidence_kind": "http", "body": "the response", "expected_revision": 0 }));
    assert!(env.findings().iter().all(|f| f.kind != FindingKind::MissingEvidence));
}

#[test]
fn two_claims_for_the_same_host_and_class_that_disagree_are_a_contradiction() {
    let env = Env::new();
    env.scope_record("app.example.com");
    put(&env.shelf, "put_claim", json!({ "key": "claims/a", "vulnclass": "cors", "name": "n", "severity": "low",
        "summary": "s", "host": "app.example.com", "poc": "curl", "expected_revision": 0 }));
    put(&env.shelf, "put_claim", json!({ "key": "claims/b", "vulnclass": "cors", "name": "n", "severity": "high",
        "summary": "s", "host": "https://App.Example.com/x", "poc": "curl", "expected_revision": 0 }));
    let found = env.findings();
    let hit = found.iter().find(|f| f.kind == FindingKind::Contradiction).expect("contradiction");
    assert!(hit.note.contains("low") && hit.note.contains("high"), "{}", hit.note);
}

// ---- trigger ----

fn run(t: Tick) -> Pass {
    match t {
        Tick::Run(pass) => pass,
        other => panic!("expected a pass, got {other:?}"),
    }
}

#[test]
fn there_is_nothing_to_watch_without_active_work() {
    let mut env = Env::new();
    env.assignments.clear();
    assert_eq!(Clerk::new().tick(&env.input()).expect("tick"), Tick::Idle);
}

#[test]
fn a_pass_runs_once_per_new_finding_and_a_quiet_room_costs_nothing() {
    let mut env = Env::new();
    env.scope_record("app.example.com");
    claim(&env.shelf, "claims/bad", "other.net");
    let mut clerk = Clerk::new();

    let pass = run(clerk.tick(&env.input()).expect("tick"));
    assert!(pass.new_findings.iter().any(|f| f.kind == FindingKind::OutOfScope));
    assert!(pass.new_findings.iter().any(|f| f.kind == FindingKind::MissingEvidence));
    assert!(clerk.is_busy());
    assert_eq!(clerk.tick(&env.input()).expect("tick"), Tick::Busy, "never two passes at once");

    clerk.finish_pass();
    assert_eq!(clerk.tick(&env.input()).expect("tick"), Tick::Quiet, "same state, same finding: nothing new");
    env.now_ms += 10 * 60_000;
    assert_eq!(clerk.tick(&env.input()).expect("tick"), Tick::Quiet, "time alone does not start a pass");
}

#[test]
fn a_shelf_change_starts_a_pass_but_not_faster_than_the_minimum_gap() {
    let mut env = Env::new();
    env.scope_record("app.example.com");
    let mut clerk = Clerk::new();
    run(clerk.tick(&env.input()).expect("first fingerprint"));
    clerk.finish_pass();

    put(&env.shelf, "put_asset", json!({ "key": "assets/h", "host": "app.example.com", "asset_type": "host", "title": "app" }));
    env.now_ms += 5_000;
    assert_eq!(clerk.tick(&env.input()).expect("tick"), Tick::Quiet, "changed, but too soon");
    env.now_ms += MIN_PASS_GAP_MS;
    let pass = run(clerk.tick(&env.input()).expect("tick"));
    assert!(pass.new_findings.is_empty(), "an in-scope asset is a shelf change, not a finding");
}

#[test]
fn a_cleared_finding_is_reported_again_if_it_returns() {
    let mut env = Env::new();
    env.scope_record("app.example.com");
    claim(&env.shelf, "claims/bad", "other.net");
    let mut clerk = Clerk::new();
    run(clerk.tick(&env.input()).expect("tick"));
    clerk.finish_pass();

    // the claim is fixed: it now points at an in-scope host
    put(&env.shelf, "put_claim", json!({ "key": "claims/bad", "vulnclass": "cors", "name": "n", "severity": "low",
        "summary": "s", "host": "app.example.com", "poc": "curl", "expected_revision": 1 }));
    env.now_ms += MIN_PASS_GAP_MS * 2;
    run(clerk.tick(&env.input()).expect("the change starts a pass"));
    clerk.finish_pass();

    put(&env.shelf, "put_claim", json!({ "key": "claims/bad", "vulnclass": "cors", "name": "n", "severity": "low",
        "summary": "s", "host": "other.net", "poc": "curl", "expected_revision": 2 }));
    env.now_ms += MIN_PASS_GAP_MS * 2;
    let again = run(clerk.tick(&env.input()).expect("tick"));
    assert_eq!(again.new_findings.len(), 1, "the returning problem is new again");
}

#[test]
fn a_finding_that_appears_while_a_pass_runs_is_not_lost() {
    let env = Env::new();
    env.scope_record("app.example.com");
    let mut clerk = Clerk::new();
    run(clerk.tick(&env.input()).expect("tick"));
    claim(&env.shelf, "claims/bad", "other.net");
    assert_eq!(clerk.tick(&env.input()).expect("tick"), Tick::Busy);
    clerk.finish_pass();
    let pass = run(clerk.tick(&env.input()).expect("tick"));
    assert!(pass.new_findings.iter().any(|f| f.kind == FindingKind::OutOfScope));
}

#[test]
fn the_brief_lists_findings_marks_the_new_ones_and_includes_the_lanes() {
    let registry = RegistryHandle::embedded().expect("registry").current();
    let f = |key: &str, severity| Finding { key: key.into(), kind: FindingKind::Stalled, avatar: None, severity, note: format!("note {key}"), refs: vec![] };
    let pass = Pass { new_findings: vec![f("b", Severity::Warn)], all_findings: vec![f("a", Severity::Stop), f("b", Severity::Warn)] };
    let brief = Clerk::brief(&registry, &pass, "Work lanes:\n  red: running (step 1)\n").expect("brief");
    assert!(brief.contains("- [stop] note a\n"), "{brief}");
    assert!(brief.contains("- [warn] [new] note b"), "{brief}");
    assert!(brief.contains("red: running (step 1)") && !brief.contains("{{"), "{brief}");

    let empty = Clerk::brief(&registry, &Pass { new_findings: vec![], all_findings: vec![] }, "").expect("brief");
    assert!(empty.contains("- nothing") && empty.contains("(no lanes yet)"), "{empty}");
}

// ---- clerk_flag ----

fn flag_args(avatar: &str) -> Value {
    json!({ "avatar": avatar, "severity": "warn", "note": "red is off scope", "refs": ["claims/bad"] })
}

#[test]
fn a_flag_is_recorded_on_the_shelf_and_reaches_the_chief_and_nobody_else() {
    let registry = RegistryHandle::embedded().expect("registry").current();
    let s = shelf();
    let mut clerk = Clerk::new();
    let (value, item) = clerk.flag(&registry, &s, &flag_args("red"), 42).expect("flag");
    assert_eq!(value["ok"], true);
    assert_eq!(
        item,
        InboxItem::ClerkFlag { avatar: "red".into(), severity: "warn".into(), note: "red is off scope (see claims/bad)".into() }
    );
    let recorded = value["recorded"].as_str().expect("key").to_string();
    assert!(recorded.starts_with("notes/clerk/flag-42-"), "{recorded}");
    let row = s.read_item(&recorded).expect("audit note");
    let body: Value = serde_json::from_str(row.content.as_deref().expect("content")).expect("json");
    assert_eq!((body["avatar"].as_str(), body["severity"].as_str()), (Some("red"), Some("warn")));

    let (second, _) = clerk.flag(&registry, &s, &flag_args("reviewer"), 42).expect("second");
    assert_ne!(second["recorded"], value["recorded"], "two flags in one millisecond keep distinct notes");
}

#[test]
fn a_flag_names_a_specialist_lane_and_nothing_else() {
    let registry = RegistryHandle::embedded().expect("registry").current();
    let s = shelf();
    let mut clerk = Clerk::new();
    let mut call = |args: Value| clerk.flag(&registry, &s, &args, 1).expect_err("rejected");
    assert!(call(flag_args("chief")).contains("specialist lanes only"));
    assert!(call(flag_args("clerk")).contains("specialist lanes only"));
    assert!(call(flag_args("ghost")).contains("specialist lanes only"));
    assert!(call(json!({ "avatar": "red", "severity": "panic", "note": "x" })).contains("severity must be"));
    assert!(call(json!({ "avatar": "red", "severity": "warn", "note": "   " })).contains("note must be"));
    assert!(call(json!({ "avatar": "red", "severity": "warn", "note": "x", "refs": vec!["k"; 17] })).contains("refs must be"));
    assert!(call(json!({ "avatar": "red", "severity": "warn", "note": "x", "to": "user" })).contains("invalid arguments"), "no other recipient exists");
    assert!(call(json!("red")).contains("invalid arguments"));
    assert!(s.scan("notes/clerk/").expect("scan").is_empty(), "a rejected flag leaves no note behind");
}

#[test]
fn repeated_superseded_verification_is_a_review_loop() {
    let mut env = Env::new();
    for i in 0..3 {
        env.assignments.push(asg("reviewer", AssignmentKind::Verification, AssignmentStatus::Superseded, &format!("round {i}")));
    }
    let found = env.findings();
    assert!(found.iter().any(|f| f.kind == FindingKind::ReviewLoop && f.key == "review-loop:reviewer"));
    env.assignments.retain(|a| a.kind != AssignmentKind::Verification);
    assert!(env.findings().iter().all(|f| f.kind != FindingKind::ReviewLoop));
}

#[test]
fn findings_with_no_report_under_way_are_a_handover_due() {
    let mut env = Env::new();
    claim(&env.shelf, "claims/a", "app.example.com");
    put(&env.shelf, "put_verdict", json!({ "key": "verdicts/a", "claim_key": "claims/a", "verdict": "confirmed", "expected_revision": 0,
        "claim_revision": 1, "note": "reproduced" }));
    put(&env.shelf, "put_finding", json!({ "key": "findings/a", "claim_key": "claims/a", "verdict_key": "verdicts/a",
        "vulnclass": "cors", "name": "n", "severity": "low", "summary": "s", "host": "app.example.com", "expected_revision": 0 }));
    env.assignments = vec![asg("reviewer", AssignmentKind::Verification, AssignmentStatus::Completed, "v")];
    assert!(env.findings().iter().any(|f| f.kind == FindingKind::ReportDue && f.key == "report-due"));
    // A review still running, or a report being written, is not a stall.
    env.assignments.push(asg("reviewer", AssignmentKind::Verification, AssignmentStatus::Running, "again"));
    assert!(env.findings().iter().all(|f| f.kind != FindingKind::ReportDue));
    env.assignments.pop();
    env.assignments.push(asg("reporter", AssignmentKind::Report, AssignmentStatus::Running, "write"));
    assert!(env.findings().iter().all(|f| f.kind != FindingKind::ReportDue));
}
