#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Unit coverage for the W8 panel-push routing core (`route_ext_notify` +
//! `parse_panel_push` + `enforce_ext_panel_cap`). These ARE the whole per-notify + cap logic
//! of [`drain_ext_notifies`]; its thin take/put-back drain shell is the identical pattern to
//! [`drain_ext_calls`] (exercised end-to-end by `app::ext`'s integration test that drives a
//! real extension's `panel_push` onto `ext_notify_tx`), so it is not re-tested against a full
//! `AppState` here.
use super::*;

fn notify(name: &str, params: serde_json::Value) -> crate::app::ext::ExtNotify {
    crate::app::ext::ExtNotify {
        ext_id: "run.koma.test".to_string(),
        name: name.to_string(),
        params,
    }
}

#[test]
fn parse_panel_push_reads_or_rejects() {
    // Well-formed → Some.
    assert_eq!(
        parse_panel_push(&serde_json::json!({ "panelId": "p1", "payload": { "x": 1 } })),
        Some(("p1".to_string(), serde_json::json!({ "x": 1 })))
    );
    // Missing payload → None.
    assert_eq!(
        parse_panel_push(&serde_json::json!({ "panelId": "p1" })),
        None
    );
    // Missing panelId → None.
    assert_eq!(parse_panel_push(&serde_json::json!({ "payload": 1 })), None);
    // Non-string panelId → None.
    assert_eq!(
        parse_panel_push(&serde_json::json!({ "panelId": 7, "payload": 1 })),
        None
    );
}

#[test]
fn route_ext_notify_appends_valid_panel_push_only() {
    let mut out = Vec::new();
    route_ext_notify(
        &mut out,
        notify(
            "panel.push",
            serde_json::json!({ "panelId": "p1", "payload": { "ok": true } }),
        ),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "run.koma.test");
    assert_eq!(out[0].1, "p1");
    assert_eq!(out[0].2, serde_json::json!({ "ok": true }));

    // Malformed panel.push → dropped, no outbox growth.
    route_ext_notify(
        &mut out,
        notify("panel.push", serde_json::json!({ "nope": 1 })),
    );
    assert_eq!(out.len(), 1);

    // Unknown notify name → dropped, no outbox growth.
    route_ext_notify(
        &mut out,
        notify(
            "tool.call",
            serde_json::json!({ "panelId": "p1", "payload": 1 }),
        ),
    );
    assert_eq!(out.len(), 1);
}

#[test]
fn enforce_cap_drops_oldest() {
    let mut out: Vec<(String, String, serde_json::Value)> = Vec::new();
    for i in 0..260 {
        route_ext_notify(
            &mut out,
            notify(
                "panel.push",
                serde_json::json!({ "panelId": format!("p{i}"), "payload": i }),
            ),
        );
    }
    assert_eq!(out.len(), 260);
    enforce_ext_panel_cap(&mut out);
    assert_eq!(out.len(), EXT_PANEL_PUSH_CAP);
    // The first 4 pushed (p0..=p3) are the shed-oldest; p4 becomes the new head, p259 the tail.
    assert_eq!(out[0].1, "p4");
    assert_eq!(out[EXT_PANEL_PUSH_CAP - 1].1, "p259");
}
