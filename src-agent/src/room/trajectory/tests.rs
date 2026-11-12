//! Ported from Orpheus `shelf-status.test.ts`, `trajectory-graph.test.ts` and
//! `trajectory-layout.test.ts`, plus tests for the renderers and the projection.

use serde_json::{json, Value};

use super::gate::{self, apply_gate_statuses, display_status_for_verdict, gate_status_for_claim, Counts, Gate};
use super::graph::{merge_with_shelf, normalize_edge, parse_trajectory, synthesize, Edge, Graph, Node};
use super::layout::layout;
use super::render::{dot, glyph, json as render_json, layered, mermaid, outline};
use super::view::{build, project};
use super::{render_all, Format};
use crate::room::shelf::{Shelf, ShelfEntry};

fn row(key: &str, content: Value) -> ShelfEntry {
    ShelfEntry { key: key.into(), revision: 1, updated: 0, content: Some(content.to_string()) }
}

fn bare(key: &str) -> ShelfEntry {
    ShelfEntry { key: key.into(), revision: 1, updated: 0, content: None }
}

fn node(id: &str, kind: &str) -> Node {
    Node::new(id, id, kind)
}

fn edge(from: &str, to: &str, rel: &str) -> Edge {
    Edge { from: from.into(), to: to.into(), rel: Some(rel.into()) }
}

fn counts(items: &[ShelfEntry]) -> Counts {
    gate::count(items)
}

fn c(open: usize, verified: usize, returned: usize, reports: usize) -> Counts {
    Counts { open, verified, returned, reports }
}

// ---- gate: counts and display ----

#[test]
fn tallies_claim_and_finding_keys_separately() {
    let items = [bare("claims/a"), bare("claims/b"), row("findings/b", json!({ "claim_key": "claims/b" })), bare("notes/surface")];
    assert_eq!(counts(&items), c(1, 1, 0, 0));
}

#[test]
fn counts_returned_claims_from_content_status() {
    let items = [
        row("claims/x", json!({ "kind": "claim", "status": "returned", "name": "CORS" })),
        row("findings/y", json!({ "kind": "finding", "name": "HSTS", "severity": "medium" })),
    ];
    assert_eq!(counts(&items), c(0, 1, 1, 0));
}

#[test]
fn counts_claims_from_keys_alone_and_reports_under_reports() {
    let keys = [bare("claims/a/cors"), bare("claims/a/hsts"), row("findings/a/cors", json!({ "claim_key": "claims/a/cors" })), bare("assets/x")];
    assert_eq!(counts(&keys), c(1, 1, 0, 0));
    let reports = [bare("findings/a/cors"), bare("reports/a/executive"), bare("reports/a/cors-tech")];
    assert_eq!(counts(&reports), c(0, 1, 0, 2));
}

#[test]
fn a_confirmed_verdict_without_a_finding_is_not_verified() {
    let items = [
        row("claims/koma/missing-hsts", json!({ "kind": "claim", "name": "missing-hsts", "severity": "low", "status": "open" })),
        row("verdicts/koma/missing-hsts", json!({ "kind": "verdict", "claim_key": "claims/koma/missing-hsts", "verdict": "confirmed", "note": "reproduced" })),
    ];
    assert_eq!(counts(&items), c(1, 0, 0, 0));
}

#[test]
fn a_claim_leaves_open_once_its_finding_exists() {
    let items = [
        row("claims/koma/m", json!({ "kind": "claim", "name": "m", "severity": "low" })),
        row("verdicts/koma/m", json!({ "kind": "verdict", "claim_key": "claims/koma/m", "verdict": "confirmed" })),
        row("findings/koma/m", json!({ "kind": "finding", "claim_key": "claims/koma/m", "name": "m" })),
        row("reports/koma/m", json!({ "kind": "report", "title": "m", "generated": true })),
    ];
    assert_eq!(counts(&items), c(0, 1, 0, 1));
}

#[test]
fn a_discarded_verdict_counts_as_returned_even_if_the_claim_still_says_open() {
    let items = [
        row("claims/koma/noise", json!({ "kind": "claim", "name": "noise", "status": "open" })),
        row("verdicts/koma/noise", json!({ "kind": "verdict", "claim_key": "claims/koma/noise", "verdict": "discarded" })),
    ];
    assert_eq!(counts(&items), c(0, 0, 1, 0));
}

#[test]
fn a_returned_verdict_for_a_missing_claim_or_naming_none_still_counts_once() {
    let orphan = [row("verdicts/v", json!({ "claim_key": "claims/gone", "verdict": "returned" }))];
    assert_eq!(counts(&orphan), c(0, 0, 1, 0));
    let nameless = [row("verdicts/v", json!({ "verdict": "dismissed" }))];
    assert_eq!(counts(&nameless), c(0, 0, 1, 0));
}

#[test]
fn a_claim_stays_open_while_its_verdict_is_only_confirmed() {
    let items = [
        row("claims/koma/csp", json!({ "kind": "claim", "name": "csp", "severity": "low" })),
        row("verdicts/koma/csp", json!({ "kind": "verdict", "claim_key": "claims/koma/csp", "verdict": "confirmed" })),
    ];
    let idx = gate::index(&items);
    assert_eq!(gate_status_for_claim("claims/koma/csp", &idx, None), Gate::Open);
    assert_eq!(display_status_for_verdict("confirmed", Some("claims/koma/csp"), &idx), "review-ok");
}

#[test]
fn a_claim_is_verified_only_after_its_finding() {
    let items = [
        row("claims/koma/x", json!({})),
        row("verdicts/koma/x", json!({ "claim_key": "claims/koma/x", "verdict": "confirmed" })),
        row("findings/koma/x", json!({ "claim_key": "claims/koma/x" })),
    ];
    let idx = gate::index(&items);
    assert_eq!(gate_status_for_claim("claims/koma/x", &idx, None), Gate::Verified);
    assert_eq!(display_status_for_verdict("confirmed", Some("claims/koma/x"), &idx), "verified");
    assert_eq!(display_status_for_verdict("returned", Some("claims/koma/x"), &idx), "returned");
    assert_eq!(display_status_for_verdict("confirmed", None, &idx), "confirmed");
    assert_eq!(display_status_for_verdict("  ", None, &idx), "verdict");
}

#[test]
fn a_bot_stamped_confirmed_is_stripped_until_the_shelf_proves_it() {
    let nodes = vec![
        Node { status: Some("confirmed".into()), severity: Some("low".into()), ..Node::new("claims/koma/missing-hsts", "missing-hsts", "claim") },
        Node { status: Some("confirmed".into()), resource_key: Some("verdicts/koma/missing-hsts".into()), ..Node::new("v1", "confirmed", "verdict") },
    ];
    let items = [
        row("claims/koma/missing-hsts", json!({ "kind": "claim", "severity": "low" })),
        row("verdicts/koma/missing-hsts", json!({ "claim_key": "claims/koma/missing-hsts", "verdict": "confirmed" })),
    ];
    let out = apply_gate_statuses(nodes, &items);
    assert_eq!(out[0].status.as_deref(), Some("open"));
    assert_eq!(out[1].status.as_deref(), Some("review-ok"));
    assert_eq!(out[1].label, "review-ok", "the label follows once it is only a status word");
}

#[test]
fn a_claim_node_the_shelf_does_not_know_loses_a_self_stamped_confirmation() {
    let out = apply_gate_statuses(vec![Node { status: Some("verified".into()), ..node("c9", "claim") }], &[]);
    assert_eq!(out[0].status.as_deref(), Some("open"));
    let untouched = apply_gate_statuses(vec![Node { status: Some("open".into()), ..node("c9", "claim") }], &[]);
    assert_eq!(untouched[0].status.as_deref(), Some("open"));
}

// ---- graph ----

fn shelf_rows() -> Vec<ShelfEntry> {
    vec![
        row("claims/retas/cors", json!({ "kind": "claim", "name": "cors-wildcard", "severity": "high" })),
        row("claims/retas/headers", json!({ "kind": "claim", "name": "missing-headers", "severity": "medium" })),
        row("verdicts/retas/cors", json!({ "kind": "verdict", "claim_key": "claims/retas/cors", "verdict": "confirmed" })),
        row("findings/retas/cors", json!({ "kind": "finding", "claim_key": "claims/retas/cors", "name": "Permissive CORS", "severity": "high" })),
        row("verdicts/retas/noise", json!({ "kind": "verdict", "claim_key": "claims/retas/headers", "verdict": "dismissed" })),
    ]
}

#[test]
fn accepts_source_target_aliases_and_lowercases_the_relation() {
    let raw = json!({ "source": "a", "target": "b", "relation": "Derives" });
    assert_eq!(normalize_edge(&raw), Some(edge("a", "b", "derives")));
    assert_eq!(normalize_edge(&json!({ "from": "a" })), None);
    assert_eq!(normalize_edge(&json!({ "from": " ", "to": "b" })), None);
    assert_eq!(normalize_edge(&json!("a->b")), None);
}

#[test]
fn connects_verdicts_and_findings_to_claims_from_the_shelf_alone() {
    let g = synthesize(&shelf_rows(), Some("trajectory/retas"));
    assert_eq!(g.goal_key, "retas");
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(g.edges.iter().all(|e| ids.contains(&e.from.as_str()) && ids.contains(&e.to.as_str())));
    assert!(g.edges.contains(&edge("claims/retas/cors", "findings/retas/cors", "proves")));
    assert!(g.edges.contains(&edge("claims/retas/cors", "verdicts/retas/cors", "checks")));
    assert!(g.edges.contains(&edge("claims/retas/headers", "verdicts/retas/noise", "checks")));
}

#[test]
fn connects_evidence_and_assets_onto_claims_from_shelf_keys() {
    let mut items = shelf_rows();
    items.push(row("assets/retas/app", json!({ "kind": "asset", "title": "app.example", "host": "app.example" })));
    items.push(row("evidence/retas/cors-headers", json!({ "kind": "evidence", "claim_key": "claims/retas/cors", "evidence_kind": "http" })));
    items[0] = row("claims/retas/cors", json!({ "kind": "claim", "name": "cors-wildcard", "severity": "high",
        "asset_keys": ["assets/retas/app"], "evidence_keys": ["evidence/retas/cors-headers"] }));
    let g = synthesize(&items, Some("trajectory/retas"));
    assert!(g.edges.contains(&edge("claims/retas/cors", "evidence/retas/cors-headers", "supports")));
    assert!(g.edges.contains(&edge("assets/retas/app", "claims/retas/cors", "hosts")));
}

#[test]
fn a_goal_defaults_to_the_scope_key_then_gate() {
    assert_eq!(synthesize(&[bare("scope/app")], None).goal_key, "app");
    assert_eq!(synthesize(&[], None).goal_key, "gate");
    assert_eq!(synthesize(&[bare("scope/app")], Some("")).goal_key, "app", "an empty goal falls through");
}

fn loaded(goal: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
    Graph { goal_key: goal.into(), nodes, edges }
}

fn status_of<'a>(g: &'a Graph, id: &str) -> Option<&'a str> {
    g.nodes.iter().find(|n| n.id == id).and_then(|n| n.status.as_deref())
}

#[test]
fn drops_broken_bot_ids_and_uses_the_shelf_spine_when_gate_records_exist() {
    let bot = loaded(
        "retas",
        vec![
            Node { status: Some("in-scope".into()), ..Node::new("start", "retas", "start") },
            Node { status: Some("confirmed".into()), severity: Some("high".into()), ..Node::new("c1", "cors-wildcard", "claim") },
            Node { status: Some("verified".into()), ..Node::new("v1", "verified", "verdict") },
            Node { status: Some("verified".into()), ..Node::new("f1", "Permissive CORS", "finding") },
            Node { status: Some("dismissed".into()), ..Node::new("v2", "dismissed", "verdict") },
        ],
        vec![edge("start", "c1", "derives")],
    );
    let g = merge_with_shelf(Some(&bot), &shelf_rows(), Some("trajectory/retas"));
    assert!(!g.nodes.iter().any(|n| ["c1", "v1", "f1"].contains(&n.id.as_str())), "short-id gate stand-ins are discarded");
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(g.edges.iter().all(|e| ids.contains(&e.from.as_str()) && ids.contains(&e.to.as_str())));
    let incoming: Vec<&str> = g.edges.iter().map(|e| e.to.as_str()).collect();
    for n in g.nodes.iter().filter(|n| n.kind != "start") {
        assert!(incoming.contains(&n.id.as_str()), "{} has no way in", n.id);
    }
    assert_eq!(status_of(&g, "claims/retas/cors"), Some("verified"));
    assert_eq!(status_of(&g, "claims/retas/headers"), Some("returned"));
}

#[test]
fn keeps_one_claim_when_the_bot_and_the_shelf_both_name_it() {
    let bot = loaded(
        "retas",
        vec![
            Node::new("start", "retas", "start"),
            Node { status: Some("open".into()), ..Node::new("missing-security-headers", "missing-headers", "claim") },
            Node { status: Some("open".into()), resource_key: Some("claims/retas/headers".into()), ..Node::new("claims/retas/headers", "missing-headers", "claim") },
        ],
        vec![edge("start", "missing-security-headers", ""), edge("start", "claims/retas/headers", "")],
    );
    let g = merge_with_shelf(Some(&bot), &shelf_rows(), Some("trajectory/retas"));
    let headers: Vec<&Node> = g.nodes.iter().filter(|n| n.kind == "claim" && n.id.contains("headers")).collect();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].id, "claims/retas/headers");
}

#[test]
fn hangs_orphans_under_start_when_only_a_bot_graph_exists() {
    let bot = loaded(
        "solo",
        vec![
            Node::new("start", "solo", "start"),
            Node { status: Some("open".into()), ..Node::new("c1", "thing", "claim") },
            Node { status: Some("open".into()), ..Node::new("v1", "open", "verdict") },
        ],
        vec![Edge { from: "start".into(), to: "c1".into(), rel: None }],
    );
    let g = merge_with_shelf(Some(&bot), &[], Some("trajectory/solo"));
    let incoming: Vec<&str> = g.edges.iter().map(|e| e.to.as_str()).collect();
    assert!(incoming.contains(&"c1") && incoming.contains(&"v1"));
    assert!(g.edges.contains(&edge("start", "v1", "checks")));
}

#[test]
fn keeps_exploration_nodes_when_gate_claims_exist() {
    let bot = loaded(
        "retas",
        vec![
            Node::new("start", "retas", "start"),
            Node::new("traj:intent:headers", "map headers", "intent"),
            Node::new("traj:fact:csp-missing", "CSP missing", "fact"),
            Node::new("c1", "cors-wildcard", "claim"),
        ],
        vec![edge("start", "traj:intent:headers", "spawns"), edge("traj:intent:headers", "traj:fact:csp-missing", "yields")],
    );
    let g = merge_with_shelf(Some(&bot), &shelf_rows(), Some("trajectory/retas"));
    let has = |id: &str| g.nodes.iter().any(|n| n.id == id);
    assert!(has("traj:fact:csp-missing") && has("traj:intent:headers") && has("claims/retas/cors"));
    assert!(!has("c1"));
    assert!(g.edges.contains(&edge("traj:intent:headers", "traj:fact:csp-missing", "yields")));
}

#[test]
fn an_empty_bot_graph_falls_back_to_the_shelf() {
    let g = merge_with_shelf(None, &shelf_rows(), None);
    assert!(g.nodes.iter().any(|n| n.id == "claims/retas/cors"));
    assert_eq!(status_of(&g, "claims/retas/cors"), Some("verified"));
}

#[test]
fn parses_nested_trajectory_wrappers_and_rejects_empty_ones() {
    let inner = json!({ "goal_key": "g", "nodes": [{ "id": "start", "kind": "start", "label": "g" }], "edges": [] }).to_string();
    let g = parse_trajectory(&json!({ "content": inner }).to_string(), "g").expect("graph");
    assert_eq!((g.goal_key.as_str(), g.nodes.len()), ("g", 1));
    assert_eq!(parse_trajectory("{\"nodes\":[],\"edges\":[]}", "g"), None);
    assert_eq!(parse_trajectory("not json", "g"), None);
    let fallback = parse_trajectory(&json!({ "nodes": [{ "id": "a" }], "edges": [] }).to_string(), "from-key").expect("graph");
    assert_eq!(fallback.goal_key, "from-key");
    assert_eq!(fallback.nodes[0].kind, "fact", "a missing kind defaults to fact");
}

// ---- layout ----

fn place<'a>(l: &'a super::layout::Layout, id: &str) -> &'a super::layout::Cell {
    l.cells.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("missing {id}"))
}

#[test]
fn stacks_a_branch_in_the_next_column_and_keeps_the_chain_moving_right() {
    let children: Vec<Node> = (0..6).map(|i| node(&format!("card-{i}"), "fact")).collect();
    let mut nodes = vec![node("start", "start"), node("confirmed", "claim")];
    nodes.extend(children.clone());
    let mut edges = vec![edge("start", "confirmed", "derives")];
    edges.extend(children.iter().map(|c| edge("confirmed", &c.id, "spawns")));
    let l = layout(&nodes, &edges);
    assert_eq!(place(&l, "confirmed").rank - place(&l, "start").rank, 1);
    let ranks: std::collections::BTreeSet<usize> = children.iter().map(|c| place(&l, &c.id).rank).collect();
    let rows: std::collections::BTreeSet<usize> = children.iter().map(|c| place(&l, &c.id).row).collect();
    assert_eq!(ranks.into_iter().collect::<Vec<_>>(), [place(&l, "confirmed").rank + 1]);
    assert_eq!(rows.len(), 6, "each child gets its own row");
}

#[test]
fn follows_a_chain_left_to_right_for_as_many_links_as_it_has() {
    let nodes: Vec<Node> = (0..8).map(|i| node(&format!("n{i}"), "")).collect();
    let edges: Vec<Edge> = (1..8).map(|i| edge(&format!("n{}", i - 1), &format!("n{i}"), "")).collect();
    let l = layout(&nodes, &edges);
    assert_eq!(place(&l, "n7").rank - place(&l, "n0").rank, 7);
    let rows: std::collections::BTreeSet<usize> = l.cells.iter().map(|c| c.row).collect();
    assert_eq!(rows.len(), 1, "a chain stays on one row");
    assert_eq!((l.columns, l.rows), (8, 1));
}

#[test]
fn branches_one_node_into_two_lanes() {
    let l = layout(&[node("start", "start"), node("a", ""), node("b", "")], &[edge("start", "a", ""), edge("start", "b", "")]);
    let (a, b, s) = (place(&l, "a"), place(&l, "b"), place(&l, "start"));
    assert_eq!((a.rank, b.rank), (s.rank + 1, s.rank + 1));
    assert_ne!(a.row, b.row);
}

#[test]
fn merges_two_lanes_back_into_one_node_between_them() {
    let l = layout(
        &[node("start", "start"), node("left", ""), node("right", ""), node("join", "")],
        &[edge("start", "left", ""), edge("start", "right", ""), edge("left", "join", ""), edge("right", "join", "")],
    );
    let (left, right, join) = (place(&l, "left"), place(&l, "right"), place(&l, "join"));
    assert_eq!(left.rank, right.rank);
    assert_eq!(join.rank, left.rank + 1);
    // rows are whole lines, so "between" can land on a lane's own row (a different column)
    let (low, high) = (left.row.min(right.row), left.row.max(right.row));
    assert!(join.row >= low && join.row <= high, "join row {} outside {low}..={high}", join.row);
}

#[test]
fn keeps_every_node_in_a_cycle_and_still_gives_the_chain_depth() {
    let l = layout(&[node("a", ""), node("b", ""), node("c", "")], &[edge("a", "b", ""), edge("b", "c", ""), edge("c", "a", "")]);
    assert_eq!(l.cells.len(), 3);
    let ranks: std::collections::BTreeSet<usize> = l.cells.iter().map(|c| c.rank).collect();
    assert_eq!(ranks.len(), 3);
}

#[test]
fn starts_the_chain_at_the_start_card_when_it_is_listed_last() {
    let l = layout(
        &[node("card", "fact"), node("confirmed", "claim"), node("start", "start")],
        &[edge("start", "confirmed", ""), edge("confirmed", "card", "")],
    );
    assert!(place(&l, "start").rank < place(&l, "confirmed").rank);
    assert!(place(&l, "confirmed").rank < place(&l, "card").rank);
}

#[test]
fn pins_a_scope_root_at_the_first_column() {
    let l = layout(
        &[node("host", "host"), node("claim", "claim"), node("org:scope", "org")],
        &[edge("org:scope", "host", "contains"), edge("host", "claim", "has-claim")],
    );
    assert!(place(&l, "org:scope").rank < place(&l, "host").rank);
    assert!(place(&l, "host").rank < place(&l, "claim").rank);
}

#[test]
fn layout_ignores_edges_to_unknown_nodes_and_self_loops_and_handles_nothing() {
    let l = layout(&[node("a", ""), node("b", "")], &[edge("a", "ghost", ""), edge("a", "a", ""), edge("a", "b", "")]);
    assert_eq!(l.edges, [edge("a", "b", "")]);
    assert_eq!(layout(&[], &[]), super::layout::Layout::default());
    let island = layout(&[node("a", ""), node("b", "")], &[]);
    assert_eq!(island.cells.len(), 2, "unlinked nodes are still placed");
}

// ---- rendering ----

fn sample_view() -> super::view::TrajectoryView {
    let g = merge_with_shelf(None, &shelf_rows(), Some("trajectory/retas"));
    build(g, &shelf_rows())
}

#[test]
fn glyphs_carry_meaning_by_shape() {
    let g = |kind: &str, status: Option<&str>| {
        let mut n = super::view::ViewNode {
            id: "x".into(), label: "x".into(), kind: kind.into(), status: status.map(str::to_string), severity: None,
            note: None, resource_key: None, revision: None, rank: 0, row: 0,
        };
        n.status = status.map(str::to_string);
        glyph(&n)
    };
    assert_eq!(g("start", Some("in-scope")), '◆');
    assert_eq!(g("claim", Some("verified")), '●');
    assert_eq!(g("claim", Some("returned")), '↩');
    assert_eq!(g("verdict", Some("dismissed")), '✕');
    assert_eq!(g("verdict", Some("review-ok")), '◐');
    assert_eq!(g("goal", Some("Met")), '◌');
    assert_eq!(g("goal", Some("Budget exhausted")), '◌');
    assert_eq!(g("goal", Some("open")), '○');
    assert_eq!(g("fact", None), '○');
    assert_eq!(g("claim", Some("Met")), '○', "only goals can be exhausted");
}

#[test]
fn the_outline_shows_the_counts_the_tree_and_the_gate_glyphs() {
    let text = outline(&sample_view());
    assert!(text.starts_with("goal: retas   ● 1 verified · ○ 0 open · ↩ 1 returned"), "cors is verified and headers returned, so nothing is open: {text}");
    assert!(text.contains("◆ start: retas"), "{text}");
    assert!(text.contains("● claim: cors-wildcard [high]  rev 1"), "{text}");
    assert!(text.contains("↩ claim: missing-headers [medium]"), "{text}");
    assert!(text.contains("● finding: Permissive CORS"), "{text}");
    assert!(text.contains("├─ ") && text.contains("└─ "), "{text}");
}

#[test]
fn the_outline_shows_a_shared_node_once_and_lists_what_the_start_cannot_reach() {
    let nodes = vec![node("start", "start"), node("a", "fact"), node("b", "fact"), node("shared", "fact"), node("lost", "fact")];
    let g = Graph { goal_key: "g".into(), nodes, edges: vec![edge("start", "a", ""), edge("start", "b", ""), edge("a", "shared", ""), edge("b", "shared", "")] };
    let text = outline(&build(g, &[]));
    assert_eq!(text.matches("fact: shared").count(), 1, "{text}");
    assert!(text.contains("↪ shared (shown above)"), "{text}");
    assert!(text.contains("unlinked:") && text.contains("fact: lost"), "{text}");
}

#[test]
fn an_empty_view_says_so() {
    let view = build(Graph { goal_key: "g".into(), nodes: vec![], edges: vec![] }, &[]);
    assert!(outline(&view).contains("nothing on the shelf yet"));
    assert_eq!(layered(&view, 200), None);
}

#[test]
fn the_layered_view_draws_straight_links_and_lists_the_rest() {
    let g = Graph {
        goal_key: "g".into(),
        nodes: vec![node("start", "start"), node("a", "fact"), node("b", "fact")],
        edges: vec![edge("start", "a", "spawns"), edge("start", "b", "spawns")],
    };
    let text = layered(&build(g, &[]), 200).expect("fits");
    assert!(text.contains("◆ start ─▶ ○ a") || text.contains("◆ start ─▶ ○ b"), "{text}");
    assert!(text.contains("links:") && text.contains("─spawns▶"), "the link that is not straight is listed: {text}");
}

#[test]
fn the_layered_view_gives_way_to_the_outline_when_too_wide() {
    let view = sample_view();
    assert!(layered(&view, 10).is_none());
    let narrow = render_all(&[view.clone()], Format::Ascii, Some(10)).expect("render");
    assert_eq!(narrow, outline(&view), "falls back to the outline");
    let default = render_all(&[view], Format::Ascii, None).expect("render");
    assert!(default.contains("├─ ") || default.contains("└─ "));
}

#[test]
fn dot_mermaid_and_json_carry_the_same_graph() {
    let view = sample_view();
    let d = dot(&view);
    assert!(d.starts_with("digraph \"retas\"") && d.contains("\"claims/retas/cors\" -> \"findings/retas/cors\" [label=\"proves\"]"), "{d}");
    let m = mermaid(&view);
    assert!(m.starts_with("flowchart LR") && m.contains("-->|proves|"), "{m}");
    let j: Value = serde_json::from_str(&render_json(&[view.clone()]).expect("json")).expect("parse");
    assert_eq!(j[0]["goal_key"], "retas");
    assert_eq!(j[0]["counts"]["verified"], 1);
    assert_eq!(j[0]["nodes"].as_array().expect("nodes").len(), view.nodes.len());
}

#[test]
fn quotes_in_labels_cannot_break_dot_or_mermaid() {
    let g = Graph { goal_key: "a\"b".into(), nodes: vec![Node::new("x", "say \"hi\"\nnow", "fact")], edges: vec![] };
    let view = build(g, &[]);
    assert!(dot(&view).contains("say \\\"hi\\\" now"));
    assert!(!mermaid(&view).contains("say \"hi\""));
}

#[test]
fn formats_parse_by_name() {
    assert_eq!(Format::parse("json"), Some(Format::Json));
    assert_eq!(Format::parse("text"), Some(Format::Ascii));
    assert_eq!(Format::parse("svg"), None);
}

// ---- projection from a real shelf ----

fn shelf_with(rows: &[(&str, Value)]) -> Shelf {
    let s = Shelf::open_in_memory().expect("shelf");
    for (key, args) in rows {
        let tool = match key.split('/').next().unwrap_or("") {
            "trajectory" => "put_trajectory",
            "scope" => "put_scope",
            "claims" => "put_claim",
            _ => "put",
        };
        let mut a = args.clone();
        a["key"] = json!(key);
        s.call(tool, &a).unwrap_or_else(|e| panic!("{key}: {}", e.0));
    }
    s
}

#[test]
fn an_empty_shelf_projects_nothing() {
    assert!(project(&Shelf::open_in_memory().expect("shelf"), None).expect("project").is_empty());
}

#[test]
fn a_shelf_without_a_published_trajectory_is_drawn_from_its_rows() {
    let s = shelf_with(&[
        ("scope/app", json!({ "host": "app.example", "expected_revision": 0 })),
        ("claims/a", json!({ "vulnclass": "cors", "name": "CORS", "severity": "high", "summary": "s", "host": "app.example", "poc": "curl", "expected_revision": 0 })),
    ]);
    let views = project(&s, None).expect("project");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].goal_key, "app");
    assert_eq!(views[0].counts, c(1, 0, 0, 0));
    let claim = views[0].nodes.iter().find(|n| n.id == "claims/a").expect("claim node");
    assert_eq!((claim.revision, claim.status.as_deref()), (Some(1), Some("open")));
}

#[test]
fn each_published_trajectory_is_a_view_and_a_goal_can_be_picked() {
    let traj = |goal: &str| json!({ "goal_key": goal, "nodes": [
        { "id": "start", "kind": "start", "label": goal }, { "id": "g1", "kind": "goal", "label": "Map login", "status": "open" } ],
        "edges": [{ "from": "start", "to": "g1", "rel": "spawns" }] });
    let s = shelf_with(&[("trajectory/web", traj("web")), ("trajectory/api", traj("api"))]);
    let views = project(&s, None).expect("project");
    let goals: Vec<&str> = views.iter().map(|v| v.goal_key.as_str()).collect();
    assert_eq!(goals, ["api", "web"], "ordered by key");
    for v in &views {
        assert!(v.nodes.iter().any(|n| n.id == "g1" && n.kind == "goal"), "the published exploration node is kept");
    }
    assert_eq!(project(&s, Some("web")).expect("project").len(), 1);
    assert_eq!(project(&s, Some("trajectory/api")).expect("project").len(), 1);
    assert!(project(&s, Some("nope")).expect("project").is_empty());
}
