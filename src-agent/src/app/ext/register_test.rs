#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// A missing manifest.json (nothing installed at that path) is a clean `Err`,
/// not a panic — `register_contributions` propagates it so the caller logs
/// and moves on (mirrors the boot loop's existing `ensure_started` failure
/// handling in `build_startup`).
#[test]
fn register_contributions_errors_on_missing_manifest() {
    let ext = InstalledExtension {
        id: format!("run.koma.example.does-not-exist-{}", uuid::Uuid::new_v4()),
        version: "0.0.1".to_string(),
        tier: "free".to_string(),
        granted: vec![],
        enabled: true,
        kind: "daemon".to_string(),
        exec: "bin/tool".to_string(),
    };
    let rt = tokio::runtime::Runtime::new().expect("build runtime");
    let ext_manager = ExtHostManager::new(rt.handle());
    let result = register_contributions(&ext, None, &ext_manager);
    assert!(
        result.is_err(),
        "a missing manifest.json must be a clean Err"
    );
}
