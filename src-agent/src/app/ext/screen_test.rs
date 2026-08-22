#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

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

/// The screen auto-start decision mirrors the GUI panel bridge's over every input:
/// running → invoke; not-running daemon+enabled → start; oneshot / disabled / missing →
/// "extension not available".
#[test]
fn screen_start_decision_covers_all_cases() {
    assert_eq!(screen_start_decision(true, None), Ok(true));
    assert_eq!(
        screen_start_decision(true, Some(&ext_record("daemon", false))),
        Ok(true)
    );
    assert_eq!(
        screen_start_decision(false, Some(&ext_record("daemon", true))),
        Ok(false)
    );
    assert_eq!(
        screen_start_decision(false, Some(&ext_record("oneshot", true))),
        Err("extension not available".to_string())
    );
    assert_eq!(
        screen_start_decision(false, Some(&ext_record("daemon", false))),
        Err("extension not available".to_string())
    );
    assert_eq!(
        screen_start_decision(false, None),
        Err("extension not available".to_string())
    );
}
