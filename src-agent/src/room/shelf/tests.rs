//! Ported from Orpheus `db/src/shelf.test.ts`.

use serde_json::{json, Value};

use super::*;

fn shelf() -> Shelf {
    Shelf::open_in_memory().expect("in-memory shelf")
}

fn claim(key: &str) -> Value {
    json!({
        "key": key, "vulnclass": "cors", "name": "CORS", "severity": "high",
        "summary": "reflects origin", "host": "app.example", "poc": "curl",
        "expected_revision": 0
    })
}

fn finding(key: &str, claim_key: &str, verdict_key: &str) -> Value {
    json!({
        "key": key, "claim_key": claim_key, "verdict_key": verdict_key,
        "vulnclass": "cors", "name": "CORS", "severity": "high",
        "summary": "confirmed", "host": "app.example", "expected_revision": 0
    })
}

fn verdict(key: &str, claim_key: &str, verdict: &str, revision: u64) -> Value {
    json!({
        "key": key, "claim_key": claim_key, "claim_revision": 1,
        "verdict": verdict, "note": "n", "expected_revision": revision
    })
}

fn content(shelf: &Shelf, key: &str) -> Value {
    let row = shelf.call("get", &json!({ "key": key })).expect("get");
    serde_json::from_str(row["content"].as_str().expect("content")).expect("json content")
}

fn err(result: Result<Value, ShelfError>) -> String {
    result.expect_err("expected a shelf error").0
}

#[test]
fn paginates_list_with_the_after_cursor_past_the_page() {
    let s = shelf();
    for i in 0..101 {
        s.call("put", &json!({ "key": format!("notes/note-{i:03}"), "content": "x", "expected_revision": 0 }))
            .expect("put");
    }
    let first = s.call("list", &json!({})).expect("list");
    assert_eq!(first["items"].as_array().expect("items").len(), 100);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_after"], "notes/note-099");
    let second = s.call("list", &json!({ "after": "notes/note-099" })).expect("list");
    assert_eq!(second["items"].as_array().expect("items").len(), 1);
    assert_eq!(second["items"][0]["key"], "notes/note-100");
    assert_eq!(second["truncated"], false);
    assert!(second["next_after"].is_null());
}

#[test]
fn prefixed_list_does_not_leak_other_prefixes() {
    let s = shelf();
    for i in 0..101 {
        let mut args = claim(&format!("claims/note-{i:03}"));
        args["name"] = json!(format!("note-{i:03}"));
        s.call("put_claim", &args).expect("claim");
    }
    for i in 0..20 {
        s.call(
            "put_evidence",
            &json!({
                "key": format!("evidence/zz-{i:03}"), "claim_key": "claims/note-000",
                "evidence_kind": "http", "body": "seed", "expected_revision": 0
            }),
        )
        .expect("evidence");
    }
    let first = s.call("list", &json!({ "prefix": "claims/" })).expect("list");
    assert_eq!(first["items"].as_array().expect("items").len(), 100);
    assert_eq!(first["next_after"], "claims/note-099");
    let second = s.call("list", &json!({ "prefix": "claims/", "after": "claims/note-099" })).expect("list");
    let items = second["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["key"], "claims/note-100");
    assert_eq!(items[0]["revision"], 1);
    assert_eq!(second["truncated"], false);
}

#[test]
fn reports_dropped_trajectory_entries_and_dangling_edges() {
    let s = shelf();
    let first = s
        .call(
            "put_trajectory",
            &json!({
                "key": "trajectory/gate", "goal_key": "scope/gate",
                "nodes": [
                    { "id": "traj:start", "kind": "start", "label": "start" },
                    { "kind": "fact", "label": "no id" },
                    { "id": "traj:ok", "kind": "fact", "label": "fine" }
                ],
                "edges": [{ "from": "traj:start", "to": "traj:ok", "rel": "spawns" }]
            }),
        )
        .expect("first");
    assert_eq!(first["dropped"], json!(["node[1]: missing id"]));
    assert!(first.get("dangling_edges").is_none());

    let second = s
        .call(
            "put_trajectory",
            &json!({
                "key": "trajectory/gate", "goal_key": "scope/gate",
                "nodes": [{ "id": "traj:other", "kind": "fact" }],
                "edges": [{ "source": "traj:ok", "target": "traj:other", "relation": "relates" }, { "to": "traj:ok" }]
            }),
        )
        .expect("second");
    assert_eq!(second["dropped"], json!(["edge[1]: missing from/to endpoint"]));
    assert!(second.get("dangling_edges").is_none());

    let third = s
        .call(
            "put_trajectory",
            &json!({
                "key": "trajectory/gate", "goal_key": "scope/gate", "nodes": [],
                "edges": [{ "from": "traj:ok", "to": "scope/gate", "rel": "checks" }]
            }),
        )
        .expect("third");
    assert_eq!(third["dangling_edges"], json!(["traj:ok→scope/gate"]));
}

#[test]
fn accepts_trajectory_entries_as_objects_or_json_strings() {
    let s = shelf();
    let pretty = serde_json::to_string_pretty(&json!({ "id": "traj:fact", "kind": "fact", "label": "line\nbreak" }))
        .expect("pretty");
    let double = serde_json::to_string(
        &serde_json::to_string(&json!({ "id": "traj:goal", "kind": "goal", "label": "goal" })).expect("inner"),
    )
    .expect("outer");
    let edges = serde_json::to_string(&json!([
        { "from": "traj:start", "to": "traj:fact", "rel": "spawns" },
        { "source": "traj:fact", "target": "traj:goal", "relation": "yields" }
    ]))
    .expect("edges");
    let written = s
        .call(
            "put_trajectory",
            &json!({
                "key": "trajectory/gate", "goal_key": "scope/gate",
                "nodes": [{ "id": "traj:start", "kind": "start", "label": "start" }, pretty, double],
                "edges": edges
            }),
        )
        .expect("write");
    assert_eq!(written["revision"], 1);
    assert!(written.get("dropped").is_none());
    let graph = content(&s, "trajectory/gate");
    let ids: Vec<&str> = graph["nodes"].as_array().expect("nodes").iter().map(|n| n["id"].as_str().expect("id")).collect();
    assert_eq!(ids, ["traj:start", "traj:fact", "traj:goal"]);
    assert_eq!(graph["nodes"][1]["label"], "line\nbreak");
    assert_eq!(graph["edges"].as_array().expect("edges").len(), 2);

    let broken = s
        .call(
            "put_trajectory",
            &json!({ "key": "trajectory/gate", "goal_key": "scope/gate", "nodes": ["{not json", { "kind": "fact" }], "edges": [42] }),
        )
        .expect("broken");
    assert_eq!(
        broken["dropped"],
        json!([
            "node[0]: JSON string did not parse as an object",
            "node[1]: missing id",
            "edge[0]: not an object"
        ])
    );
}

#[test]
fn merges_trajectory_from_parallel_writers_without_a_conflict() {
    let s = shelf();
    let first = s
        .call("put_trajectory", &json!({ "key": "trajectory/gate", "goal_key": "gate", "nodes": [{ "id": "n1", "label": "recon", "kind": "fact" }], "edges": [] }))
        .expect("first");
    assert_eq!(first["revision"], 1);
    let second = s
        .call(
            "put_trajectory",
            &json!({
                "key": "trajectory/gate", "goal_key": "gate",
                "nodes": [{ "id": "n2", "label": "probe", "kind": "intent" }],
                "edges": [{ "id": "e1", "from": "n1", "to": "n2", "rel": "spawns" }]
            }),
        )
        .expect("second");
    assert_eq!(second["revision"], 2);
    let graph = content(&s, "trajectory/gate");
    assert_eq!(graph["nodes"].as_array().expect("nodes").len(), 2);
    assert_eq!(graph["edges"].as_array().expect("edges").len(), 1);
    assert_eq!(graph["nodes"][0]["kind"], "fact");
    assert_eq!(graph["nodes"][1]["kind"], "intent");
}

#[test]
fn republishing_a_node_id_updates_it_in_place() {
    let s = shelf();
    for label in ["recon", "recon-done"] {
        s.call("put_trajectory", &json!({ "key": "trajectory/gate", "goal_key": "gate", "nodes": [{ "id": "n1", "label": label }], "edges": [] }))
            .expect("write");
    }
    assert_eq!(content(&s, "trajectory/gate")["nodes"], json!([{ "id": "n1", "label": "recon-done", "kind": "fact" }]));
}

#[test]
fn normalizes_edge_aliases_and_defaults_kind_to_fact() {
    let s = shelf();
    s.call(
        "put_trajectory",
        &json!({
            "key": "trajectory/gate", "goal_key": "gate",
            "nodes": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B", "kind": "goal" }],
            "edges": [{ "source": "a", "target": "b", "relation": "Spawns" }]
        }),
    )
    .expect("write");
    let graph = content(&s, "trajectory/gate");
    assert_eq!(graph["nodes"][0]["kind"], "fact");
    assert_eq!(graph["nodes"][1]["kind"], "goal");
    assert_eq!(graph["edges"][0]["from"], "a");
    assert_eq!(graph["edges"][0]["to"], "b");
    assert_eq!(graph["edges"][0]["rel"], "spawns");
}

#[test]
fn merges_asset_fields_without_a_revision() {
    let s = shelf();
    s.call("put_asset", &json!({ "key": "assets/app.example", "host": "app.example", "asset_type": "host", "title": "App host", "status": "discovered", "tech": ["nginx"] }))
        .expect("first");
    let second = s
        .call(
            "put_asset",
            &json!({
                "key": "assets/app.example", "host": "app.example", "asset_type": "host",
                "title": "App host (verified)", "status": "mapped", "tech": ["react"],
                "paths": ["/api"], "tags": ["spa"]
            }),
        )
        .expect("second");
    assert_eq!(second["revision"], 2);
    let asset = content(&s, "assets/app.example");
    assert_eq!(asset["status"], "mapped");
    assert_eq!(asset["tech"], json!(["nginx", "react"]));
    assert_eq!(asset["paths"], json!(["/api"]));
    assert_eq!(asset["tags"], json!(["spa"]));
    assert_eq!(asset["title"], "App host (verified)");
}

#[test]
fn writes_scope_then_a_claim_then_lists_both() {
    let s = shelf();
    let scope = s.call("put_scope", &json!({ "key": "scope/gate", "host": "app.example", "expected_revision": 0 })).expect("scope");
    assert_eq!((scope["key"].as_str(), scope["revision"].as_i64()), (Some("scope/gate"), Some(1)));
    let claim = s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    assert_eq!(claim["revision"], 1);
    let all = s.list_items(&[], false, 100).expect("list");
    let keys: Vec<&str> = all.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["claims/gate/cors", "scope/gate"]);
    let bulk = s.list_items(&["claims/", "scope/"], true, 250).expect("bulk");
    assert_eq!(bulk.len(), 2);
    assert!(bulk.iter().all(|e| e.content.is_some()));
}

#[test]
fn rejects_evidence_without_a_claim_then_links_it_once_the_claim_exists() {
    let s = shelf();
    let evidence = json!({
        "key": "evidence/gate/cors", "claim_key": "claims/gate/cors",
        "evidence_kind": "http", "body": "ACAO: evil", "expected_revision": 0
    });
    assert!(err(s.call("put_evidence", &evidence)).contains("put_claim first"));
    s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    s.call("put_evidence", &evidence).expect("evidence");
    assert_eq!(content(&s, "claims/gate/cors")["evidence_keys"], json!(["evidence/gate/cors"]));
}

#[test]
fn a_reviewer_s_evidence_does_not_rewrite_the_claim_it_is_checking() {
    let s = shelf();
    s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    let before = s.call("get", &json!({ "key": "claims/gate/cors" })).expect("get")["revision"].as_i64();
    let evidence = json!({
        "key": "evidence/reviewer/cors", "claim_key": "claims/gate/cors",
        "evidence_kind": "http", "body": "re-read", "expected_revision": 0
    });
    let reviewer: Vec<String> = ["get", "list", "put_evidence", "put_verdict"].iter().map(|s| s.to_string()).collect();
    s.call_as(&reviewer, "put_evidence", &evidence).expect("reviewer evidence");
    let after = s.call("get", &json!({ "key": "claims/gate/cors" })).expect("get")["revision"].as_i64();
    assert_eq!(before, after, "the claim's revision must not move under a review");
    assert_eq!(content(&s, "claims/gate/cors")["evidence_keys"], json!([]));
    // A role that owns claims still lists its evidence on the claim.
    let red: Vec<String> = ["put_claim", "put_evidence"].iter().map(|s| s.to_string()).collect();
    let own = json!({ "key": "evidence/gate/own", "claim_key": "claims/gate/cors", "evidence_kind": "http", "body": "x", "expected_revision": 0 });
    s.call_as(&red, "put_evidence", &own).expect("red evidence");
    assert_eq!(content(&s, "claims/gate/cors")["evidence_keys"], json!(["evidence/gate/own"]));
}

#[test]
fn rejects_claim_evidence_keys_that_do_not_exist_yet() {
    let s = shelf();
    let mut args = claim("claims/gate/cors");
    args["evidence_keys"] = json!(["evidence/gate/cors"]);
    assert!(err(s.call("put_claim", &args)).contains("evidence_keys entry missing"));
}

#[test]
fn a_finding_needs_a_confirmed_verdict_first() {
    let s = shelf();
    s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    let f = finding("findings/gate/cors", "claims/gate/cors", "verdicts/gate/cors");
    assert!(err(s.call("put_finding", &f)).contains("put_verdict confirmed first"));

    s.call("put_verdict", &verdict("verdicts/gate/cors", "claims/gate/cors", "returned", 0)).expect("returned");
    assert!(err(s.call("put_finding", &f)).contains("verdict=confirmed"));

    s.call("put_verdict", &verdict("verdicts/gate/cors", "claims/gate/cors", "confirmed", 1)).expect("confirmed");
    let written = s.call("put_finding", &f).expect("finding");
    assert_eq!((written["key"].as_str(), written["revision"].as_i64()), (Some("findings/gate/cors"), Some(1)));
    let keys: Vec<String> = s.list_items(&[], false, 100).expect("list").into_iter().map(|e| e.key).collect();
    assert!(!keys.iter().any(|k| k.starts_with("reports/")), "findings never mint reports");
}

#[test]
fn a_verdict_for_a_different_claim_does_not_unlock_a_finding() {
    let s = shelf();
    s.call("put_claim", &claim("claims/a")).expect("claim a");
    s.call("put_claim", &claim("claims/b")).expect("claim b");
    s.call("put_verdict", &verdict("verdicts/b", "claims/b", "confirmed", 0)).expect("verdict");
    let f = finding("findings/a", "claims/a", "verdicts/b");
    assert!(err(s.call("put_finding", &f)).contains("must equal finding claim_key"));
}

#[test]
fn rejects_a_second_writer_that_skipped_get() {
    let s = shelf();
    let scope = json!({ "key": "scope/gate", "host": "app.example", "expected_revision": 0 });
    s.call("put_scope", &scope).expect("first");
    assert!(err(s.call("put_scope", &scope)).contains("Revision conflict"));
}

#[test]
fn a_failed_write_leaves_no_partial_state() {
    let s = shelf();
    s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    // Evidence links onto the claim after the write; a conflicting second put must not bump the claim.
    let evidence = json!({ "key": "evidence/e1", "claim_key": "claims/gate/cors", "evidence_kind": "http", "body": "b", "expected_revision": 0 });
    s.call("put_evidence", &evidence).expect("first evidence");
    let claim_rev = s.call("get", &json!({ "key": "claims/gate/cors" })).expect("get")["revision"].as_i64();
    assert!(s.call("put_evidence", &evidence).is_err());
    let after = s.call("get", &json!({ "key": "claims/gate/cors" })).expect("get")["revision"].as_i64();
    assert_eq!(claim_rev, after);
}

#[test]
fn validates_arguments_and_keys() {
    let s = shelf();
    assert!(err(s.call("get", &json!("nope"))).contains("must be an object"));
    assert!(err(s.call("get", &json!({ "key": "../etc" }))).contains("Key must be"));
    assert!(err(s.call("get", &json!({ "key": "notes/missing" }))).contains("Item not found"));
    assert!(err(s.call("put", &json!({ "key": "claims/x", "content": "x", "expected_revision": 0 }))).contains("Free put only allows"));
    assert!(err(s.call("put_claim", &json!({ "key": "notes/x" }))).contains("must start with claims/"));
    let mut bad = claim("claims/x");
    bad["severity"] = json!("catastrophic");
    assert!(err(s.call("put_claim", &bad)).contains("severity must be"));
    assert!(err(s.call("put", &json!({ "key": "notes/x", "content": "x" }))).contains("expected_revision is required"));
    assert!(err(s.call("nope", &json!({}))).contains("Unknown tool"));
    let big = "x".repeat(129 * 1024);
    assert!(err(s.call("put", &json!({ "key": "notes/big", "content": big, "expected_revision": 0 }))).contains("128 KiB"));
}

#[test]
fn call_as_enforces_the_role_allowlist() {
    let s = shelf();
    let clerk: Vec<String> = ["get", "list", "put_asset", "put_trajectory"].iter().map(|s| s.to_string()).collect();
    assert!(err(s.call_as(&clerk, "put_claim", &claim("claims/x"))).contains("not available to this role"));
    assert!(s.call_as(&clerk, "list", &json!({})).is_ok());
}

#[test]
fn shelf_survives_reopening_the_file() {
    let dir = std::env::temp_dir().join(format!("koma-shelf-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("room.sqlite");
    {
        let s = Shelf::open(&path).expect("open");
        s.call("put_scope", &json!({ "key": "scope/gate", "host": "app.example", "expected_revision": 0 })).expect("write");
    }
    let s = Shelf::open(&path).expect("reopen");
    assert_eq!(s.read_item("scope/gate").expect("read").revision, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reporter_dual_pack_needs_technical_per_finding_then_manager_coverage() {
    let s = shelf();
    s.call("put_claim", &claim("claims/gate/cors")).expect("claim");
    s.call("put_verdict", &verdict("verdicts/gate/cors", "claims/gate/cors", "confirmed", 0)).expect("verdict");
    s.call("put_finding", &finding("findings/gate/cors", "claims/gate/cors", "verdicts/gate/cors")).expect("finding");

    let gap = s.assert_reporter_pack_complete().expect_err("no reports yet").0;
    assert!(gap.contains("missing audience=technical"));

    let technical = |markdown: String, revision: u64| {
        json!({
            "key": "reports/gate/cors", "audience": "technical", "finding_key": "findings/gate/cors",
            "title": "CORS tech", "markdown": markdown, "expected_revision": revision
        })
    };
    s.call("put_report", &technical("x".repeat(50), 0)).expect("thin");
    assert!(s.assert_reporter_pack_complete().expect_err("thin").0.contains("too thin"));

    s.call("put_report", &technical(format!("# CORS detail\n\n{}", "detail ".repeat(80)), 1)).expect("detailed");
    assert!(s.assert_reporter_pack_complete().expect_err("no manager").0.contains("audience=manager"));

    s.call(
        "put_report",
        &json!({
            "key": "reports/gate/executive", "audience": "manager", "finding_key": "findings/gate/cors",
            "finding_keys": ["findings/gate/cors"], "title": "Executive", "go_no_go": "no-go",
            "severity_summary": "high CORS", "markdown": "# Executive\n\nDo not ship until CORS is fixed.\n",
            "expected_revision": 0
        }),
    )
    .expect("manager");
    s.assert_reporter_pack_complete().expect("pack complete");
}

#[test]
fn no_findings_means_no_report_requirement() {
    shelf().assert_reporter_pack_complete().expect("nothing to report on");
}

#[test]
fn verify_references_requires_exact_revisions_except_for_living_records() {
    use crate::room::types::Reference;
    let r = |key: &str, revision: u64| Reference { key: key.into(), revision };
    let s = shelf();
    s.call("put_claim", &claim("claims/a")).expect("claim"); // claims/a rev 1
    s.call("put_asset", &json!({ "key": "assets/h", "host": "h", "asset_type": "host", "title": "t" })).expect("asset 1");
    s.call("put_asset", &json!({ "key": "assets/h", "host": "h", "asset_type": "host", "title": "t2" })).expect("asset 2");

    s.verify_references(&[r("claims/a", 1)]).expect("exact");
    s.verify_references(&[]).expect("nothing cited");
    let moved = s.verify_references(&[r("claims/a", 2)]).expect_err("future revision").0;
    assert!(moved.contains("changed or missing: claims/a"));
    s.call("put_claim", &{ let mut c = claim("claims/a"); c["expected_revision"] = json!(1); c }).expect("claim rev 2");
    assert!(s.verify_references(&[r("claims/a", 1)]).is_err(), "a claim that moved on is stale");

    s.verify_references(&[r("assets/h", 1)]).expect("a newer asset revision still covers");
    s.verify_references(&[r("assets/h", 2)]).expect("exact asset");
    assert!(s.verify_references(&[r("assets/h", 3)]).is_err());
    assert!(s.verify_references(&[r("claims/missing", 1)]).is_err());
}

#[test]
fn announces_committed_writes_including_the_claim_an_evidence_row_links_to() {
    let s = shelf();
    let rx = s.subscribe();
    s.call("put_claim", &claim("claims/a")).expect("claim");
    s.call("put_evidence", &json!({ "key": "evidence/e", "claim_key": "claims/a", "evidence_kind": "http", "body": "b", "expected_revision": 0 }))
        .expect("evidence");
    let seen: Vec<(String, i64)> = rx.try_iter().map(|e| (e.key, e.revision)).collect();
    assert_eq!(
        seen,
        [("claims/a".into(), 1), ("evidence/e".into(), 1), ("claims/a".into(), 2)]
    );
    // Reads and failed writes announce nothing.
    s.call("get", &json!({ "key": "claims/a" })).expect("get");
    assert!(s.call("put_claim", &claim("claims/a")).is_err());
    assert_eq!(rx.try_iter().count(), 0);
}

#[test]
fn a_dropped_subscriber_does_not_break_writes() {
    let s = shelf();
    drop(s.subscribe());
    let kept = s.subscribe();
    s.call("put_scope", &json!({ "key": "scope/x", "host": "h", "expected_revision": 0 })).expect("write");
    assert_eq!(kept.try_iter().count(), 1);
}

#[test]
fn concurrent_merge_writers_never_lose_nodes() {
    use std::sync::Arc;
    let s = Arc::new(shelf());
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                for i in 0..10 {
                    s.call(
                        "put_trajectory",
                        &json!({ "key": "trajectory/gate", "goal_key": "g", "nodes": [{ "id": format!("t{t}-n{i}") }], "edges": [] }),
                    )
                    .expect("merge write");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
    let row = s.call("get", &json!({ "key": "trajectory/gate" })).expect("get");
    assert_eq!(row["revision"], 80, "every merge bumped the revision exactly once");
    assert_eq!(content(&s, "trajectory/gate")["nodes"].as_array().expect("nodes").len(), 80);
}

#[test]
fn concurrent_compare_and_swap_lets_exactly_one_writer_win() {
    use std::sync::Arc;
    let s = Arc::new(shelf());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || s.call("put_scope", &json!({ "key": "scope/gate", "host": "h", "expected_revision": 0 })).is_ok())
        })
        .collect();
    let wins = handles.into_iter().map(|h| h.join().expect("thread")).filter(|ok| *ok).count();
    assert_eq!(wins, 1);
    assert_eq!(s.read_item("scope/gate").expect("read").revision, 1);
}
