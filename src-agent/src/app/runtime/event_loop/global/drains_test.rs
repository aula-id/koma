#![allow(clippy::unwrap_used, clippy::expect_used)]
//! W13 additional regression suite for `drains.rs` — PURE ADDITION alongside the existing
//! inline `mod ext_notify_tests` in that file (never touched here).
//!
//! Explicitly SKIPPED as already fully covered inline (see `drains.rs::ext_notify_tests`):
//! - `parse_panel_push` missing `panelId` / non-string `panelId` / missing `payload`
//!   (`parse_panel_push_reads_or_rejects` already probes exactly these three);
//! - `route_ext_notify` unknown-name drop (`route_ext_notify_appends_valid_panel_push_only`
//!   already probes an unrecognised `"tool.call"` notify name).
//!
//! Gaps targeted here: a well-formed `panel.push` with EXTRA unrelated keys still parses
//! (forward-compat), and the outbox cap holds correctly across MULTIPLE interleaved
//! push/enforce cycles (the existing inline `enforce_cap_drops_oldest` test only proves a
//! single enforce call after one big push burst).

use super::*;

fn notify(name: &str, params: serde_json::Value) -> crate::app::ext::ExtNotify {
    crate::app::ext::ExtNotify {
        ext_id: "run.koma.test".to_string(),
        name: name.to_string(),
        params,
    }
}

/// A well-formed `panel.push` carrying EXTRA, unrecognised keys alongside `panelId`/`payload`
/// still parses — the SDK's wire shape is forward-compat, so a future extra field never breaks
/// an older host's `parse_panel_push`.
#[test]
fn parse_panel_push_ignores_extra_fields() {
    let parsed = parse_panel_push(&serde_json::json!({
        "panelId": "p1",
        "payload": { "x": 1 },
        "futureField": "ignored",
        "anotherOne": [1, 2, 3],
    }));
    assert_eq!(
        parsed,
        Some(("p1".to_string(), serde_json::json!({ "x": 1 })))
    );
}

/// The outbox cap holds correctly across MULTIPLE interleaved push/enforce cycles — mirroring
/// how `drain_ext_notifies` actually calls `enforce_ext_panel_cap` ONCE per tick, but several
/// ticks in a row: push a small batch, enforce, push another, enforce again. The sliding window
/// must always keep exactly the most-recent [`EXT_PANEL_PUSH_CAP`] entries, never re-admitting
/// something already shed nor double-counting a not-yet-shed entry.
#[test]
fn enforce_cap_holds_across_interleaved_push_cycles() {
    let mut out: Vec<(String, String, serde_json::Value)> = Vec::new();

    // Tick 1: push 200 (under cap) — enforce is a no-op.
    for i in 0..200 {
        route_ext_notify(
            &mut out,
            notify(
                "panel.push",
                serde_json::json!({ "panelId": format!("p{i}"), "payload": i }),
            ),
        );
    }
    enforce_ext_panel_cap(&mut out);
    assert_eq!(out.len(), 200, "under cap: enforce must not shed anything");
    assert_eq!(
        out[0].1, "p0",
        "no shedding yet — the oldest entry is still p0"
    );

    // Tick 2: push another 100 (total 300, 44 over cap) — enforce sheds the 44 oldest.
    for i in 200..300 {
        route_ext_notify(
            &mut out,
            notify(
                "panel.push",
                serde_json::json!({ "panelId": format!("p{i}"), "payload": i }),
            ),
        );
    }
    assert_eq!(out.len(), 300);
    enforce_ext_panel_cap(&mut out);
    assert_eq!(out.len(), EXT_PANEL_PUSH_CAP);
    // 300 - 256 = 44 shed: p0..=p43 gone, p44 is the new head, p299 the tail.
    assert_eq!(
        out[0].1, "p44",
        "the second enforce must shed exactly the newly-oldest entries"
    );
    assert_eq!(out[EXT_PANEL_PUSH_CAP - 1].1, "p299");

    // Tick 3: push 10 more (now over cap by 10) — enforce sheds exactly those 10 oldest
    // (p44..=p53), never re-admitting anything from tick 1 that was already shed.
    for i in 300..310 {
        route_ext_notify(
            &mut out,
            notify(
                "panel.push",
                serde_json::json!({ "panelId": format!("p{i}"), "payload": i }),
            ),
        );
    }
    enforce_ext_panel_cap(&mut out);
    assert_eq!(out.len(), EXT_PANEL_PUSH_CAP);
    assert_eq!(
        out[0].1, "p54",
        "tick 3 must shed only the entries that aged out since tick 2"
    );
    assert_eq!(out[EXT_PANEL_PUSH_CAP - 1].1, "p309");
    // Nothing from the first-ever shed window (p0..=p43) can have resurfaced.
    assert!(out
        .iter()
        .all(|(_, id, _)| id.as_str() != "p0" && id.as_str() != "p43"));
}
