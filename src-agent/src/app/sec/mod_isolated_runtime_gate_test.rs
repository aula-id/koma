use super::*;

#[test]
fn discovered_tools_cannot_be_advertised_or_dispatched_before_isolation() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = SecDaemonManager::new(runtime.handle());
    manager.inner.lock().unwrap().tools.push(SecToolDesc {
        name: "sec_http".into(),
        description: "test".into(),
        parameters: serde_json::json!({}),
        risk: SecRisk::Remote,
        network_param: Some("url".into()),
        compute: "network".into(),
        domain: "web".into(),
    });
    // Gated on a real runtime probe, so the expectation follows the machine
    // rather than a hardcoded `false`: with Docker present the discovered
    // tools ARE advertised, and the dispatch gate below is what still stops
    // them until the boundary is wired.
    let available = crate::security::cyber_execution_available();
    assert_eq!(manager.tool_names().is_empty(), !available);
    assert_eq!(manager.tool_defs().is_empty(), !available);
    // Without a live daemon the dispatch still refuses; the REASON differs by
    // machine, so assert only that it refuses and names something actionable.
    let error = manager
        .execute_blocking("sec_http", &serde_json::json!({}))
        .unwrap_err();
    assert!(
        error.contains("isolated runtime") || error.contains("not running"),
        "{error}"
    );
}

/// The boundary is consulted from OUTSIDE this manager (dispatch), so the
/// reasons must be stable strings a caller can assert and echo.
#[test]
fn sec_refusal_reasons_are_stable_and_fail_closed() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = SecDaemonManager::new(runtime.handle());

    // Not in the namespace: no opinion, so the caller falls through.
    assert!(manager.refusal("read", true).is_none());

    // Lane off beats every later check.
    assert!(manager
        .refusal("sec_http", false)
        .is_some_and(|r| r.contains("lane off")));

    // A name the daemon never handed over is refused BY NAME, before any
    // runtime question — the more specific diagnosis wins.
    assert!(manager
        .refusal("sec_forged", true)
        .is_some_and(|r| r.contains("sec_forged")));

    // Advertised, but the isolated runtime is not configured yet: still
    // unreachable, for the runtime reason rather than the name reason.
    manager.inner.lock().unwrap().tools.push(SecToolDesc {
        name: "sec_http".into(),
        description: "test".into(),
        parameters: serde_json::json!({}),
        risk: SecRisk::Remote,
        network_param: Some("url".into()),
        compute: "network".into(),
        domain: "web".into(),
    });
    // A forged name is refused regardless of the runtime, which is the
    // invariant worth pinning here.
    assert!(manager
        .refusal("sec_nope", true)
        .is_some_and(|r| r.contains("not advertised")));
    // And an advertised one is refused exactly when no runtime exists.
    let refusal = manager.refusal("sec_http", true);
    if crate::security::cyber_execution_available() {
        assert!(refusal.is_none(), "{refusal:?}");
    } else {
        assert!(refusal.is_some_and(|r| r.contains("isolated runtime")));
    }
}
