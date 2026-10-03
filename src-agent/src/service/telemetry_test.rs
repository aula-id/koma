#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{advance_acked_at, gap_after_success, WireEvent, BACKLOG_GAP, BATCH, MIN_GAP};
use crate::model::usage::TelemetryEvent;

#[test]
fn watermark_only_moves_forward() {
    let dir = std::env::temp_dir().join(format!("koma-tel-wm-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("telemetry.json");
    let _ = std::fs::remove_file(&path);

    advance_acked_at(&path, 10);
    advance_acked_at(&path, 40);
    advance_acked_at(&path, 25);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("40"), "rewind must not land: {text}");
    assert!(!text.contains("25"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn wire_skips_empty_model_and_keeps_koma_ids() {
    assert!(WireEvent::from_ledger(TelemetryEvent {
        id: 1,
        ts: 10,
        model_id: String::new(),
        tokens_in: 1,
        tokens_out: 1,
    })
    .is_none());
    let ok = WireEvent::from_ledger(TelemetryEvent {
        id: 2,
        ts: 10,
        model_id: "koma/apple".into(),
        tokens_in: 8,
        tokens_out: 3,
    })
    .unwrap();
    assert_eq!(ok.model, "koma/apple");
    assert_eq!(ok.tokens_in, 8);
}

#[test]
fn first_run_gap_slows_down_when_the_backlog_is_huge() {
    assert_eq!(gap_after_success(0), MIN_GAP);
    assert_eq!(gap_after_success(500), MIN_GAP);
    assert_eq!(gap_after_success(800_000), BACKLOG_GAP);
    assert_eq!(BATCH, 50);
}
