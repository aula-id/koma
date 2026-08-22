#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// A missing/never-installed manifest degrades to an empty panel list rather than
/// failing — the id here is guaranteed to have no `extensions/<id>/manifest.json` on
/// any test machine.
#[test]
fn read_ext_panels_degrades_to_empty_on_missing_manifest() {
    assert_eq!(
        read_ext_panels("run.koma.definitely-not-installed.test-fixture"),
        Vec::<PanelWire>::new()
    );
}

/// A registry fixture for the panel-start decision (only `enabled` + `kind` matter to it).
fn ext_record(kind: &str, enabled: bool) -> InstalledExtension {
    InstalledExtension {
        id: "run.koma.test".to_string(),
        version: "0.0.1".to_string(),
        tier: "free".to_string(),
        granted: Vec::new(),
        enabled,
        kind: kind.to_string(),
        exec: "bin/x".to_string(),
    }
}

/// The panel-start decision (W8 auto-start) over every input combination: an already-running
/// extension is invoked straight away; a not-running daemon+enabled one is started; a
/// oneshot / disabled / missing one is a clean "extension not available" error.
#[test]
fn panel_start_decision_covers_all_cases() {
    // Already running → invoke straight away, regardless of the record (or its absence).
    assert_eq!(panel_start_decision(true, None), Ok(true));
    assert_eq!(
        panel_start_decision(true, Some(&ext_record("daemon", true))),
        Ok(true)
    );
    // Disabled but somehow running → still Ok(true): the enabled flag only gates auto-start.
    assert_eq!(
        panel_start_decision(true, Some(&ext_record("daemon", false))),
        Ok(true)
    );

    // Not running, daemon + enabled → auto-start (Ok(false)).
    assert_eq!(
        panel_start_decision(false, Some(&ext_record("daemon", true))),
        Ok(false)
    );
    // Not running, oneshot-kind → error (no persistent panel backend).
    assert_eq!(
        panel_start_decision(false, Some(&ext_record("oneshot", true))),
        Err("extension not available".to_string())
    );
    // Not running, disabled daemon → error (auto-start intentionally off).
    assert_eq!(
        panel_start_decision(false, Some(&ext_record("daemon", false))),
        Err("extension not available".to_string())
    );
    // Not running, not installed → error.
    assert_eq!(
        panel_start_decision(false, None),
        Err("extension not available".to_string())
    );
}
